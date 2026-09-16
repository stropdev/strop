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
                    // A server that warmed up while the workspace
                    // symbols picker is open joins it now (0063 §2).
                    self.query_workspace_symbols();
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
                // The reply is retained whole (0063 §3):
                // `documentSymbol` carries no query, so Boolean/`kind:`
                // narrowing re-filters these candidates locally as the
                // input changes. The fresh picker has no query yet —
                // the initial rows are the unfiltered tree.
                let items = document_symbol_items(&symbols, None, &self.cwd);
                self.open_picker(strop_picker::Kind::Symbols);
                if let Some(glue) = self.picker.as_mut() {
                    glue.symbols_candidates = symbols;
                    glue.picker.append(items);
                }
                // Items landed after the initial (empty-catalog)
                // ranking: re-rank or the list renders empty.
                self.request_picker_ranking();
            }

            LspEvent::WorkspaceSymbols {
                server: _,
                generation,
                symbols,
            } => self.merge_workspace_symbols(generation, symbols),
            LspEvent::WorkspaceSymbolsFailed {
                server: _,
                generation,
                reason,
            } => {
                let live = self
                    .picker
                    .as_ref()
                    .filter(|glue| glue.picker.kind == strop_picker::Kind::WorkspaceSymbols)
                    .is_some_and(|glue| glue.wsymbols_generation == generation);
                if live {
                    self.message = format!("lsp: {reason}");
                } else {
                    trace::services::rejected("lsp", "workspace-symbol reply superseded");
                }
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
                                        location: strop_workspace::ResourceLocation::local(
                                            location.doc.path,
                                        ),
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

    /// Ask every warm attached server for workspace symbols (0063
    /// §2): one generation bump per query change; only that
    /// generation's replies merge. Refusals are per-server skips —
    /// not-ready servers join on their `Ready` event.
    pub(crate) fn query_workspace_symbols(&mut self) {
        let (generation, query) = {
            let Some(glue) = self.picker.as_mut() else {
                return;
            };
            if glue.picker.kind != strop_picker::Kind::WorkspaceSymbols {
                return;
            }
            glue.wsymbols_generation += 1;
            // Only bare content text crosses the wire (0063 §4):
            // qualifiers and operators never reach `workspace/symbol`;
            // the reply is filtered locally against the full AST. A
            // not-Ready query asks nothing — the generation bump above
            // still retires in-flight replies.
            let Some(probe) = glue.query.as_ref().and_then(|query| query.wsymbols_probe()) else {
                return;
            };
            (glue.wsymbols_generation, probe)
        };
        for attachment in self.lsp_state.attach.attached.clone() {
            let args = serde_json::json!({
                "server": attachment.server,
                "generation": generation,
                "query": query,
            });
            match self.tape.request("lsp.wsymbols", &args) {
                Ok(true) => {
                    if let Some(client) = self.lsp_live_client(attachment.server) {
                        // Admission refusals (still initializing, no
                        // provider) are skips, not errors: the syntax
                        // tier already carries the surface.
                        let _ = client.workspace_symbols(generation, &query);
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    self.message = format!("workspace symbols diverged from trace: {error}");
                    return;
                }
            }
        }
    }

    /// Merge one server's workspace-symbol reply into the open picker
    /// (0063 §2): same row convention as the syntax tier, duplicates
    /// by (location, name) skipped, then a local re-rank.
    fn merge_workspace_symbols(&mut self, generation: u64, symbols: Vec<strop_lsp::ProtoSymbol>) {
        // RowsCurrent (0063 §6.6): only the live generation's replies
        // merge — the verified kernel's decision.
        let picker_live = self
            .picker
            .as_ref()
            .filter(|glue| glue.picker.kind == strop_picker::Kind::WorkspaceSymbols)
            .is_some_and(|glue| {
                strop_core::searchguard::generation_is_live(generation, glue.wsymbols_generation)
            });
        if !picker_live {
            trace::services::rejected("lsp", "workspace-symbol reply superseded");
            return;
        }
        use strop_picker::{Item, Payload};
        let mut known: std::collections::HashSet<(String, usize, String)> = self
            .picker
            .as_ref()
            .map(|glue| {
                glue.picker
                    .items
                    .iter()
                    .filter_map(|item| match &item.payload {
                        Payload::Grep { location, line, .. } => Some((
                            location.path.to_string_lossy().into_owned(),
                            *line,
                            item_symbol_name(item).to_owned(),
                        )),
                        Payload::Remote { path, line, .. } => Some((
                            path.to_string_lossy().into_owned(),
                            *line,
                            item_symbol_name(item).to_owned(),
                        )),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let cwd = self.cwd.clone();
        // Local AST admission (0063 §4): the wire carried only bare
        // content text, so returned candidates are filtered against
        // the full query before merging — content atoms against the
        // symbol name (or its qualified container form), `kind:`
        // against the server's own classification, path-family atoms
        // against the workspace-relative path. Undecidable evidence
        // admits (no catalog here, so `repo:` overfetches honestly).
        // Operator-free queries leave narrowing to the local ranker,
        // exactly as before the grammar existed. Only the incoming
        // candidates are tested — engine-appended per-project status
        // rows (Payload::ProjectStatus) are informational, never
        // symbol candidates, and never enter this path.
        let admission = self
            .picker
            .as_ref()
            .and_then(|glue| glue.query.as_deref())
            .filter(|query| {
                query.state == strop_picker::query::QueryState::Ready && query.boolean.is_some()
            })
            .and_then(|query| {
                strop_picker::query::ContentPlan::compile(query)
                    .ok()
                    .flatten()
            });
        let items = symbols
            .into_iter()
            .map(|symbol| (short_kind(&symbol.kind), symbol))
            .filter_map(|(badge, symbol)| {
                let line = symbol.location.position.line.get() + 1;
                let col = symbol.location.position.column.get() + 1;
                let path = symbol.location.doc.path.clone();
                if let Some(plan) = &admission {
                    let qualified = if symbol.container.is_empty() {
                        None
                    } else {
                        Some(format!("{}::{}", symbol.container, symbol.name))
                    };
                    let evidence = strop_picker::query::Evidence {
                        path: path.strip_prefix(&cwd).ok().and_then(|path| path.to_str()),
                        line: Some(line),
                        symbol: Some(strop_picker::query::SymbolEvidence {
                            kind: lsp_symbol_kind(&symbol.kind),
                            qualified: qualified.as_deref(),
                        }),
                        ..Default::default()
                    };
                    if !plan.admits(evidence, &symbol.name) {
                        return None;
                    }
                }
                if !known.insert((
                    path.to_string_lossy().into_owned(),
                    line,
                    symbol.name.clone(),
                )) {
                    return None;
                }
                let shown = path
                    .strip_prefix(&cwd)
                    .map(strop_picker::display_path)
                    .unwrap_or_else(|_| std::borrow::Cow::from(path.display().to_string()))
                    .into_owned();
                let payload = match symbol.location.doc.filesystem {
                    Filesystem::Local => Payload::Grep {
                        location: strop_workspace::ResourceLocation::local(path),
                        line,
                        col,
                        match_len: 1,
                        line_text: "".into(),
                    },
                    Filesystem::Remote(endpoint) => Payload::Remote {
                        endpoint,
                        path,
                        line,
                        col,
                    },
                    Filesystem::Container(_) => {
                        trace::services::rejected("lsp", "container symbol dropped");
                        return None;
                    }
                };
                Some(Item {
                    badge: Some(badge.into()),
                    text: format!("{}  {} · :{}", symbol.name, shown, line),
                    payload,
                })
            })
            .collect::<Vec<_>>();
        if let Some(glue) = self.picker.as_mut() {
            glue.picker.append(items);
        }
        self.request_picker_ranking();
    }

    /// Re-derive the document-symbols rows from the retained reply
    /// (0063 §3): `textDocument/documentSymbol` has no query parameter,
    /// so a moved Boolean AST re-filters the full tree locally — the
    /// same admission the workspace-symbols merge applies to server
    /// candidates. Operator-free queries never reach here: the static
    /// list stays and the local ranker narrows.
    pub(crate) fn rebuild_document_symbols(&mut self) {
        let admission = self
            .picker
            .as_ref()
            .filter(|glue| glue.picker.kind == strop_picker::Kind::Symbols)
            .and_then(|glue| glue.query.as_deref())
            .filter(|query| {
                query.state == strop_picker::query::QueryState::Ready && query.boolean.is_some()
            })
            .and_then(|query| {
                strop_picker::query::ContentPlan::compile(query)
                    .ok()
                    .flatten()
            });
        let cwd = self.cwd.clone();
        let Some(glue) = self
            .picker
            .as_mut()
            .filter(|glue| glue.picker.kind == strop_picker::Kind::Symbols)
        else {
            return;
        };
        let items = document_symbol_items(&glue.symbols_candidates, admission.as_ref(), &cwd);
        glue.picker.clear_items();
        glue.rank_pending = None;
        glue.ranked_query = None;
        glue.picker.append(items);
    }
}

/// Document-symbol rows from one `documentSymbol` reply (0063 §3): the
/// kind rides in the chip; the row text is name, container, line. The
/// wire carries no query, so a Ready Boolean AST filters candidates
/// locally instead — content atoms against the name (or its qualified
/// container form), `kind:` against the server's own classification,
/// path-family atoms against the workspace-relative path. Undecidable
/// evidence admits (three-valued), exactly like the workspace-symbols
/// merge; operator-free queries pass no plan and admit everything.
fn document_symbol_items(
    symbols: &[strop_lsp::ProtoSymbol],
    admission: Option<&strop_picker::query::ContentPlan>,
    cwd: &std::path::Path,
) -> Vec<strop_picker::Item> {
    use strop_picker::{Item, Payload};
    symbols
        .iter()
        .map(|symbol| (short_kind(&symbol.kind), symbol))
        .filter_map(|(badge, symbol)| {
            let line = symbol.location.position.line.get() + 1;
            let col = symbol.location.position.column.get() + 1;
            let path = symbol.location.doc.path.clone();
            if let Some(plan) = admission {
                let qualified = if symbol.container.is_empty() {
                    None
                } else {
                    Some(format!("{}::{}", symbol.container, symbol.name))
                };
                let evidence = strop_picker::query::Evidence {
                    path: path.strip_prefix(cwd).ok().and_then(|path| path.to_str()),
                    line: Some(line),
                    symbol: Some(strop_picker::query::SymbolEvidence {
                        kind: lsp_symbol_kind(&symbol.kind),
                        qualified: qualified.as_deref(),
                    }),
                    ..Default::default()
                };
                if !plan.admits(evidence, &symbol.name) {
                    return None;
                }
            }
            let payload = match &symbol.location.doc.filesystem {
                Filesystem::Local => Payload::Grep {
                    location: strop_workspace::ResourceLocation::local(path),
                    line,
                    col,
                    match_len: 1,
                    line_text: "".into(),
                },
                Filesystem::Remote(endpoint) => Payload::Remote {
                    endpoint: endpoint.clone(),
                    path,
                    line,
                    col,
                },
                // No container LSP is wired (DC1a); drop with a
                // trace rather than aliasing a local path.
                Filesystem::Container(_) => {
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
        .collect()
}

/// The name field of a symbol row ("name  container · :line").
fn item_symbol_name(item: &strop_picker::Item) -> &str {
    item.text
        .split_once("  ")
        .map(|(name, _)| name.trim())
        .unwrap_or(item.text.trim())
}

/// The LSP SymbolKind name → the canonical classification deciding
/// `kind:` atoms for server candidates (0063 §4). Names outside the
/// canonical vocabulary stay Unknown — admitting, never false.
fn lsp_symbol_kind(name: &str) -> Option<strop_syntax::symbols::SymbolKind> {
    use strop_syntax::symbols::SymbolKind::*;
    Some(match name {
        "Function" => Function,
        "Method" | "Constructor" => Method,
        "Class" => Class,
        "Struct" => Struct,
        "Enum" => Enum,
        "Interface" => Interface,
        "Module" | "Namespace" | "Package" => Module,
        "Constant" => Constant,
        _ => return None,
    })
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
