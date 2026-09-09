//! Live document bindings and original request ownership. Lifecycle
//! calls run through the replay tape (R11): model owners update
//! identically live and replayed; only the native wire work is gated.
use std::collections::HashMap;
use std::path::PathBuf;

use strop_core::id::{BufferRevision, ByteColumn, DocumentId, LineIndex};
use strop_lsp::{
    Client, ReplyContext, RequestInput, RequestKind, RequestRefusal, RequestStamp, ServerId,
};

use super::super::Editor;
use super::attach::AttachState;

pub(crate) struct Binding {
    pub server: ServerId,
    pub path: PathBuf,
    pub root: PathBuf,
    /// Which filesystem `path` names — remote bindings never alias
    /// same-bytes local paths (0036 RW8).
    pub target: strop_workspace::Filesystem,
    pub revision: BufferRevision,
}

/// Tape arguments for open/change — identity only; document content
/// never enters the trace (metadata exports drop content-bearing
/// fields, and R6 forbids per-change full-text materialization).
#[derive(Debug, serde::Serialize)]
pub(crate) struct SyncArgs {
    pub server: ServerId,
    pub document: DocumentId,
    pub revision: BufferRevision,
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
    pub bytes: usize,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct CloseArgs {
    pub server: ServerId,
    pub document: DocumentId,
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
}

pub(crate) struct LspState {
    pub bindings: HashMap<DocumentId, Binding>,
    pub hover: Option<RequestStamp>,
    pub navigation: Option<RequestStamp>,
    pub attach: AttachState,
}

impl Default for LspState {
    fn default() -> Self {
        Self {
            bindings: HashMap::new(),
            hover: None,
            navigation: None,
            attach: AttachState::new(),
        }
    }
}

impl Editor {
    fn lsp_live_client(&self, server: ServerId) -> Option<Client> {
        self.lsp_servers
            .iter()
            .find(|s| s.id == server)
            .and_then(|s| s.client.clone())
    }

    pub(super) fn lsp_did_open_current(&mut self) {
        let document = self.current();
        let Some(doc) = self.lsp_current_doc_path() else {
            return;
        };
        let Some(language) = super::lsp_language(&doc.path) else {
            return;
        };
        let Some((server, root)) = self.lsp_server_for(&doc.path, language, &doc.filesystem) else {
            return;
        };
        if let Some(binding) = self.lsp_state.bindings.get(&document) {
            if binding.server == server
                && binding.path == doc.path
                && binding.target == doc.filesystem
            {
                return;
            }
            self.lsp_close_document(document);
        }
        let revision = self.buf().revision();
        let text = self.buf().snapshot();
        let args = SyncArgs {
            server,
            document,
            revision,
            path: doc.path.clone(),
            bytes: text.len_bytes(),
        };
        // Replay reproduces the recorded admission result; the binding
        // updates identically so injected replies pass freshness.
        let opened = self.tape.call("lsp.open", &args, || {
            self.lsp_live_client(server).map(|client| {
                client.did_open(
                    document,
                    revision,
                    &doc.path,
                    super::lang_id(&doc.path),
                    text,
                )
            })
        });
        match opened {
            Ok(Some(true)) => {
                self.lsp_state.bindings.insert(
                    document,
                    Binding {
                        server,
                        path: doc.path,
                        root,
                        target: doc.filesystem,
                        revision,
                    },
                );
            }
            // A replayed refusal or a vanished connection: no binding.
            Ok(_) => {}
            Err(error) => self.message = format!("lsp open diverged from trace: {error}"),
        }
    }

