//! Reply admission and presentation, guarded by request ownership.
use crate::editor::{trace, Editor};
use strop_lsp::{protocol::ResolvedDiag, LspEvent};
use strop_workspace::{Filesystem, ResourceLocation};

impl Editor {
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
                    crate::editor::diagnostics::DocumentDiagnostics {
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
                self.continue_after_format(&context, Some(text.clone()));
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
                    self.continue_after_format(
                        &context,
                        Some("format reply was stale; source edits retained".into()),
                    );
                    return;
                }
                if edits.is_empty() {
                    self.message = "already formatted".into();
                    self.continue_after_format(&context, None);
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
                    self.continue_after_format(
                        &context,
                        Some("source binding was lost before formatting".into()),
                    );
                    return;
                };
                let plan = self.build_change_plan(
                    crate::editor::changes::ChangeProducer::Format,
                    vec![(location, edits)],
                    context.encoding,
                );
                let refused = !plan.refused.is_empty()
                    || plan.documents.iter().any(|target| {
                        self.docs
                            .get(target.document)
                            .is_none_or(|document| document.buf.readonly)
                    });
                self.apply_change_plan(plan);
                self.continue_after_format(&context, refused.then(|| self.message.clone()));
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
                    strop_lsp::RequestKind::Rename => {
                        crate::editor::changes::ChangeProducer::Rename
                    }
                    _ => crate::editor::changes::ChangeProducer::CodeAction,
                };
                let plan = self.build_change_plan(producer, edits, context.encoding);
                self.present_change_plan(plan);
            }
            LspEvent::Symbols { context, symbols } => {
                if !self.finish_lsp_reply(&context) {
                    trace::services::rejected("lsp", "symbol owner/revision changed");
                    return;
                }
                if symbols.is_empty() {
                    self.message = "no symbols in this document".into();
                    return;
                }
                use strop_picker::{Item, Payload};
                let items = symbols
                    .into_iter()
                    .map(|symbol| (short_kind(&symbol.kind), symbol))
                    .filter_map(|(badge, symbol)| {
                        let line = symbol.location.position.line.get() + 1;
                        let col = symbol.location.position.column.get() + 1;
                        let path = symbol.location.doc.path.clone();
                        let payload = match symbol.location.doc.filesystem {
                            strop_workspace::Filesystem::Local => Payload::Grep {
                                path,
                                line,
                                col,
                                match_len: 1,
                                line_text: "".into(),
                            },
                            strop_workspace::Filesystem::Remote(endpoint) => Payload::Remote {
                                endpoint,
                                path,
                                line,
                                col,
                            },
                            // No container LSP is wired (DC1a); drop with a
                            // trace rather than aliasing a local path.
                            strop_workspace::Filesystem::Container(_) => {
                                trace::services::rejected("lsp", "container symbol dropped");
                                return None;
                            }
                        };
                        // The kind moves into the chip; the row text is
                        // name, container path, line.
                        let text = if symbol.container.is_empty() {
                            format!("{}  · :{}", symbol.name, line)
                        } else {
                            format!("{}  {} · :{}", symbol.name, symbol.container, line)
                        };
                        Some(Item {
                            badge: Some(badge.into()),
                            text,
                            payload,
                        })
                    })
                    .collect();
                self.open_picker(strop_picker::Kind::Symbols);
                if let Some(glue) = self.picker.as_mut() {
                    glue.picker.append(items);
                }
                // Items landed after the initial (empty-catalog)
                // ranking: re-rank or the list renders empty.
                self.request_picker_ranking();
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
                        badge: None,
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
                // Same post-append re-rank as the symbols arm above:
                // the initial ranking ran over an empty catalog.
                self.request_picker_ranking();
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
                            .filter_map(|location| {
                                let line = location.position.line.get() + 1;
                                let col = location.position.column.get() + 1;
                                let text = format!("{}:{}:{}", location.doc.label(), line, col);
                                let payload = match location.doc.filesystem {
                                    Filesystem::Local => Payload::Grep {
                                        path: location.doc.path,
                                        line,
                                        col,
                                        match_len: 1,
                                        line_text: "".into(),
                                    },
                                    Filesystem::Remote(endpoint) => Payload::Remote {
                                        endpoint,
                                        path: location.doc.path,
                                        line,
                                        col,
                                    },
                                    // DC1a wires no container LSP, so a container
                                    // location cannot arrive; if one ever does, it
                                    // is dropped with a trace, never aliased to a
                                    // local path.
                                    Filesystem::Container(_) => {
                                        trace::services::rejected(
                                            "lsp",
                                            "location in a container namespace (unwired)",
                                        );
                                        return None;
                                    }
                                };
                                Some(Item {
                                    badge: None,
                                    text,
                                    payload,
                                })
                            })
                            .collect();
                        let mut glue = crate::editor::PickerGlue::diagnostics(
                            strop_picker::Picker::new(Kind::Locations, items, false),
                        );
                        glue.lsp_context = Some(context);
                        self.set_picker(glue);
                        self.message = format!("{count} {}", kind.label());
                    }
                }
            }
        }
    }
}

/// Compact chip text for a symbol kind (the picker's badge column).
fn short_kind(kind: &str) -> &'static str {
    match kind {
        "Function" => "fn",
        "Method" => "meth",
        "Constructor" => "new",
        "Struct" => "struct",
        "Class" => "class",
        "Interface" => "iface",
        "Enum" => "enum",
        "EnumMember" => "variant",
        "Constant" => "const",
        "Variable" => "var",
        "Field" => "field",
        "Property" => "prop",
        "Module" => "mod",
        "Namespace" => "ns",
        "Package" => "pkg",
        "TypeParameter" => "T",
        _ => "sym",
    }
}
