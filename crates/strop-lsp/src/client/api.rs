//! client/api.rs — the editor-facing surface: document sync
//! (didOpen/didChange) and the requests (hover, goto, locations,
//! clangd's switchSourceHeader).

use std::path::Path;

use async_lsp::lsp_types::notification::{DidChangeTextDocument, DidOpenTextDocument};
use async_lsp::lsp_types::request::{GotoDefinition, HoverRequest};
use async_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, GotoDefinitionParams, HoverParams,
    Position, TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams,
    VersionedTextDocumentIdentifier, WorkDoneProgressParams,
};

use crate::convert::hover_text;
use crate::protocol::{LocKind, LspEvent, PendingRequest, PositionEncoding, QueuedRequest};

use super::wire::{is_content_modified, request_locations};
use super::{Client, SwitchSourceHeader};

impl Client {
    /// didOpen — full text, full sync (simplest correct; incremental sync
    /// is the perf follow-up, noted in 0009 §3).
    pub fn did_open(&self, path: &Path, language_id: &str, text: &str) {
        // initialize must hit the wire first — queue until it has
        if !self.initialized.load(std::sync::atomic::Ordering::Relaxed) {
            self.pending_opens.lock().push((
                path.to_path_buf(),
                language_id.to_string(),
                text.to_string(),
            ));
            return;
        }
        let Some(uri) = self.uri(path) else { return };
        self.versions.lock().insert(path.to_path_buf(), 1);
        let socket = self.socket.clone();
        let item = TextDocumentItem {
            uri,
            language_id: language_id.to_string(),
            version: 1,
            text: text.to_string(),
        };
        let _ = socket.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: item,
        });
    }

    /// The negotiated column encoding — callers convert at the boundary.
    pub fn encoding(&self) -> PositionEncoding {
        self.caps.encoding()
    }

    /// didChange — full document replacement (TextDocumentSyncKind::Full).
    /// The protocol version this client last sent for a path (0020 §6:
    /// diagnostics are fresh when their version is at least this —
    /// the buffer's edit epoch is a DIFFERENT clock and must never be
    /// compared against it).
    pub fn sent_version(&self, path: &Path) -> Option<i32> {
        self.versions.lock().get(path).copied()
    }

    pub fn did_change(&self, path: &Path, text: &str) {
        let Some(uri) = self.uri(path) else { return };
        let socket = self.socket.clone();
        // strictly increasing per the spec — "full sync doesn't care"
        // was wrong: pyright rejects stale versions (0014)
        let version = {
            let mut m = self.versions.lock();
            let v = m.entry(path.to_path_buf()).or_insert(1);
            *v += 1;
            *v
        };
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version },
            content_changes: vec![async_lsp::lsp_types::TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.to_string(),
            }],
        };
        let _ = socket.notify::<DidChangeTextDocument>(params);
    }

    /// Hover at (line, col) — UTF-8 converted at the boundary. The
    /// response posts onto the channel as HoverText (or nothing).
    /// Quiet no-op when the server doesn't advertise hover (0009 §2.5).
    pub fn hover(&self, path: &Path, line: usize, col: usize) {
        if !self.initialized.load(std::sync::atomic::Ordering::Relaxed) {
            self.pending_requests.lock().push(PendingRequest {
                path: path.to_path_buf(),
                line,
                col,
                req_revision: 0,
                kind: QueuedRequest::Hover,
            });
            return;
        }
        if !self.caps.hover() {
            return;
        }
        let Some(uri) = self.uri(path) else { return };
        let sock = self.socket.clone();
        let tx = self.tx.clone();
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: line as u32,
                    character: col as u32,
                },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        self.handle.spawn(async move {
            // -32801 "content modified": servers reject during their
            // initial index — one retry after a beat (helix does the same
            // class of dance)
            let mut resp = sock.request::<HoverRequest>(params.clone()).await;
            if is_content_modified(&resp) {
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                resp = sock.request::<HoverRequest>(params).await;
            }
            if let Ok(Some(hover)) = resp {
                let text = hover_text(&hover);
                if !text.is_empty() {
                    let _ = tx.send(LspEvent::HoverText { text });
                }
            }
        });
    }

    /// Goto-definition; response posts as GotoLocation. Quiet no-op when
    /// the server doesn't advertise definitions (0009 §2.5).
    pub fn goto_definition(&self, path: &Path, line: usize, col: usize, req_revision: u64) {
        // pre-init: caps unknown ≠ unsupported — queue, flush on
        // Initialized (gd right after opening a project used to die
        // silently here)
        if !self.initialized.load(std::sync::atomic::Ordering::Relaxed) {
            self.pending_requests.lock().push(PendingRequest {
                path: path.to_path_buf(),
                line,
                col,
                req_revision,
                kind: QueuedRequest::Goto,
            });
            return;
        }
        if !self.caps.goto_definition() {
            return;
        }
        let Some(uri) = self.uri(path) else { return };
        let sock = self.socket.clone();
        let tx = self.tx.clone();
        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: line as u32,
                    character: col as u32,
                },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: Default::default(),
        };
        self.handle.spawn(async move {
            let mut resp = sock.request::<GotoDefinition>(params.clone()).await;
            if is_content_modified(&resp) {
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                resp = sock.request::<GotoDefinition>(params).await;
            }
            if let Ok(Some(resp)) = resp {
                use async_lsp::lsp_types::GotoDefinitionResponse as R;
                let loc = match resp {
                    R::Scalar(l) => Some(l),
                    R::Array(v) => v.into_iter().next(),
                    R::Link(v) => v
                        .into_iter()
                        .next()
                        .map(|l| async_lsp::lsp_types::Location {
                            uri: l.target_uri,
                            range: l.target_selection_range,
                        }),
                };
                if let Some(l) = loc {
                    if let Ok(path) = l.uri.to_file_path() {
                        let _ = tx.send(LspEvent::GotoLocation {
                            path,
                            line: l.range.start.line as usize,
                            col: l.range.start.character as usize,
                            req_revision,
                        });
                    }
                }
            }
        });
    }

    /// references / implementation / type-definition / declaration:
    /// one shape, four LSP methods; the response posts as Locations.
    pub fn locations(
        &self,
        kind: LocKind,
        path: &Path,
        line: usize,
        col: usize,
        req_revision: u64,
    ) {
        // pre-init: caps unknown ≠ unsupported — queue like gd (0015)
        if !self.initialized.load(std::sync::atomic::Ordering::Relaxed) {
            self.pending_requests.lock().push(PendingRequest {
                path: path.to_path_buf(),
                line,
                col,
                req_revision,
                kind: QueuedRequest::Locations(kind),
            });
            return;
        }
        let supported = match kind {
            LocKind::References => self.caps.references(),
            LocKind::Implementation => self.caps.implementation(),
            LocKind::TypeDefinition => self.caps.type_definition(),
            LocKind::Declaration => self.caps.declaration(),
        };
        if !supported {
            return; // quiet no-op like gd (0009 §2.5)
        }
        let Some(uri) = self.uri(path) else { return };
        let sock = self.socket.clone();
        let tx = self.tx.clone();
        let tdp = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position {
                line: line as u32,
                character: col as u32,
            },
        };
        self.handle
            .spawn(request_locations(sock, tx, kind, tdp, req_revision));
    }

    /// clangd's `textDocument/switchSourceHeader` (a clangd extension,
    /// absent from lsp-types' request set): the .cpp ↔ .h jump. The
    /// counterpart posts as GotoLocation at its top; "no counterpart"
    /// and unsupported servers surface as a Note, never an error.
    pub fn switch_source_header(&self, path: &Path) {
        if !self.initialized.load(std::sync::atomic::Ordering::Relaxed) {
            self.pending_requests.lock().push(PendingRequest {
                path: path.to_path_buf(),
                line: 0,
                col: 0,
                req_revision: 0,
                kind: QueuedRequest::SwitchHeader,
            });
            return;
        }
        let Some(uri) = self.uri(path) else { return };
        let sock = self.socket.clone();
        let tx = self.tx.clone();
        self.handle.spawn(async move {
            match sock
                .request::<SwitchSourceHeader>(TextDocumentIdentifier { uri })
                .await
            {
                Ok(Some(target)) => {
                    if let Ok(path) = target.to_file_path() {
                        let _ = tx.send(LspEvent::GotoLocation {
                            path,
                            line: 0,
                            col: 0,
                            req_revision: 0, // gs is a jump command, not a position answer
                        });
                    }
                }
                Ok(None) => {
                    let _ = tx.send(LspEvent::Note {
                        text: "no header/source counterpart".into(),
                    });
                }
                Err(e) => {
                    let _ = tx.send(LspEvent::Note {
                        text: format!("switch source/header unsupported: {e}"),
                    });
                }
            }
        });
    }
}
