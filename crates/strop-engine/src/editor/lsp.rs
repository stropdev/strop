//! Editor-side LSP event handling and asynchronous navigation. Local
//! and remote documents share the request/server/incarnation/revision
//! ownership; a remote workspace adds the endpoint to every identity —
//! diagnostics, bindings and navigation never alias a remote path onto
//! the local disk (0036 RW8).

use super::{trace, Editor};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use strop_lsp::registry;
use strop_lsp::{LspEvent, ServerId};
use strop_workspace::{Filesystem, ResourceLocation};

pub(crate) mod attach;
mod lifecycle;
mod navigation;
pub(crate) mod remote;
mod routing;
pub(crate) mod state;
#[cfg(test)]
mod tests;

pub struct LspServer {
    pub id: ServerId,
    /// None for replayed servers: identity and replies come from the
    /// injected record/event stream — never a fake client.
    pub client: Option<strop_lsp::Client>,
    pub rx: Receiver<LspEvent>,
    pub ready: bool,
}

impl Editor {
    pub(crate) fn open_hover_document(&mut self) {
        let Some(text) = self.hover_card.take() else {
            return;
        };
        self.push_jump();
        let mut document = super::Document::documentation(strop_core::Buffer::from_text(&text));
        document.set_return_point(self.jump_record());
        let id = self.docs.insert(document);
        self.switch_to(id);
        self.set_head(0);
        self.view_mut().view_top = 0;
        self.message = "documentation: search/scroll normally; Ctrl-O returns".into();
    }
}

impl Editor {
    /// The current document's path identity: a local absolute path, or
    /// the canonical remote file's endpoint-scoped path. Remote
    /// windows must be complete for language services (0036 RW8) —
    /// partial/follow windows refuse, they never pretend.
    pub(super) fn lsp_current_doc_path(&self) -> Option<ResourceLocation> {
        if self.cur().remote_metadata().is_some() && !self.remote_window_complete() {
            return None;
        }
        self.lsp_doc_path(self.current())
    }

    fn lsp_doc_path(&self, document: strop_core::id::DocumentId) -> Option<ResourceLocation> {
        let document = self.docs.get(document)?;
        match &document.source {
            crate::editor::document::DocumentSource::Remote(file) => {
                Some(ResourceLocation::remote(
                    file.file.endpoint().clone(),
                    file.file.path().to_path_buf(),
                ))
            }
            crate::editor::document::DocumentSource::Container { container, path } => {
                Some(ResourceLocation {
                    filesystem: strop_workspace::Filesystem::Container(container.clone()),
                    path: path.clone(),
                })
            }
            _ => document
                .buf
                .path
                .as_ref()
                .map(|path| ResourceLocation::local(self.cwd.join(path))),
        }
    }

    /// A canonical remote file on `endpoint`, when any open document
    /// still owns one — the `with_path` seed for remote navigation.
    pub(super) fn remote_file_for(
        &self,
        endpoint: &strop_workspace::RemoteEndpoint,
    ) -> Option<strop_workspace::RemoteFile> {
        self.docs.iter().find_map(|(_, document)| {
            match &document.source {
                crate::editor::document::DocumentSource::Remote(source) => Some(&source.file),
                _ => None,
            }
            .filter(|file| file.endpoint() == endpoint)
            .cloned()
        })
    }

    pub(crate) fn lsp_locations(&mut self, kind: strop_lsp::LocKind) {
        self.lsp_request(strop_lsp::RequestKind::Locations(kind));
    }
    pub(crate) fn lsp_hover(&mut self) {
        self.lsp_request(strop_lsp::RequestKind::Hover);
    }
    pub(crate) fn lsp_goto_definition(&mut self) {
        self.lsp_request(strop_lsp::RequestKind::Goto);
    }
    pub(crate) fn lsp_switch_source_header(&mut self) {
        self.lsp_request(strop_lsp::RequestKind::SwitchHeader);
    }
    /// `:format` — server formatting through a change plan (0043).
    /// auto_format admission: a binding with a live, formatting-capable
    /// client. Anything less saves without formatting.
    pub(crate) fn lsp_format_available(&self) -> bool {
        let Some(binding) = self.lsp_state.bindings.get(&self.current()) else {
            return false;
        };
        self.lsp_live_client(binding.server)
            .is_some_and(|client| client.caps().formatting())
    }

    /// The format reply concluded: run the save that was waiting on it
    /// (config auto_format). Never fires twice — the slot is taken.
    pub(crate) fn continue_after_format(
        &mut self,
        context: &strop_lsp::ReplyContext,
        warning: Option<String>,
    ) {
        if context.kind != strop_lsp::RequestKind::Format
            || !matches!(
                self.lsp_state.after_format.as_ref(),
                Some(state::AfterFormat::Save { request, .. }) if *request == context.stamp
            )
        {
            return;
        }
        let Some(state::AfterFormat::Save {
            document,
            close,
            force,
            request,
        }) = self.lsp_state.after_format.take()
        else {
            return;
        };
        if warning.is_some()
            && self
                .docs
                .get(document)
                .is_none_or(|source| source.buf.revision() != request.revision)
        {
            self.message = "write not started: source changed while formatting — repeat :w to save newer edits".into();
            return;
        }
        let admitted = self.request_save_document(document, None, force, close);
        if let Some(warning) = warning {
            if admitted {
                self.io.format_warnings.insert(document, warning);
            } else {
                self.message
                    .push_str(&format!(" — format warning: {warning}"));
            }
        }
    }

