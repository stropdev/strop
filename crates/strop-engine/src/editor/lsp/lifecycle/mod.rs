//! Attachment admission, publication, trust and server lifetime ownership.
mod attach_outcomes;
mod spawn;
mod warm;

#[cfg(test)]
pub(crate) use warm::warm_language;

use super::attach::AttachKey;
use super::*;
use strop_core::id::DocumentId;

impl Editor {
    /// Try to attach a language server for the current buffer. The
    /// synchronous part only touches in-memory state; discovery
    /// (config, root, trust, executability) is owned worker work behind the
    /// replay gate.
    /// The LSP half of the startup "start services" action: enable
    /// attach, then attach for the current buffer. A pure state
    /// transition performed identically live and replayed — the tape
    /// gates native discovery inside `lsp_attach_document`.
    pub fn lsp_start_services(&mut self) {
        self.lsp_state.attach.enabled = true;
        self.lsp_attach_document(self.current());
    }

    pub(crate) fn lsp_attach_document(&mut self, document: DocumentId) {
        if !self.lsp_state.attach.enabled {
            return;
        }
        let Some(doc) = self.lsp_document_location(document) else {
            return;
        };
        // Extensionless/ambiguous headers keep a navigation-bound
        // context (0049 §4.4); without one there is nothing to attach.
        let Some(language) = self.lsp_doc_language(document, &doc.path) else {
            return;
        };
        if self
            .lsp_server_for(document, &doc.path, &language, &doc.filesystem)
            .is_some()
        {
            self.lsp_did_open_document(document);
            return;
        }
        // Discovery runs only for an unambiguous extension — never
        // rediscover project commands from /usr/include (0049 §4.7).
        let Some(ext) = doc
            .path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
        else {
            return;
        };
        let Some(language) = registry::language_for_extension(&ext) else {
            return;
        };
        let key = AttachKey {
            target: doc.filesystem.clone(),
            language: language.to_string(),
            path: doc.path.clone(),
        };
        match self.lsp_state.attach.refused.get(&key) {
            // Trust decisions change (`:trust`); re-discover those.
            Some(attach::AttachDecision::TrustRequired { .. })
            | Some(attach::AttachDecision::TrustError { .. })
            // Remote infrastructure failures are transient: the
            // connection may be back; re-discover.
            | Some(attach::AttachDecision::RemoteIo { .. }) => {}
            // Everything else was reported once and stays refused.
            Some(_) => return,
            None => {}
        }
        if self.lsp_state.attach.pending.contains_key(&key) {
            return;
        }
        let ticket = match self.worker_ids.allocate() {
            Ok(ticket) => ticket,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        self.lsp_state.attach.refused.remove(&key);
        self.lsp_state.attach.pending.insert(key, ticket);
        let args = attach::AttachArgs {
            ticket,
            path: doc.path.clone(),
            language: language.to_string(),
            target: doc.filesystem.clone(),
        };
        // Replay gate: no native config/trust/executability before this
        // registration (R11).
        match self.tape.request("lsp.attach", &args) {
            Ok(true) => self.lsp_spawn_discovery(ticket, doc, ext, language),
            Ok(false) => {}
            Err(error) => {
                self.lsp_state.attach.pending.remove(&args_key(&args));
                self.message = format!("lsp attach diverged from trace: {error}");
            }
        }
    }

    /// The server placement for a buffer, if one is attached: exact
    /// language match on the same filesystem target, longest covering
    /// root wins.
    /// Resolve the serving (server, root) for a document: the
    /// navigation-bound context first (0049 §4.2 — the server that
    /// brought this document here keeps answering inside it), then
    /// longest-covering-root discovery for unbound ordinary opens.
    /// Never a scan that picks the first server speaking the language.
    pub(crate) fn lsp_server_for(
        &self,
        document: DocumentId,
        path: &Path,
        language: &str,
        target: &Filesystem,
    ) -> Option<(ServerId, PathBuf)> {
        // The maps are the authority: `lsp_failed` drops a dead
        // server's bindings and carried contexts, so a present entry is
        // a live context. Replay resolves identically without a client.
        if let Some(binding) = self.lsp_state.bindings.get(&document) {
            if binding.target == *target && binding.language == language {
                return Some((binding.server, binding.root.clone()));
            }
        }
        if let Some(context) = self.lsp_state.jump_contexts.get(&document) {
            if context.target == *target && context.language == language {
                return Some((context.server, context.root.clone()));
            }
        }
        let attach = &self.lsp_state.attach;
        let covering = attach
            .attached
            .iter()
            .filter(|a| a.language == language && &a.target == target && path.starts_with(&a.root))
            .max_by_key(|a| a.root.as_os_str().len());
        if let Some(best) = covering {
            return Some((best.server, best.root.clone()));
        }
        self.lsp_project_server(path, language, target)
    }

    /// A local document outside the pinned project — a library source, a
    /// loose file — belongs to the project's server for its language
    /// (plan 0091 §2.3; rust-analyzer, clangd and gopls serve library
    /// files from the workspace that depends on them). Only when that
    /// owner is unambiguous: with two project servers for the language
    /// there is no single context, and 0049 §4.2 forbids picking one.
    fn lsp_project_server(
        &self,
        path: &Path,
        language: &str,
        target: &Filesystem,
    ) -> Option<(ServerId, PathBuf)> {
        if *target != Filesystem::Local || path.starts_with(&self.cwd) {
            return None;
        }
        let mut owners = self.lsp_state.attach.attached.iter().filter(|a| {
            a.target == *target
                && same_language_family(&a.language, language)
                && (a.root.starts_with(&self.cwd) || self.cwd.starts_with(&a.root))
        });
        let owner = owners.next()?;
        owners
            .next()
            .is_none()
            .then(|| (owner.server, owner.root.clone()))
    }

    /// The first recorded layer diagnostic, when any — readiness and
    /// later messages must not erase it (0033 §2).
    pub(super) fn layer_warning(&self) -> Option<String> {
        layer_suffix(&self.lsp_state.attach.layer_diagnostics)
    }

    /// Retire non-local servers whose owning workspace no longer has a
    /// document: no orphan SSH process or live placement.
    /// Local servers keep their session-long lifetime.
    pub(crate) fn lsp_retire_workspace_servers(&mut self) {
        let retired: Vec<ServerId> = self
            .lsp_state
            .attach
            .attached
            .iter()
            .filter(|a| match &a.target {
                Filesystem::Local => false,
                Filesystem::Remote(endpoint) => !self.docs.iter().any(|(id, document)| {
                    document.remote_metadata().is_some_and(|source| {
                        source.file.endpoint() == endpoint
                            && source.file.path().starts_with(&a.root)
                            && self.lsp_doc_language(id, source.file.path()).as_deref()
                                == Some(a.language.as_str())
                    })
                }),
            })
            .map(|a| a.server)
            .collect();
        for server in retired {
            self.lsp_retire_server(server, "owning workspace closed");
        }
    }

    /// Remove one server's placements, bindings and connection with a
    /// graceful shutdown that never blocks the input thread.
    fn lsp_retire_server(&mut self, server: ServerId, reason: &str) {
        // Remove the placements first: the closes below re-enter the
        // retirement scan, and this server must already be gone from
        // the tables so the recursion is empty.
        self.lsp_state
            .attach
            .attached
            .retain(|a| a.server != server);
        let connection = self
            .lsp_servers
            .iter()
            .position(|s| s.id == server)
            .map(|index| self.lsp_servers.remove(index));
        let mut documents: Vec<_> = self
            .lsp_state
            .bindings
            .iter()
            .filter(|(_, binding)| binding.server == server)
            .map(|(document, _)| *document)
            .collect();
        documents.sort();
        for document in documents {
            self.lsp_close_document(document);
        }
        if let Some(connection) = connection {
            if let Some(client) = connection.client {
                std::thread::spawn(move || {
                    client.shutdown();
                    client.wait(std::time::Duration::from_secs(2));
                });
            }
        }
        trace::services::rejected("lsp", reason);
    }
}

/// One server speaks both C and C++ (0049 §4.4); every other language
/// is its own family.
fn same_language_family(served: &str, language: &str) -> bool {
    served == language || (matches!(served, "c" | "cpp") && matches!(language, "c" | "cpp"))
}

fn args_key(args: &attach::AttachArgs) -> AttachKey {
    AttachKey {
        target: args.target.clone(),
        language: args.language.clone(),
        path: args.path.clone(),
    }
}

/// Modeline suffix for malformed layers: the first diagnostic's exact
/// path, plus a count when more follow (0033 §2).
fn layer_suffix(layers: &[strop_lsp::languages::LayerDiagnostic]) -> Option<String> {
    let first = layers.first()?;
    Some(if layers.len() == 1 {
        first.display()
    } else {
        format!("{} (+{} more)", first.display(), layers.len() - 1)
    })
}
