//! Reply admission and presentation, guarded by request ownership.
use crate::editor::{trace, Editor};
use strop_lsp::{protocol::ResolvedDiag, LspEvent};
use strop_workspace::{Filesystem, ResourceLocation};

impl Editor {
    pub(crate) fn handle_lsp_event(&mut self, event: LspEvent) {
        trace::services::lsp(&event);
        match event {
            LspEvent::Ready { server, name } => {
                if let Some(index) = self.lsp_servers.iter().position(|owner| owner.id == server) {
                    self.clear_lsp_failure(server);
                    self.lsp_servers[index].ready = true;
                    // Success must not erase a configuration warning
                    // (0033 §2): readiness is reported alongside it.
                    self.message = match self.layer_warning() {
                        Some(warning) => format!("lsp: {name} ready — {warning}"),
                        None => format!("lsp: {name} ready"),
                    };
                    // A server that warmed up while the workspace
                    // symbols picker is open joins it now (0063 §2).
                    self.query_workspace_symbols();
                } else {
                    trace::services::rejected("lsp", "readiness for an unowned server");
                }
            }
            LspEvent::Failed { server, name, hint } => {
                // Captured before teardown erases the bindings (0092 §2.4).
                let restart = self.lsp_restart_plan(server);
                if self.record_lsp_failure(server, &name, &hint) {
                    self.lsp_failed(server);
                    self.message = match restart {
                        Some(plan) => self.lsp_restart(plan, &name),
                        None => format!("lsp: {name} failed — :explain for details"),
                    };
                } else {
                    trace::services::rejected("lsp", "failure for an unowned server");
                }
            }
            LspEvent::NotificationDropped { server, name, .. } => {
                if self.lsp_servers.iter().any(|s| s.id == server) {
                    self.message = format!(
                        "lsp: {name}: a notification too large to decode (over 512 MiB) was ignored"
                    );
                } else {
                    trace::services::rejected("lsp", "dropped notice for an unowned server");
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
                self.record_diagnostics(
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
                if context.kind == strop_lsp::RequestKind::DocumentSymbols {
                    // an error or "no symbols": the build's owner words it
                    crate::editor::symbols::server_reply(self, &context, None, Some(text));
                    return;
                }
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
                if context.kind == strop_lsp::RequestKind::WillRenameFiles {
                    // Rename-preparation replies resume the staged
                    // filesystem apply; an empty set is a valid "this
                    // server contributes nothing" answer, never an error.
                    self.will_rename_edits(context, edits);
                    return;
                }
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
                crate::editor::symbols::server_reply(self, &context, Some(symbols), None);
            }

            LspEvent::WorkspaceSymbols {
                server,
                generation,
                symbols,
            } => self.merge_workspace_symbols(server, generation, symbols),
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
                self.deliver_code_actions(actions, context.encoding);
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
                sources,
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
                        use strop_picker::Kind;
                        let items = super::locations::rows(self, items, sources);
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
    /// by (location, name) skipped, then a local re-rank. The rows go in
    /// whole: the ranking pass applies the query's filters to them and
    /// to the syntax tier's rows alike (0109 §3.2d).
    fn merge_workspace_symbols(
        &mut self,
        server: strop_lsp::ServerId,
        generation: u64,
        symbols: Vec<strop_lsp::ProtoSymbol>,
    ) {
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
        use strop_picker::Payload;
        let mut known: std::collections::HashSet<(Filesystem, std::path::PathBuf, usize, String)> =
            self.picker
                .as_ref()
                .map(|glue| {
                    glue.picker
                        .items
                        .iter()
                        .filter_map(|item| {
                            let name = item.facts().text.to_owned();
                            match &item.payload {
                                Payload::Grep { location, line, .. } => Some((
                                    location.filesystem.clone(),
                                    location.path.clone(),
                                    *line,
                                    name,
                                )),
                                Payload::Remote {
                                    endpoint,
                                    path,
                                    line,
                                    ..
                                } => Some((
                                    Filesystem::Remote(endpoint.clone()),
                                    path.clone(),
                                    *line,
                                    name,
                                )),
                                _ => None,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
        let root = self
            .lsp_state
            .attach
            .attached
            .iter()
            .find(|attachment| attachment.server == server)
            .map(|attachment| attachment.root.clone())
            .unwrap_or_else(|| self.cwd.clone());
        let mut paths = RelativePaths::new(&root);
        // symbol rows show no source line: one shared empty text for all
        let no_text: std::sync::Arc<str> = "".into();
        let items = symbols
            .into_iter()
            .filter_map(|symbol| {
                let line = symbol.location.position.line.get() + 1;
                let path = symbol.location.doc.path.clone();
                if !known.insert((
                    symbol.location.doc.filesystem.clone(),
                    path.clone(),
                    line,
                    symbol.name.clone(),
                )) {
                    return None;
                }
                let shown = path
                    .strip_prefix(&root)
                    .map(strop_picker::display_path)
                    .unwrap_or_else(|_| std::borrow::Cow::from(path.display().to_string()))
                    .into_owned();
                Some(symbol_item(
                    &symbol,
                    None,
                    format_args!("{shown} · :{line}"),
                    paths.of(&path),
                    &no_text,
                ))
            })
            .collect::<Vec<_>>();
        if let Some(glue) = self.picker.as_mut() {
            glue.picker.append(items);
        }
        self.request_picker_ranking();
    }
}

/// Workspace-relative path facts, one allocation per file: the rows of
/// one file share it.
struct RelativePaths<'a> {
    root: &'a std::path::Path,
    last: Option<(std::path::PathBuf, std::sync::Arc<str>)>,
}

impl<'a> RelativePaths<'a> {
    fn new(root: &'a std::path::Path) -> Self {
        Self { root, last: None }
    }

    fn of(&mut self, path: &std::path::Path) -> Option<std::sync::Arc<str>> {
        if let Some((last, shared)) = &self.last {
            if last == path {
                return Some(shared.clone());
            }
        }
        let relative: std::sync::Arc<str> = path.strip_prefix(self.root).ok()?.to_str()?.into();
        self.last = Some((path.to_path_buf(), relative.clone()));
        Some(relative)
    }
}

/// One server or outline symbol as a list row (0109): the kind is the
/// chip and a fact the ranking pass filters on.
fn symbol_item(
    symbol: &strop_lsp::ProtoSymbol,
    container: Option<&str>,
    detail: std::fmt::Arguments<'_>,
    path: Option<std::sync::Arc<str>>,
    no_text: &std::sync::Arc<str>,
) -> strop_picker::Item {
    use strop_picker::Payload;
    let line = symbol.location.position.line.get() + 1;
    let col = symbol.location.position.column.get() + 1;
    let location = symbol.location.doc.path.clone();
    let payload = match &symbol.location.doc.filesystem {
        Filesystem::Local => Payload::Grep {
            location: strop_workspace::ResourceLocation::local(location),
            line,
            col,
            match_len: 1,
            line_text: no_text.clone(),
        },
        Filesystem::Remote(endpoint) => Payload::Remote {
            endpoint: endpoint.clone(),
            path: location,
            line,
            col,
        },
    };
    strop_picker::Item::symbol(strop_picker::SymbolRow {
        name: &symbol.name,
        container,
        detail,
        kind: symbol.kind,
        path,
        payload,
    })
}

/// Document-symbol rows from one `documentSymbol` reply or outline
/// (0063 §3): the kind rides in the chip; the row text is name,
/// container, line. The wire carries no query: the ranking pass filters
/// these rows as the field changes, and they are built once per list.
pub(crate) fn document_symbol_items(
    symbols: &[strop_lsp::ProtoSymbol],
    root: &std::path::Path,
) -> Vec<strop_picker::Item> {
    let mut paths = RelativePaths::new(root);
    // symbol rows show no source line: one shared empty text for all
    let no_text: std::sync::Arc<str> = "".into();
    symbols
        .iter()
        .map(|symbol| {
            let line = symbol.location.position.line.get() + 1;
            let path = paths.of(&symbol.location.doc.path);
            symbol_item(
                symbol,
                Some(&symbol.container),
                format_args!("· :{line}"),
                path,
                &no_text,
            )
        })
        .collect()
}
