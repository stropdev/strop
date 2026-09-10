//! Attachment admission, publication, trust and server lifetime ownership.
use super::attach::{AttachKey, AttachRecord};
use super::*;
use std::sync::mpsc::channel;
use strop_core::id::DocumentId;

impl Editor {
    /// Try to attach a language server for the current buffer. The
    /// synchronous part only touches in-memory state; discovery
    /// (config, root, trust, executability) is owned worker work behind the
    /// replay gate.
    /// The LSP half of the startup "start services" action: enable
    /// attach, then attach for the current buffer. A pure state
    /// transition performed identically live and replayed — the tape
    /// gates the native discovery inside `lsp_maybe_attach`.
    pub fn lsp_start_services(&mut self) {
        self.lsp_state.attach.enabled = true;
        self.lsp_maybe_attach();
    }

    pub(crate) fn lsp_maybe_attach(&mut self) {
        if !self.lsp_state.attach.enabled {
            return;
        }
        if self.cur().remote_metadata().is_some() && !self.remote_window_complete() {
            // Typed refusal, never silence: a partial/follow window is
            // not a document a server can be told about.
            self.message = "lsp unavailable — partial remote window".into();
            return;
        }
        let Some(doc) = self.lsp_current_doc_path() else {
            return;
        };
        // Extensionless/ambiguous headers keep a navigation-bound
        // context (0049 §4.4); without one there is nothing to attach.
        let Some(language) = self.lsp_doc_language(self.current(), &doc.path) else {
            return;
        };
        if self
            .lsp_server_for(self.current(), &doc.path, &language, &doc.filesystem)
            .is_some()
        {
            self.lsp_did_open_current();
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

    fn lsp_spawn_discovery(
        &mut self,
        ticket: strop_core::worker::WorkerId,
        doc: ResourceLocation,
        ext: String,
        language: &'static str,
    ) {
        let place = match doc.filesystem.clone() {
            Filesystem::Local => attach::DiscoverPlace::Local {
                abs: doc.path.clone(),
                cwd: self.cwd.clone(),
                git_workdir: self.git.as_ref().map(|g| g.workdir().to_path_buf()),
            },
            // The remote client is a cheap clone routed to the owned
            // session actor; the document's lease keeps it connected.
            Filesystem::Remote(_) => {
                let Some(file) = self.remote_file().cloned() else {
                    return;
                };
                attach::DiscoverPlace::Remote {
                    file,
                    client: self.remote_client(),
                }
            }
            // The container workspace roots at the document's directory;
            // the id is the canonical inspect identity.
            Filesystem::Container(id) => attach::DiscoverPlace::Container {
                root: doc.path.parent().unwrap_or(Path::new("/")).to_path_buf(),
                id,
            },
        };
        let input = attach::DiscoverInput {
            ticket,
            place,
            ext,
            language,
            state_dir: self.state_dir.clone(),
            xdg: strop_lsp::languages::xdg_path(),
            transport: self.lsp_state.attach.transport.clone(),
        };
        let cancelled = attach::AttachRecord {
            ticket,
            server: None,
            language: language.to_owned(),
            name: language.to_owned(),
            root: doc.path.parent().unwrap_or(Path::new("/")).to_owned(),
            target: doc.filesystem,
            outcome: attach::AttachDecision::Cancelled,
            layers: Vec::new(),
        };
        let done = self.lsp_state.attach.attach_channel();
        // Discovery is an owned worker: the remote reads and probes it
        // performs are cancellable (superseded attach attempts are
        // cancelled when a newer ticket takes the key).
        let handle = strop_core::worker::spawn(
            "strop-lsp-attach",
            move |outcome| {
                let record = match outcome {
                    strop_core::worker::Outcome::Success(record) => record,
                    strop_core::worker::Outcome::Cancelled(_) => cancelled,
                    strop_core::worker::Outcome::Failed { failure, .. } => attach::AttachRecord {
                        outcome: attach::AttachDecision::SpawnFailed {
                            reason: failure.message,
                        },
                        ..cancelled
                    },
                };
                let _ = done.send(record);
            },
            move |token| match attach::discover(input, &token) {
                Some(record) => strop_core::worker::Outcome::Success(record),
                None => strop_core::worker::Outcome::Cancelled(
                    strop_core::worker::CancelReason::OwnerClosed,
                ),
            },
        );
        self.worker_handles.insert(ticket, handle);
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
        let best = attach
            .attached
            .iter()
            .filter(|a| a.language == language && &a.target == target && path.starts_with(&a.root))
            .max_by_key(|a| a.root.as_os_str().len())?;
        Some((best.server, best.root.clone()))
    }

    pub(crate) fn handle_lsp_attach(&mut self, record: AttachRecord) {
        trace_attach(&record);
        self.worker_handles.remove(&record.ticket);
        let key = self
            .lsp_state
            .attach
            .pending
            .iter()
            .find_map(|(key, &owner)| (owner == record.ticket).then(|| key.clone()));
        let Some(key) = key else {
            trace::services::rejected("lsp", "attach completion superseded");
            self.retire_superseded_transport(record.server);
            return;
        };
        self.lsp_state.attach.pending.remove(&key);
        if key.target != record.target || key.language != record.language {
            self.retire_superseded_transport(record.server);
            trace::services::rejected("lsp", "attach result differs from its requested workspace");
            return;
        }
        let attach::AttachRecord {
            ticket: _,
            server,
            language,
            name,
            root,
            target,
            outcome,
            layers,
        } = record;
        // Malformed layers are diagnosed whatever the outcome (0033
        // §2): a healthy fallback server must not erase them.
        self.record_layer_diagnostics(&layers);
        let warning = layer_suffix(&layers);
        match outcome {
            attach::AttachDecision::Cancelled => {}
            attach::AttachDecision::Attached => {
                let Some(server) = server else { return };
                if self.lsp_state.attach.attached.iter().any(|attachment| {
                    attachment.language == language
                        && attachment.root == root
                        && attachment.target == target
                }) {
                    self.retire_superseded_transport(Some(server));
                    self.lsp_did_open_current();
                    return;
                }
                self.lsp_state
                    .attach
                    .attached
                    .retain(|a| !(a.language == language && a.root == root && a.target == target));
                // The placement exists before any didOpen resolves
                // against it — live and replayed alike.
                self.lsp_state.attach.attached.push(attach::Attachment {
                    language: language.clone(),
                    root: root.clone(),
                    server,
                    target: target.clone(),
                });
                match warning {
                    Some(warning) => self.message = format!("lsp: {warning}"),
                    None => self.message = format!("lsp: {name} starting"),
                }
                let transport = self
                    .lsp_state
                    .attach
                    .transport
                    .lock()
                    .ok()
                    .and_then(|mut table| table.remove(&server));
                match transport {
                    Some(attach::LiveTransport { client, rx }) => {
                        // TUI: forward like every late-attaching server.
                        if let Some(app_tx) = &self.app_tx {
                            let tx = app_tx.clone();
                            std::thread::spawn(move || {
                                while let Ok(event) = rx.recv() {
                                    if tx
                                        .send(crate::editor::events::AppEvent::Lsp(event))
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            });
                            let (_, empty) = channel();
                            self.lsp_servers.push(LspServer {
                                id: server,
                                client: Some(client),
                                rx: empty,
                                ready: false,
                            });
                        } else {
                            self.lsp_servers.push(LspServer {
                                id: server,
                                client: Some(client),
                                rx,
                                ready: false,
                            });
                        }
                    }
                    None => {
                        // Replayed server: identity only, replies arrive
                        // through the injected event stream.
                        self.lsp_servers.push(LspServer {
                            id: server,
                            client: None,
                            rx: channel().1,
                            ready: false,
                        });
                    }
                }
                self.lsp_did_open_current();
            }
            decision => {
                if matches!(
                    decision,
                    attach::AttachDecision::TrustRequired { .. }
                        | attach::AttachDecision::TrustError { .. }
                ) {
                    self.lsp_state
                        .attach
                        .trust_roots
                        .insert(key.clone(), root.clone());
                }
                let sticky = !matches!(
                    decision,
                    attach::AttachDecision::TrustRequired { .. }
                        | attach::AttachDecision::TrustError { .. }
                        | attach::AttachDecision::RemoteIo { .. }
                );
                let first = self
                    .lsp_state
                    .attach
                    .refused
                    .insert(key, decision.clone())
                    .is_none();
                if sticky && !first {
                    return;
                }
                self.message = match decision {
                    attach::AttachDecision::NoServer => format!("no language server for {name}"),
                    attach::AttachDecision::TrustRequired { command } => {
                        format!("project config wants to run `{command}` — :trust to allow (once)")
                    }
                    attach::AttachDecision::TrustError { error } => {
                        format!("project trust: {error}")
                    }
                    attach::AttachDecision::NotExecutable {
                        command,
                        reason,
                        hint,
                    } => {
                        format!("lsp: {command} {reason} — {hint}")
                    }
                    attach::AttachDecision::SpawnFailed { reason } => {
                        format!("lsp: {name} could not start — {reason}")
                    }
                    attach::AttachDecision::RemoteIo { reason } => {
                        format!("lsp: remote discovery failed — {reason}")
                    }
                    attach::AttachDecision::Attached | attach::AttachDecision::Cancelled => {
                        unreachable!("matched above")
                    }
                };
                if let Some(warning) = warning {
                    self.message = format!("{} — {}", self.message, warning);
                }
            }
        }
    }

    /// Record newly reported malformed-layer diagnostics (0033 §2),
    /// deduped: every later attach for the same layers is already
    /// covered.
    fn record_layer_diagnostics(&mut self, layers: &[strop_lsp::languages::LayerDiagnostic]) {
        for diagnostic in layers {
            let state = &mut self.lsp_state.attach;
            if !state.layer_diagnostics.contains(diagnostic) {
                state.layer_diagnostics.push(diagnostic.clone());
            }
        }
    }

    /// The first recorded layer diagnostic, when any — readiness and
    /// later messages must not erase it (0033 §2).
    pub(super) fn layer_warning(&self) -> Option<String> {
        layer_suffix(&self.lsp_state.attach.layer_diagnostics)
    }

    /// A superseded discovery may already have published a live
    /// transport for its server: retire it so the attempt leaves no
    /// orphan process and no undrained event stream.
    fn retire_superseded_transport(&mut self, server: Option<ServerId>) {
        let Some(server) = server else { return };
        let transport = self
            .lsp_state
            .attach
            .transport
            .lock()
            .ok()
            .and_then(|mut table| table.remove(&server));
        if let Some(attach::LiveTransport { client, .. }) = transport {
            // Joining never blocks the input thread (same policy as
            // lsp_failed).
            std::thread::spawn(move || {
                client.shutdown();
                client.wait(std::time::Duration::from_secs(2));
            });
        }
    }

    /// Retire remote servers whose workspace no longer has a document:
    /// closing the last owning remote workspace retires its server
    /// (0036 RW8) — no orphan ssh, no orphan remote process. Local
    /// servers keep their session-long lifetime.
    pub(crate) fn lsp_retire_remote_servers(&mut self) {
        let retired: Vec<ServerId> = self
            .lsp_state
            .attach
            .attached
            .iter()
            .filter(|a| a.target.is_remote())
            .filter(|a| {
                let endpoint = match &a.target {
                    Filesystem::Remote(endpoint) => endpoint,
                    _ => return false,
                };
                !self.docs.iter().any(|(id, document)| {
                    document.remote_metadata().is_some_and(|source| {
                        source.file.endpoint() == endpoint
                            && source.file.path().starts_with(&a.root)
                            && lsp_language(source.file.path()) == Some(a.language.as_str())
                            && source.window.is_complete()
                            && !self.remote_following(id)
                    })
                })
            })
            .map(|a| a.server)
            .collect();
        for server in retired {
            self.lsp_retire_server(server, "remote workspace closed");
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

impl Editor {
    pub(crate) fn remote_trust_target(&self) -> Result<strop_workspace::RemoteFile, String> {
        let doc = self
            .lsp_current_doc_path()
            .ok_or("trust requires a file buffer")?;
        let Filesystem::Remote(endpoint) = doc.filesystem else {
            return Err("not a remote workspace".into());
        };
        let language = lsp_language(&doc.path).ok_or("no language server for this file")?;
        let key = AttachKey {
            target: Filesystem::Remote(endpoint.clone()),
            language: language.to_owned(),
            path: doc.path,
        };
        let root = self
            .lsp_state
            .attach
            .trust_roots
            .get(&key)
            .ok_or("no pending remote project trust request")?;
        strop_workspace::RemoteFile::from_path(endpoint, root.clone())
            .map_err(|error| error.to_string())
    }
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

/// Attach completions reach the structured trace with their outcome
/// and any malformed-layer diagnostics (0033 §2/§3) — silence is not a
/// report. Runs at handler entry, before ownership decisions.
fn trace_attach(record: &attach::AttachRecord) {
    use strop_trace::{record_with, EventKind};
    record_with(EventKind::JobFinished, || {
        let mut value = serde_json::json!({
            "service": "lsp",
            "result": "attach",
            "outcome": record.outcome.label(),
            "language": record.language,
            "name": record.name,
            "server": record.server,
            "target": record.target.label(),
            "root": trace::services::NativePath(record.root.clone()),
            "layers": &record.layers,
        });
        match &record.outcome {
            attach::AttachDecision::NotExecutable {
                command,
                reason,
                hint,
            } => {
                value["command"] = serde_json::json!(command);
                value["reason"] = serde_json::json!(reason);
                value["hint"] = serde_json::json!(hint);
            }
            attach::AttachDecision::SpawnFailed { reason }
            | attach::AttachDecision::RemoteIo { reason } => {
                value["reason"] = serde_json::json!(reason);
            }
            _ => {}
        }
        value
    });
}
