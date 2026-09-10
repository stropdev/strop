//! Editor-side LSP event handling and asynchronous navigation. Local
//! and remote documents share the request/server/incarnation/revision
//! ownership; a remote workspace adds the endpoint to every identity —
//! diagnostics, bindings and navigation never alias a remote path onto
//! the local disk (0036 RW8).

use super::{trace, Editor};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use strop_lsp::protocol::ResolvedDiag;
use strop_lsp::registry;
use strop_lsp::{LspEvent, ServerId};
use strop_workspace::{Filesystem, ResourceLocation};

pub(crate) mod attach;
mod lifecycle;
pub(crate) mod remote;
pub(crate) mod state;
#[cfg(test)]
mod tests;

pub(crate) struct LspServer {
    pub id: ServerId,
    /// None for replayed servers: identity and replies come from the
    /// injected record/event stream — never a fake client.
    pub client: Option<strop_lsp::Client>,
    pub rx: Receiver<LspEvent>,
    pub ready: bool,
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

    pub(crate) fn handle_lsp_event(&mut self, event: LspEvent) {
        trace::services::lsp(&event);
        match event {
            LspEvent::Ready { server, name } => {
                if let Some(owner) = self.lsp_servers.iter_mut().find(|owner| owner.id == server) {
                    owner.ready = true;
                    // Success must not erase a configuration warning
                    // (0033 §2): readiness is reported alongside it.
                    self.message = match self.layer_warning() {
                        Some(warning) => format!("lsp: {name} ready — {warning}"),
                        None => format!("lsp: {name} ready"),
                    };
                }
            }
            LspEvent::Failed { server, name, hint } => {
                if self.lsp_servers.iter().any(|s| s.id == server) {
                    self.lsp_failed(server);
                    self.message = format!("lsp: {name} failed — {hint}");
                } else {
                    trace::services::rejected("lsp", "failure for an unowned server");
                }
            }
            LspEvent::ServerMessage { server, name, text } => {
                if self.lsp_servers.iter().any(|s| s.id == server) {
                    self.message = format!("lsp: {name}: {text}");
                } else {
                    trace::services::rejected("lsp", "message for an unowned server");
                }
            }
            LspEvent::Diagnostics {
                context,
                doc,
                diags,
            } => {
                let valid = self
                    .lsp_state
                    .bindings
                    .get(&context.document)
                    .is_some_and(|b| {
                        b.server == context.server
                            && b.path == doc.path
                            && b.target == doc.filesystem
                            && b.revision == context.revision
                    });
                let Some(doc_buffer) = self
                    .docs
                    .get(context.document)
                    .filter(|d| valid && d.buf.revision() == context.revision)
                else {
                    trace::services::rejected("lsp", "diagnostic owner/revision changed");
                    return;
                };
                let buffer = &doc_buffer.buf;
                let resolved: Vec<ResolvedDiag> = diags
                    .into_iter()
                    .map(|d| d.resolve(context.encoding, buffer))
                    .collect();
                self.diags.insert(
                    context.document,
                    super::diagnostics::DocumentDiagnostics {
                        revision: context.revision,
                        items: resolved,
                    },
                );
            }
            LspEvent::HoverText { context, text } => {
                if !self.finish_lsp_reply(&context) {
                    trace::services::rejected(
                        "lsp",
                        "hover request/server/document/revision changed",
                    );
                    return;
                }
                self.hover_card = Some(text);
            }
            LspEvent::Note { context, text } => {
                if !self.finish_lsp_reply(&context) {
                    trace::services::rejected(
                        "lsp",
                        "navigation request/server/document/revision changed",
                    );
                    return;
                }
                self.message = text;
            }
            LspEvent::Edits { context, edits } => {
                if !self.finish_lsp_reply(&context) {
                    trace::services::rejected("lsp", "edit request owner/revision changed");
                    return;
                }
                if edits.is_empty() {
                    self.message = "already formatted".into();
                    return;
                }
                let Some(location) =
                    self.lsp_state
                        .bindings
                        .get(&context.stamp.document)
                        .map(|binding| ResourceLocation {
                            filesystem: binding.target.clone(),
                            path: binding.path.clone(),
                        })
                else {
                    trace::services::rejected("lsp", "edits for an unbound document");
                    return;
                };
                let plan = self.build_change_plan(
                    super::changes::ChangeProducer::Format,
                    vec![(location, edits)],
                    context.encoding,
                );
                self.apply_change_plan(plan);
            }
            LspEvent::WorkspaceEdits { context, edits } => {
                if !self.finish_lsp_reply(&context) {
                    trace::services::rejected("lsp", "workspace-edit owner/revision changed");
                    return;
                }
                if edits.is_empty() {
                    self.message = format!("lsp: {} made no edits", context.kind.label());
                    return;
                }
                let producer = match context.kind {
                    strop_lsp::RequestKind::Rename => super::changes::ChangeProducer::Rename,
                    _ => super::changes::ChangeProducer::CodeAction,
                };
                let plan = self.build_change_plan(producer, edits, context.encoding);
                self.apply_change_plan(plan);
            }
            LspEvent::ActionList { context, actions } => {
                if !self.finish_lsp_reply(&context) {
                    trace::services::rejected("lsp", "code-action owner/revision changed");
                    return;
                }
                if actions.is_empty() {
                    self.message = "no code actions here".into();
                    return;
                }
                let items = actions
                    .iter()
                    .enumerate()
                    .map(|(index, action)| strop_picker::Item {
                        text: action.title.clone(),
                        payload: strop_picker::Payload::CodeAction(index),
                    })
                    .collect();
                self.changes.pending_actions = actions;
                self.open_picker(strop_picker::Kind::CodeActions);
                self.changes.pending_encoding = context.encoding;
                if let Some(glue) = self.picker.as_mut() {
                    glue.picker.append(items);
                }
            }
            LspEvent::GotoLocation { context, location } => {
                if !self.finish_lsp_reply(&context) {
                    trace::services::rejected(
                        "lsp",
                        "navigation request/server/document/revision changed",
                    );
                    return;
                }
                self.jump_to_location(location, context);
            }
            LspEvent::Locations {
                context,
                kind,
                items,
            } => {
                if !self.finish_lsp_reply(&context) {
                    trace::services::rejected("lsp", "location-list owner changed");
                    return;
                }
                match items.len() {
                    0 => {
                        self.message = format!("no {}", kind.label());
                    }
                    1 => {
                        if let Some(location) = items.into_iter().next() {
                            self.jump_to_location(location, context);
                        }
                    }
                    count => {
                        use strop_picker::{Item, Kind, Payload};
                        let items = items
                            .into_iter()
                            .map(|location| {
                                let line = location.position.line.get() + 1;
                                let col = location.position.column.get() + 1;
                                let text = format!("{}:{}:{}", location.doc.label(), line, col);
                                let payload = match location.doc.filesystem {
                                    Filesystem::Local => Payload::Grep {
                                        path: location.doc.path,
                                        line,
                                        col,
                                        match_len: 1,
                                        line_text: String::new(),
                                    },
                                    Filesystem::Remote(endpoint) => Payload::Remote {
                                        endpoint,
                                        path: location.doc.path,
                                        line,
                                        col,
                                    },
                                };
                                Item { text, payload }
                            })
                            .collect();
                        let mut glue = super::PickerGlue::diagnostics(strop_picker::Picker::new(
                            Kind::Locations,
                            items,
                            false,
                        ));
                        glue.lsp_context = Some(context);
                        self.set_picker(glue);
                        self.message = format!("{count} {}", kind.label());
                    }
                }
            }
        }
    }

