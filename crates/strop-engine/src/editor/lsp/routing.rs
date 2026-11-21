//! Reply admission and presentation, guarded by request ownership.
use crate::editor::{trace, Editor};
use strop_lsp::{protocol::ResolvedDiag, LspEvent};
use strop_workspace::ResourceLocation;

impl Editor {
    pub(crate) fn handle_lsp_event(&mut self, event: LspEvent) {
        trace::services::lsp(&event);
        match event {
            LspEvent::Ready { server, name } => {
                if let Some(index) = self.lsp_servers.iter().position(|owner| owner.id == server) {
                    self.clear_lsp_failure(server);
                    self.lsp_servers[index].ready = true;
                    let capable =
                        self.tape
                            .observe_owned("lsp.document_symbol_capability", &server, || {
                                self.lsp_servers[index]
                                    .client
                                    .as_ref()
                                    .is_some_and(|c| c.caps().document_symbols())
                            });
                    if !matches!(capable, Ok(true)) {
                        self.analysis.symbols.incapable.insert(server);
                    }
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
                encoding,
                generation,
                symbols,
            } => {
                let owns = self
                    .picker
                    .as_mut()
                    .filter(|g| g.wsymbols_generation == generation)
                    .and_then(|g| g.wsources.get_mut(&server))
                    .is_some_and(|(enc, pending)| *enc == encoding && std::mem::take(pending));
                if owns {
                    self.merge_workspace_symbols(server, encoding, generation, symbols);
                }
            }
            LspEvent::WorkspaceSymbolsFailed {
                server,
                generation,
                reason,
            } => {
                let live = self
                    .picker
                    .as_mut()
                    .filter(|g| g.wsymbols_generation == generation)
                    .and_then(|g| g.wsources.get_mut(&server))
                    .is_some_and(|(_, pending)| std::mem::take(pending));
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

    pub(crate) fn retire_workspace_symbol_source(&mut self, server: strop_lsp::ServerId) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        if glue.wsources.remove(&server).is_none() {
            return;
        }
        if let Some(len) = glue.ws_syntax_len {
            glue.picker.truncate_results(len);
        }
        self.query_workspace_symbols();
        self.request_picker_ranking();
    }

    /// Ask every warm attached server for workspace symbols (0063
    /// §2): one generation bump per query change; only that
    /// generation's replies merge. Refusals are per-server skips —
    /// not-ready servers join on their `Ready` event.
    pub(crate) fn query_workspace_symbols(&mut self) {
        let (generation, query, scope) = {
            let Some(glue) = self.picker.as_mut() else {
                return;
            };
            if glue.picker.kind != strop_picker::Kind::WorkspaceSymbols {
                return;
            }
            glue.wsources.clear();
            glue.ws_pending.clear();
            glue.ws_pending_bytes = 0;
            let Some(generation) = self.analysis.symbols.workspace_generation.checked_add(1) else {
                glue.picker.error = Some("workspace-symbol request identities exhausted".into());
                return;
            };
            self.analysis.symbols.workspace_generation = generation;
            glue.wsymbols_generation = generation;
            // Only bare content text crosses the wire (0063 §4):
            // qualifiers and operators never reach `workspace/symbol`;
            // the reply is filtered locally against the full AST. A
            // not-Ready query asks nothing — the generation bump above
            // still retires in-flight replies.
            let Some(probe) = glue.query.as_ref().and_then(|query| query.wsymbols_probe()) else {
                return;
            };
            if let Some(len) = glue.ws_syntax_len {
                if glue.picker.items.len() - glue.picker.pinned_tail > len {
                    glue.picker.truncate_results(len);
                }
            }
            let Some(scope) = glue.file_root.clone() else {
                return;
            };
            (glue.wsymbols_generation, probe, scope)
        };
        for attachment in self.lsp_state.attach.attached.clone() {
            if attachment.target != scope.filesystem
                || !(attachment.root.starts_with(&scope.path)
                    || scope.path.starts_with(&attachment.root))
            {
                continue;
            }
            let args = serde_json::json!({
                "server": attachment.server,
                "generation": generation,
                "query": query,
            });
            let launch = match self.tape.request("lsp.wsymbols", &args) {
                Ok(launch) => launch,
                Err(error) => {
                    self.message = format!("workspace symbols diverged from trace: {error}");
                    return;
                }
            };
            let admitted = self.tape.observe_owned("lsp.wsymbols_admitted", &args, || {
                let client = self.lsp_live_client(attachment.server)?;
                if !launch {
                    return None;
                }
                Some(
                    client
                        .workspace_symbols(generation, &query)
                        .map(|()| client.caps().encoding()),
                )
            });
            match admitted {
                Ok(Some(Ok(encoding))) => {
                    if let Some(glue) = self.picker.as_mut() {
                        glue.wsources.insert(attachment.server, (encoding, true));
                    }
                }
                Ok(Some(Err(refusal))) => {
                    if let Some(glue) = self.picker.as_mut() {
                        glue.partial_coverage = true;
                        if refusal == strop_lsp::RequestRefusal::Unsupported {
                            glue.picker.warning = Some(format!(
                                "workspace symbols: {} server does not provide workspace symbols",
                                attachment.language
                            ));
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.message = error.to_string();
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
        encoding: strop_lsp::PositionEncoding,
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
                    && glue
                        .wsources
                        .get(&server)
                        .is_some_and(|(enc, _)| *enc == encoding)
            });
        if !picker_live {
            trace::services::rejected("lsp", "workspace-symbol reply superseded");
            return;
        }
        let Some(placement) = self
            .lsp_state
            .attach
            .attached
            .iter()
            .find(|a| a.server == server)
        else {
            return;
        };
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        // Defer an early reply until file eligibility and child ownership
        // arrive, and dedup in syntax-first order regardless of event order.
        if glue.active.is_some() {
            let bytes = symbols.iter().fold(
                symbols
                    .capacity()
                    .saturating_mul(size_of::<strop_lsp::ProtoSymbol>()),
                |n, s| {
                    n.saturating_add(s.name.capacity())
                        .saturating_add(s.container.capacity())
                        .saturating_add(s.location.doc.path.capacity())
                },
            );
            if bytes > (16 * 1024 * 1024usize).saturating_sub(glue.ws_pending_bytes) {
                glue.partial_coverage = true;
                glue.picker.warning =
                    Some("workspace semantic reply exceeds the delivery budget".into());
            } else {
                glue.ws_pending_bytes += bytes;
                glue.ws_pending.push((server, encoding, symbols));
            }
            return;
        }
        if glue.picker.error.is_some() || glue.ws_syntax_len.is_none() {
            return;
        }
        let Some(scope) = glue.file_root.as_ref() else {
            return;
        };
        let dirty: std::collections::HashSet<_> = self
            .docs
            .iter()
            .filter_map(|(_, doc)| {
                doc.buf
                    .is_dirty()
                    .then(|| doc.file_target(&self.cwd)?.resource_location())
                    .flatten()
            })
            .collect();
        let mask = self.tape.observe_owned("lsp.wsymbols_members", &(server, generation), || {
            symbols.iter().map(|symbol| {
                let target = &symbol.location.doc;
                let Ok(relative) = target.path.strip_prefix(&scope.path) else { return false };
                let scope_admits = if scope.local_path().is_some() {
                    glue.inventory.symbol_owner(relative).is_some_and(|child|
                        child.as_os_str().is_empty() || placement.root.strip_prefix(&scope.path).is_ok_and(|r| r.starts_with(child)))
                } else {
                    // No local index or local path probing for an SSH endpoint.
                    relative.components().all(|component| matches!(component, std::path::Component::Normal(name) if name != ".git"))
                        && !self.lsp_state.attach.attached.iter().any(|a|
                            a.target == target.filesystem && a.root != placement.root
                            && a.root.starts_with(&placement.root) && target.path.starts_with(&a.root))
                };
                scope_admits && placement.target == target.filesystem && scope.filesystem == target.filesystem
                    && target.path.starts_with(&placement.root)
                    && super::lsp_language(&target.path) == Some(placement.language.as_str())
                    && !dirty.contains(target)
                    && !self.lsp_state.bindings.values().any(|b|
                        b.path == target.path && b.target == target.filesystem && b.server != server)
            }).collect::<Vec<_>>()
        });
        let mask = match mask {
            Ok(mask) if mask.len() == symbols.len() => mask,
            _ => return,
        };
        use strop_picker::Payload;
        let mut known: std::collections::HashSet<(&ResourceLocation, usize, &str)> = glue
            .picker
            .items
            .iter()
            .filter_map(|item| {
                let Payload::Symbol { location, line, .. } = &item.payload else {
                    return None;
                };
                Some((location, line.get(), item.facts().text))
            })
            .collect();
        let root = &scope.path;
        let mut paths = RelativePaths::new(root);
        let items = symbols
            .iter()
            .zip(mask)
            .filter_map(|(symbol, admitted)| {
                if !admitted {
                    return None;
                }
                let position = symbol.location.position;
                let line = position.line.get() + 1;
                let path = &symbol.location.doc.path;
                if !known.insert((&symbol.location.doc, position.line.get(), &symbol.name)) {
                    return None;
                }
                let shown = path.strip_prefix(root).unwrap_or(path);
                Some(strop_picker::Item::symbol(strop_picker::SymbolRow {
                    name: &symbol.name,
                    container: None,
                    detail: format_args!("{} · :{line}", strop_picker::display_path(shown)),
                    kind: symbol.kind,
                    path: paths.of(path),
                    payload: Payload::Symbol {
                        location: symbol.location.doc.clone(),
                        line: position.line,
                        column: strop_picker::SymbolColumn::Server {
                            server,
                            encoding,
                            column: position.column,
                        },
                    },
                }))
            })
            .collect::<Vec<_>>();
        if let Some(glue) = self.picker.as_mut() {
            glue.picker.append(items);
        }
        self.request_picker_ranking();
    }

    pub(crate) fn finish_workspace_symbols(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        glue.ws_syntax_len = Some(glue.picker.items.len() - glue.picker.pinned_tail);
        let generation = glue.wsymbols_generation;
        glue.ws_pending_bytes = 0;
        for (server, encoding, symbols) in std::mem::take(&mut glue.ws_pending) {
            self.merge_workspace_symbols(server, encoding, generation, symbols);
        }
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

/// Document-symbol rows from one `documentSymbol` reply or outline
/// (0063 §3): the kind rides in the chip; the row text is name,
/// container, line. The wire carries no query: the ranking pass filters
/// these rows as the field changes, and they are built once per list.
pub(crate) fn document_symbol_items(
    symbols: &[strop_lsp::ProtoSymbol],
    root: &std::path::Path,
    server: Option<(strop_lsp::ServerId, strop_lsp::PositionEncoding)>,
) -> Vec<strop_picker::Item> {
    let mut paths = RelativePaths::new(root);
    symbols
        .iter()
        .map(|symbol| {
            let position = symbol.location.position;
            let column = match server {
                Some((server, encoding)) => strop_picker::SymbolColumn::Server {
                    server,
                    encoding,
                    column: position.column,
                },
                None => strop_picker::SymbolColumn::Byte(strop_core::id::ByteColumn::new(
                    position.column.get(),
                )),
            };
            strop_picker::Item::symbol(strop_picker::SymbolRow {
                name: &symbol.name,
                container: Some(&symbol.container),
                detail: format_args!("· :{}", position.line.get() + 1),
                kind: symbol.kind,
                path: paths.of(&symbol.location.doc.path),
                payload: strop_picker::Payload::Symbol {
                    location: symbol.location.doc.clone(),
                    line: position.line,
                    column,
                },
            })
        })
        .collect()
}