    pub fn lsp_sync_changed(&mut self) {
        // Journal consumers can edit a non-current document; sync every
        // live binding in a deterministic order.
        let mut changed: Vec<_> = self
            .lsp_state
            .bindings
            .iter()
            .filter_map(|(&id, binding)| {
                let doc = self.docs.get(id)?;
                let revision = doc.buf.revision();
                (revision != binding.revision).then(|| {
                    (
                        id,
                        binding.server,
                        binding.path.clone(),
                        revision,
                        doc.buf.snapshot(),
                    )
                })
            })
            .collect();
        changed.sort_by_key(|(id, _, _, _, _)| *id);
        for (document, server, path, revision, text) in changed {
            let args = SyncArgs {
                server,
                document,
                revision,
                path: path.clone(),
                bytes: text.len_bytes(),
            };
            match self.tape.call("lsp.change", &args, || {
                self.lsp_live_client(server)
                    .map(|client| client.did_change(document, revision, &path, text))
            }) {
                Ok(Some(true)) => {
                    if let Some(binding) = self.lsp_state.bindings.get_mut(&document) {
                        binding.revision = revision;
                    }
                }
                Ok(_) => self.message = "lsp: document change refused".into(),
                Err(error) => self.message = error.to_string(),
            }
        }
    }

    pub(crate) fn lsp_close_document(&mut self, document: DocumentId) {
        self.diags.remove(&document);
        if !self.docs.is_empty() && document == self.current() {
            self.hover_card = None;
        }
        // Model owner removal happens in both modes; only the native
        // didClose notification is gated.
        if let Some(binding) = self.lsp_state.bindings.remove(&document) {
            let args = CloseArgs {
                server: binding.server,
                document,
                path: binding.path.clone(),
            };
            match self.tape.request("lsp.close", &args) {
                Ok(true) => {
                    if let Some(client) = self.lsp_live_client(binding.server) {
                        client.did_close(document, &binding.path);
                    }
                }
                Ok(false) => {}
                Err(error) => self.message = format!("lsp close diverged from trace: {error}"),
            }
        }
        if self.lsp_state.hover.is_some_and(|r| r.document == document) {
            self.lsp_state.hover = None;
            self.hover_card = None;
        }
        if self
            .lsp_state
            .navigation
            .is_some_and(|r| r.document == document)
        {
            self.lsp_state.navigation = None;
        }
        if self
            .picker
            .as_ref()
            .and_then(|p| p.lsp_context)
            .is_some_and(|c| c.stamp.document == document)
        {
            self.close_picker();
        }
        // Closing the last owning remote workspace retires its server.
        self.lsp_retire_remote_servers();
    }

    pub(crate) fn lsp_reply_fresh(&self, context: &ReplyContext) -> bool {
        let stamp = context.stamp;
        let expected = if context.kind == RequestKind::Hover {
            self.lsp_state.hover
        } else {
            self.lsp_state.navigation
        };
        expected == Some(stamp) && self.lsp_context_fresh(context)
    }

    /// An accepted result may transfer to a picker or I/O ticket after its
    /// server request is terminal. The original document/server/revision still
    /// has to be current; the new subsystem owns cancellation after transfer.
    pub(crate) fn lsp_context_fresh(&self, context: &ReplyContext) -> bool {
        let stamp = context.stamp;
        let newer = if context.kind == RequestKind::Hover {
            self.lsp_state.hover
        } else {
            self.lsp_state.navigation
        };
        !self.docs.is_empty()
            && stamp.document == self.current()
            && newer.is_none_or(|owner| owner == stamp)
            && self
                .docs
                .get(stamp.document)
                .is_some_and(|d| d.buf.revision() == stamp.revision)
            && self
                .lsp_state
                .bindings
                .get(&stamp.document)
                .is_some_and(|b| b.server == stamp.server && b.revision == stamp.revision)
    }

    pub(super) fn finish_lsp_reply(&mut self, context: &ReplyContext) -> bool {
        let fresh = self.lsp_reply_fresh(context);
        let slot = if context.kind == RequestKind::Hover {
            &mut self.lsp_state.hover
        } else {
            &mut self.lsp_state.navigation
        };
        if *slot == Some(context.stamp) {
            *slot = None;
        }
        fresh
    }