    pub(crate) fn jump_to_location(
        &mut self,
        location: strop_lsp::ServerLocation,
        context: strop_lsp::ReplyContext,
    ) {
        if !self.lsp_context_fresh(&context) {
            return;
        }
        let intent = super::io::OpenIntent::LspLocation {
            context,
            position: location.position,
        };
        match location.doc.filesystem {
            Filesystem::Local => self.request_open(location.doc.path, intent),
            Filesystem::Remote(endpoint) => {
                // The target is a file on the replying server's host:
                // resolve it through an open document's canonical seed
                // (`with_path` keeps endpoint + native bytes) — the
                // analogous local path is never opened or probed.
                match self.remote_file_for(&endpoint) {
                    Some(seed) => match seed.with_path(location.doc.path.clone()) {
                        Ok(file) => self
                            .request_target(crate::files::FileTarget::Remote(file.into()), intent),
                        Err(error) => {
                            trace::services::rejected("lsp", "remote navigation target invalid");
                            self.message = format!("lsp: remote target invalid: {error}");
                        }
                    },
                    None => {
                        trace::services::rejected("lsp", "remote navigation endpoint lost");
                        self.message =
                            "lsp: the remote workspace for this target was closed".into();
                    }
                }
            }
        }
    }

    pub(crate) fn finish_lsp_jump(
        &mut self,
        target: strop_core::id::DocumentId,
        position: strop_lsp::ServerPosition,
        context: strop_lsp::ReplyContext,
    ) {
        if !self.lsp_context_fresh(&context) {
            trace::services::rejected("lsp", "navigation changed while target was loading");
            return;
        }
        let Some(target_doc) = self.docs.get(target) else {
            return;
        };
        let Some(binding) = self.lsp_state.bindings.get(&context.stamp.document) else {
            return;
        };
        let outside = match &target_doc.source {
            // A remote target is outside the workspace when its remote
            // path leaves the binding's remote root — never by
            // comparing against local paths.
            crate::editor::document::DocumentSource::Remote(file) => {
                !file.file.path().starts_with(&binding.root)
            }
            _ => target_doc
                .buf
                .path
                .as_ref()
                .is_some_and(|path| !self.cwd.join(path).starts_with(&binding.root)),
        };
        let line = position
            .line
            .get()
            .min(target_doc.buf.len_lines().saturating_sub(1));
        let text = target_doc
            .buf
            .text()
            .byte_slice(target_doc.buf.line_start(line)..target_doc.buf.line_end(line));
        let col = strop_lsp::to_byte_col_slice(text, position.column, context.encoding).get();
        let head = target_doc
            .buf
            .clamp_boundary(target_doc.buf.line_start(line).saturating_add(col));
        self.push_jump();
        self.lsp_state.navigation = None;
        self.switch_to(target);
        if outside && !self.buf().readonly {
            self.buf_mut().readonly = true;
            self.message = "readonly — outside workspace (:set noro to edit)".into();
        }
        self.set_head(head);
        self.clamp_cursor();
        self.scroll_to_cursor(self.view_rows());
        self.lsp_maybe_attach();
    }