    pub(crate) fn lsp_format(&mut self) {
        self.lsp_change_request(strop_lsp::RequestKind::Format, None);
    }
    /// `:rename <new>` — workspace rename through a change plan.
    pub(crate) fn lsp_rename(&mut self, new_name: &str) {
        self.lsp_change_request(strop_lsp::RequestKind::Rename, Some(new_name.to_string()));
    }
    /// `Space s` — the current document's symbols as a picker (0047 §1).
    pub(crate) fn lsp_document_symbols(&mut self) {
        self.lsp_request(strop_lsp::RequestKind::DocumentSymbols);
    }
    /// `Space a` — code actions at the cursor, offered as a picker.
    pub(crate) fn lsp_code_actions(&mut self) {
        self.lsp_change_request(strop_lsp::RequestKind::CodeAction, None);
    }

    pub(crate) fn jump_diagnostic(&mut self, forward: bool) {
        let Some(diags) = self
            .diags_for(self.current())
            .filter(|diags| !diags.is_empty())
        else {
            self.message = "no diagnostics".into();
            return;
        };
        let cur = self.buf().line_of(self.head());
        let col = self.buf().col_of(self.head());
        let target = if forward {
            diags
                .iter()
                .find(|d| d.line.get() > cur || (d.line.get() == cur && d.col.get() > col))
                .or(diags.first())
        } else {
            diags
                .iter()
                .rev()
                .find(|d| d.line.get() < cur || (d.line.get() == cur && d.col.get() < col))
                .or(diags.last())
        };
        let Some(d) = target else {
            return;
        };
        let (line, col, msg) = (d.line.get(), d.col.get(), d.message.clone());
        let start = self
            .buf()
            .line_start(line.min(self.buf().len_lines().saturating_sub(1)));
        self.set_head(self.buf().clamp_boundary(start + col));
        self.clamp_cursor();
        self.scroll_to_cursor(self.view_rows());
        self.message = msg;
    }

    pub(crate) fn open_diagnostics_picker(&mut self) {
        use strop_picker::{Item, Kind, Payload};
        // Deterministic row order across hash seeds (R11).
        let mut by_doc: Vec<_> = self
            .diags
            .keys()
            .filter_map(|&id| Some((self.lsp_doc_path(id)?, self.diags_for(id)?)))
            .collect();
        by_doc.sort_by(|a, b| {
            (a.0.filesystem.label(), &a.0.path).cmp(&(b.0.filesystem.label(), &b.0.path))
        });
        let mut items: Vec<Item> = Vec::new();
        for (doc, diags) in by_doc {
            for d in diags {
                let line = d.line.get() + 1;
                let col = d.col.get() + 1;
                match &doc.filesystem {
                    Filesystem::Local => items.push(Item {
                        badge: None,
                        text: format!(
                            "{}:{} {} {}",
                            doc.path.display(),
                            line,
                            d.severity_char(),
                            d.message
                        ),
                        payload: Payload::Grep {
                            path: doc.path.clone(),
                            line,
                            col,
                            match_len: 1,
                            line_text: d.message.clone(),
                        },
                    }),
                    // Remote diagnostics carry their endpoint: the
                    // preview stays local-clean and acceptance opens
                    // the remote target (0036).
                    Filesystem::Remote(endpoint) => items.push(Item {
                        badge: None,
                        text: format!(
                            "{}{}:{} {} {}",
                            endpoint,
                            doc.path.display(),
                            line,
                            d.severity_char(),
                            d.message
                        ),
                        payload: Payload::Remote {
                            endpoint: endpoint.clone(),
                            path: doc.path.clone(),
                            line,
                            col,
                        },
                    }),
                    // Unreachable in DC1a (no container bindings); if one
                    // ever arrives it is dropped with a trace, never
                    // aliased to a local path.
                    Filesystem::Container(_) => {
                        trace::services::rejected(
                            "lsp",
                            "diagnostic in a container namespace (unwired)",
                        );
                    }
                }
            }
        }
        if items.is_empty() {
            self.message = "no diagnostics".into();
            return;
        }
        self.set_picker(super::PickerGlue::diagnostics(strop_picker::Picker::new(
            Kind::Diagnostics,
            items,
            false,
        )));
    }

    pub(crate) fn lsp_goto_definition_pub(&mut self) {
        self.lsp_goto_definition();
    }
    pub(crate) fn lsp_switch_source_header_pub(&mut self) {
        self.lsp_switch_source_header();
    }
    pub(crate) fn lsp_hover_pub(&mut self) {
        self.lsp_hover();
    }
    pub fn lsp_code_actions_pub(&mut self) {
        self.lsp_code_actions();
    }
    pub fn lsp_document_symbols_pub(&mut self) {
        self.lsp_document_symbols();
    }
    pub fn lsp_locations_pub(&mut self, kind: strop_lsp::LocKind) {
        self.lsp_locations(kind);
    }
    pub fn jump_diagnostic_pub(&mut self, forward: bool) {
        self.jump_diagnostic(forward);
    }
}

/// The LSP language for a path, from the embedded extension table —
/// pure, in-memory, safe on every keystroke.
pub(crate) fn lsp_language(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    registry::language_for_extension_name(ext)
}

/// The didOpen languageId sent to servers.
pub(crate) fn lang_id(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("py") | Some("pyi") => "python",
        Some("go") => "go",
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript",
        Some("ts") => "typescript",
        Some("tsx") => "typescriptreact",
        Some("json") => "json",
        Some("sh") | Some("bash") => "shellscript",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") | Some("hh") => "cpp",
        _ => "plaintext",
    }
}