    pub(super) fn lsp_request(&mut self, kind: RequestKind) {
        let hover = kind == RequestKind::Hover;
        if hover {
            self.lsp_state.hover = None;
        } else {
            self.lsp_state.navigation = None;
            self.cancel_open(strop_core::worker::CancelReason::Superseded);
        }
        let Some(doc) = self.lsp_current_doc_path() else {
            self.message =
                "language services require a complete file buffer, not a partial/follow view"
                    .into();
            return;
        };
        let Some(language) = super::lsp_language(&doc.path) else {
            self.message = "no language server for this file type".into();
            return;
        };
        let Some((server, _)) = self.lsp_server_for(&doc.path, language, &doc.filesystem) else {
            match doc.filesystem {
                strop_workspace::Filesystem::Local => {
                    self.message = "no language server — install it or fix languages.toml".into()
                }
                strop_workspace::Filesystem::Remote(endpoint) => {
                    self.message = format!(
                        "no language server on {endpoint} — install it there or fix languages.toml"
                    )
                }
            }
            return;
        };
        self.lsp_did_open_current();
        self.lsp_sync_changed();
        let Some(doc) = self.lsp_current_doc_path() else {
            return;
        };
        let line = self.buf().line_of(self.head());
        let input = RequestInput {
            document: self.current(),
            revision: self.buf().revision(),
            path: doc.path.clone(),
            line: LineIndex::new(line),
            byte_col: ByteColumn::new(self.buf().col_of(self.head())),
            line_text: strop_lsp::FrozenLine::from_slice(
                self.buf()
                    .text()
                    .byte_slice(self.buf().line_start(line)..self.buf().line_end(line)),
            ),
            kind,
        };
        let native_input = input.clone();
        let prepared = self.tape.call("lsp.prepare", &input, || {
            let client = self
                .lsp_live_client(server)
                .ok_or(RequestRefusal::NotOpen)?;
            client.prepare_request(native_input)
        });
        match prepared {
            Ok(Ok(prepared)) => {
                // Register the owner stamp before launching; replayed
                // replies validate against exactly this stamp.
                if hover {
                    self.lsp_state.hover = Some(prepared.stamp);
                } else {
                    self.lsp_state.navigation = Some(prepared.stamp);
                }
                if let RequestKind::Locations(kind) = kind {
                    self.message = format!("lsp: {} …", kind.label());
                }
                match self.tape.request("lsp.launch", &prepared) {
                    Ok(true) => {
                        if let Some(client) = self.lsp_live_client(server) {
                            client.launch_request(prepared);
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        if hover {
                            self.lsp_state.hover = None;
                        } else {
                            self.lsp_state.navigation = None;
                        }
                        self.message = format!("lsp request diverged from trace: {error}");
                    }
                }
            }
            Ok(Err(refusal)) => {
                self.message = match refusal {
                    RequestRefusal::NotOpen => {
                        "lsp: the document is not open on this server".into()
                    }
                    RequestRefusal::StaleRevision => format!(
                        "lsp: buffer changed while syncing — repeat {}",
                        kind.label()
                    ),
                    RequestRefusal::Unsupported => {
                        format!("lsp: {} is not supported by this server", kind.label())
                    }
                    RequestRefusal::IdentityExhausted => "lsp: request identities exhausted".into(),
                };
            }
            Err(error) => self.message = format!("lsp prepare diverged from trace: {error}"),
        }
    }

    pub(super) fn lsp_failed(&mut self, server: ServerId) {
        let mut docs: Vec<_> = self
            .lsp_state
            .bindings
            .iter()
            .filter_map(|(&id, b)| (b.server == server).then_some(id))
            .collect();
        docs.sort();
        for document in docs {
            self.lsp_close_document(document);
        }
        self.lsp_state
            .attach
            .attached
            .retain(|a| a.server != server);
        if let Some(index) = self.lsp_servers.iter().position(|s| s.id == server) {
            let connection = self.lsp_servers.remove(index);
            if let Some(client) = connection.client {
                // Joining a dead/failed server never blocks the input thread.
                std::thread::spawn(move || {
                    client.shutdown();
                    client.wait(std::time::Duration::from_secs(2));
                });
            }
        }
    }
}