    /// A local picker hit with a live LSP request context: the server
    /// that produced the list owns the target's filesystem.
    pub(crate) fn lsp_jump_from_picker(
        &mut self,
        path: PathBuf,
        line: usize,
        col: usize,
        context: strop_lsp::ReplyContext,
    ) {
        self.jump_to_location(
            strop_lsp::ServerLocation {
                doc: ResourceLocation::local(path),
                position: strop_lsp::ServerPosition {
                    line: strop_core::id::LineIndex::new(line.saturating_sub(1)),
                    column: strop_lsp::ServerColumn::new(col.saturating_sub(1)),
                },
            },
            context,
        );
    }

    /// A remote picker hit (locations or diagnostics): re-parse the
    /// endpoint and route through the endpoint's file identity — with
    /// a live request context through the freshness-checked navigation
    /// path (server columns), without one as a direct remote open at a
    /// byte column. The analogous local path is never touched.
    pub(crate) fn lsp_open_remote_hit(
        &mut self,
        endpoint: &strop_workspace::RemoteEndpoint,
        path: &Path,
        line: usize,
        col: usize,
        context: Option<strop_lsp::ReplyContext>,
    ) {
        if let Some(context) = context {
            self.jump_to_location(
                strop_lsp::ServerLocation {
                    doc: ResourceLocation::remote(endpoint.clone(), path.to_owned()),
                    position: strop_lsp::ServerPosition {
                        line: strop_core::id::LineIndex::new(line.saturating_sub(1)),
                        column: strop_lsp::ServerColumn::new(col.saturating_sub(1)),
                    },
                },
                context,
            );
        } else if let Some(seed) = self.remote_file_for(endpoint) {
            match seed.with_path(path.to_owned()) {
                Ok(file) => self.request_target(
                    crate::files::FileTarget::Remote(file.into()),
                    super::io::OpenIntent::Grep {
                        line: strop_core::id::LineIndex::new(line.saturating_sub(1)),
                        column: strop_core::id::ByteColumn::new(col.saturating_sub(1)),
                    },
                ),
                Err(error) => self.message = format!("lsp remote location: {error}"),
            }
        } else {
            self.message = "lsp: the remote workspace for this hit was closed".into();
        }
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
    pub(crate) fn lsp_format(&mut self) {
        self.lsp_change_request(strop_lsp::RequestKind::Format, None);
    }
    /// `:rename <new>` — workspace rename through a change plan.
    pub(crate) fn lsp_rename(&mut self, new_name: &str) {
        self.lsp_change_request(strop_lsp::RequestKind::Rename, Some(new_name.to_string()));
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
