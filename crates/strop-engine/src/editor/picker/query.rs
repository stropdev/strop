//! Shared query evaluation, scoped source requests and manual query suggestions.
use super::*;
use std::sync::Arc;
use strop_picker::query::SearchQuery;

impl Editor {
    pub(crate) fn source_worker(
        &mut self,
    ) -> Result<&mut strop_picker::SourceWorker, strop_core::worker::Failure> {
        let watching = self.notify.push_coverage();
        let cwd = self.cwd.clone();
        match &mut self.picker_source {
            Some(worker) => Ok(worker),
            slot @ None => {
                let worker = slot.insert(strop_picker::SourceWorker::new()?);
                // 0058 S7: a worker created after the subscription still
                // inherits the current push-coverage state.
                worker.set_watching(&cwd, watching);
                Ok(worker)
            }
        }
    }

    fn query_policy(&self) -> strop_picker::SelectionPolicy {
        modes::policy(&self.config, self.picker_ranking.modes)
    }

    /// Parse the field for its surface (0109): one parse per revision,
    /// against what the surface declares. A Ready query sets the ranked
    /// needle and the filters the ranking pass applies; any other state
    /// puts its located diagnostic on the card and leaves the list as it
    /// was (`None`).
    pub(super) fn picker_query_eval(&mut self, kind: Kind) -> Option<Arc<SearchQuery>> {
        let session = self.picker_ranking.modes;
        let glue = self.picker.as_mut()?;
        // The toggles re-read a content expression; the other surfaces'
        // fuzzy needles have no expression to re-read.
        let modes = if kind == Kind::Search {
            session
        } else {
            Default::default()
        };
        let query = Arc::new(SearchQuery::parse(
            glue.picker.input.text(),
            kind.schema(),
            modes,
        ));
        glue.query = Some(query.clone());
        glue.replace_target = (kind == Kind::Search)
            .then(|| {
                strop_picker::query::ContentPlan::compile(&query)
                    .ok()
                    .flatten()
            })
            .flatten()
            .and_then(|plan| match plan.replacement_target() {
                strop_picker::query::ReplacementTarget::Single(regex) => Some(regex),
                _ => None,
            });
        glue.query_highlights = query.highlights.clone();
        let root = glue
            .search
            .as_ref()
            .map(|context| &context.scope.root)
            .or(glue.file_root.as_ref());
        glue.status_row = modes::status_row(&modes::Scope {
            kind,
            elsewhere: root.filter(|root| {
                root.filesystem != strop_workspace::Filesystem::Local || root.path != self.cwd
            }),
            config: &self.config,
            modes: session,
            query: &query,
        });
        match query.state {
            strop_picker::query::QueryState::Ready => {}
            strop_picker::query::QueryState::Incomplete
            | strop_picker::query::QueryState::Invalid => {
                let diagnostic = query.diagnostics.first();
                glue.picker.warning = None;
                glue.picker.error = diagnostic
                    .map(|d| match &d.suggestion {
                        Some(suggestion) => format!("{} — {suggestion}", d.message),
                        None => d.message.clone(),
                    })
                    .or_else(|| Some("incomplete query".into()));
                return None;
            }
        }
        glue.picker.error = None;
        glue.picker.warning = None;
        // the rank needle is the free text only — filters never rank
        let needle = match &query.content {
            Some(strop_picker::query::ContentExpr::Literal(text)) => text.clone(),
            Some(strop_picker::query::ContentExpr::Regex(pattern)) => pattern.clone(),
            None => String::new(),
        };
        glue.picker.rank_query = Some(needle);
        let case = query.case_mode();
        // Search's hits are already exact; elsewhere quoted, text: and
        // regex: input matches exactly instead of fuzzily.
        let exact = query.exact_expression && kind != Kind::Search;
        glue.picker.rank_mode = match (&query.content, exact) {
            (Some(expression), true) => {
                strop_picker::rank::MatchMode::Exact(Arc::new(expression.clone()), case)
            }
            _ => strop_picker::rank::MatchMode::Fuzzy(case),
        };
        // Where the row filters run (0109 §3.2d): Search's workers apply
        // them per hit; the Files walk applies the flat ones, so the
        // ranking pass judges only an operator query's rows there; every
        // other list's rows are judged in the ranking pass.
        glue.picker.filter = match kind {
            Kind::Search => None,
            Kind::Files => query.boolean.is_some().then(|| query.clone()),
            _ => Some(query.clone()),
        };
        Some(query)
    }

    /// The files walk as an owned request. Registration precedes
    /// launch: the worker can only post onto its stream, and nothing
    /// reaches the model until the ticket is the active owner. Replay
    /// mode stops after registration (Main's service seam).
    fn launch_files_request(&mut self, parsed: Arc<SearchQuery>) {
        let Some((picker, root)) = self
            .picker
            .as_ref()
            .and_then(|glue| Some((glue.id, glue.file_root.clone()?)))
        else {
            return;
        };
        if root.filesystem != strop_workspace::Filesystem::Local || !root.path.is_absolute() {
            if let Some(glue) = self.picker.as_mut() {
                glue.picker.error = Some(
                    "Files requires an absolute local selected root; no filesystem fallback".into(),
                );
                glue.picker.streaming = false;
                glue.file_scope = None;
            }
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: PickerKey {
                picker,
                cwd: root.path.clone(),
            },
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.active = Some(ticket.clone());
            glue.partial_coverage = false;
            glue.picker.streaming = true;
        }
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"picker","source":"files","id":picker.0.get(),
                "request":request.get(),"cwd":root.path.to_string_lossy(),
                "scope":root.label(),
            })
        });
        match self.tape.request(
            "picker-files",
            &serde_json::json!({"ticket":ticket,"scope":root}),
        ) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_picker_event(PickerEvent {
                    ticket,
                    msg: PickerMsg::Finished(strop_core::worker::Outcome::failed(
                        strop_core::worker::FailureKind::Protocol,
                        error.to_string(),
                    )),
                });
                return;
            }
        }
        let policy = self.query_policy();
        let tx = self.picker_source_sink(ticket.clone());
        let worker = match self.source_worker() {
            Ok(source) => source.files(root.path, parsed, policy, tx),
            Err(failure) => {
                let _ = tx.send(PickerMsg::Finished(strop_core::worker::Outcome::Failed {
                    failure,
                    partial: None,
                }));
                return;
            }
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.worker = Some(worker);
        }
    }

    /// `space S` (0063 §2): one bounded source run per walk scope —
    /// the walk and the symbol index consume the parsed query; the row
    /// filters apply in the ranking pass. Replay registers like files.
    pub(super) fn launch_workspace_symbols_request(&mut self, parsed: Arc<SearchQuery>) {
        let Some((picker, scope)) = self
            .picker
            .as_ref()
            .and_then(|g| Some((g.id, g.file_root.clone()?)))
        else {
            return;
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: PickerKey {
                picker,
                cwd: scope.path.clone(),
            },
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.active = Some(ticket.clone());
            glue.partial_coverage = false;
            glue.picker.streaming = true;
        }
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"picker","source":"workspace-symbols","id":picker.0.get(),
                "request":request.get(),"cwd":self.cwd.to_string_lossy(),
            })
        });
        let mut buffers = std::collections::HashMap::new();
        let snapshots = self
            .docs
            .iter()
            .filter_map(|(id, doc)| {
                let location = doc.file_target(&self.cwd)?.resource_location()?;
                if location.filesystem != scope.filesystem
                    || !location.path.starts_with(&scope.path)
                {
                    return None;
                }
                buffers.insert(location.path.clone(), (id, doc.buf.revision()));
                doc.buf.is_dirty().then(|| strop_picker::SourceSnapshot {
                    path: location.path,
                    text: doc.buf.snapshot(),
                })
            })
            .collect();
        if let Some(glue) = self.picker.as_mut() {
            glue.ws_buffers = buffers;
            glue.ws_syntax_len = None;
            if scope.filesystem != strop_workspace::Filesystem::Local {
                glue.active = None;
                glue.picker.streaming = false;
                glue.partial_coverage = true;
                glue.picker.warning = Some(
                    "remote workspace symbols: semantic servers only; syntax coverage unavailable"
                        .into(),
                );
                glue.ws_syntax_len = Some(0);
                return;
            }
        }
        match self.tape.request(
            "picker-workspace-symbols",
            &serde_json::json!({"ticket":ticket}),
        ) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_picker_event(PickerEvent {
                    ticket,
                    msg: PickerMsg::Finished(strop_core::worker::Outcome::failed(
                        strop_core::worker::FailureKind::Protocol,
                        error.to_string(),
                    )),
                });
                return;
            }
        }
        let policy = self.query_policy();
        let tx = self.picker_source_sink(ticket.clone());
        let root = scope.path;
        let worker = match self.source_worker() {
            Ok(source) => source.workspace_symbols(root, parsed, policy, snapshots, tx),
            Err(failure) => {
                let _ = tx.send(PickerMsg::Finished(strop_core::worker::Outcome::Failed {
                    failure,
                    partial: None,
                }));
                return;
            }
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.worker = Some(worker);
        }
    }

    pub(super) fn picker_input_changed(&mut self) {
        let Some(kind) = self.picker.as_ref().map(|glue| glue.picker.kind) else {
            return;
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.suggestions = None;
            glue.deferred = None;
            if kind == Kind::Search && glue.picker.field == strop_picker::Field::Replace {
                self.search_intent_changed();
                return;
            }
        }
        match kind {
            Kind::Files => self.files_query_changed(),
            Kind::Search => self.restart_search_query(),
            Kind::WorkspaceSymbols => self.workspace_symbols_query_changed(),
            Kind::RemoteAddress => {
                if let Some(glue) = self.picker.as_mut() {
                    glue.picker.error = None;
                }
            }
            // A plain list: the field is literal text, ranked as typed.
            _ if kind.schema().is_plain() => self.request_picker_ranking(),
            // A kept list (symbols, jumps, buffers, diagnostics,
            // locations): the ranking pass filters its rows. A query
            // that is not Ready leaves the list as it was, its
            // diagnostic on the card.
            _ => {
                if self.picker_query_eval(kind).is_some() {
                    self.request_picker_ranking();
                }
            }
        }
    }

    fn files_query_changed(&mut self) {
        let Some(plans) = self.picker_query_eval(Kind::Files) else {
            if let Some(glue) = self.picker.as_mut() {
                glue.revoke(CancelReason::Superseded);
                glue.picker.streaming = false;
                glue.rank_pending = None;
                glue.file_scope = None;
            }
            return;
        };
        let same_scope = self
            .picker
            .as_ref()
            .and_then(|glue| glue.file_scope.as_ref())
            .is_some_and(|scope| scope.same_scope(&plans));
        if same_scope {
            self.request_picker_ranking();
        } else {
            if let Some(glue) = self.picker.as_mut() {
                glue.revoke(CancelReason::Superseded);
                glue.picker.clear_items();
                glue.file_scope = Some(plans.clone());
                glue.rank_pending = None;
                glue.ranked_query = None;
            }
            self.launch_files_request(plans);
        }
    }

    fn workspace_symbols_query_changed(&mut self) {
        let previous = self
            .picker
            .as_ref()
            .and_then(|glue| glue.query.as_ref().cloned());
        let Some(parsed) = self.picker_query_eval(Kind::WorkspaceSymbols) else {
            // The diagnostic is on the card (the shared parse path);
            // retire in-flight LSP replies — they answered a query the
            // input has left behind (0063 §4: no retired-query
            // publication).
            self.query_workspace_symbols();
            return;
        };
        // The syntax tier re-walks only when the walk's scope moved;
        // every other filter, and the needle, apply in the ranking pass.
        let relaunch = previous.is_none_or(|previous| !previous.same_scope(&parsed));
        if relaunch {
            if let Some(glue) = self.picker.as_mut() {
                glue.revoke(CancelReason::Superseded);
                glue.picker.clear_items();
                glue.rank_pending = None;
                glue.ranked_query = None;
            }
            self.launch_workspace_symbols_request(parsed);
        }
        // Warm servers re-ask per query change under a new
        // generation; only bare content text crosses the wire.
        self.query_workspace_symbols();
        self.request_picker_ranking();
    }

    pub(crate) fn invalidate_manifest_picker(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        if glue.picker.kind != Kind::WorkspaceSymbols
            || !glue.file_root.as_ref().is_some_and(|root| {
                root.filesystem == strop_workspace::Filesystem::Local && root.path == self.cwd
            })
        {
            return;
        }
        glue.revoke(CancelReason::Superseded);
        glue.query = None;
        glue.deferred = None;
        glue.rank_pending = None;
        glue.ranked_query = None;
        glue.project_statuses.clear();
        glue.picker.clear_items();
        glue.picker.streaming = true;
        // Retire already-issued semantic replies as well as syntax batches.
        glue.wsources.clear();
    }

    pub(crate) fn refresh_manifest_picker(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        match glue.picker.kind {
            Kind::WorkspaceSymbols
                if glue.file_root.as_ref().is_some_and(|root| {
                    root.filesystem == strop_workspace::Filesystem::Local && root.path == self.cwd
                }) =>
            {
                glue.query = None;
                glue.project_statuses.clear();
                self.workspace_symbols_query_changed();
            }
            Kind::Search
                if glue.search.as_ref().is_some_and(|s| {
                    s.scope.root.filesystem == strop_workspace::Filesystem::Local
                }) =>
            {
                self.restart_search_query()
            }
            _ => {}
        }
    }

    pub(crate) fn filesystem_picker_changed(&mut self) {
        match self.picker.as_ref().map(|glue| glue.picker.kind) {
            Some(Kind::Files) => {
                if let Some(glue) = self.picker.as_mut() {
                    glue.file_scope = None;
                }
                self.picker_input_changed();
            }
            Some(Kind::Search) => self.restart_search_query(),
            _ => {}
        }
    }

    pub(super) fn restart_search_query(&mut self) {
        let plans = self.picker_query_eval(Kind::Search);
        let (query, picker, root, session) = {
            let Some(glue) = self.picker.as_mut() else {
                return;
            };
            glue.suggestions = None;
            glue.deferred = None;
            glue.revoke(CancelReason::Superseded);
            if let Some(worker) = &glue.rank_worker {
                worker.cancel_pending();
            }
            glue.rank_pending = None;
            glue.rank_dirty = false;
            glue.ranked_query = None;
            glue.picker.streaming = false;
            let Some(context) = glue.search.as_mut() else {
                glue.picker.error = Some("Search has no captured workspace scope".into());
                return;
            };
            let session = modes::policy(&self.config, self.picker_ranking.modes);
            let query = glue.query.as_deref();
            let policy = (
                query
                    .and_then(|query| query.hidden)
                    .unwrap_or(session.hidden),
                !query
                    .and_then(|query| query.ignored)
                    .unwrap_or(!session.respect_ignore),
            );
            let refresh =
                std::mem::take(&mut context.refresh_requested) && context.policy == policy;
            context.policy = policy;
            context.begin_view_restore();
            if refresh {
                glue.picker.refresh_items();
            } else {
                glue.picker.clear_items();
                context.discard_view_restore();
            }
            let Some(next) = context.stamp.dataset.checked_add(1) else {
                glue.picker.error = Some("search dataset generation exhausted".into());
                return;
            };
            context.stamp.dataset = next;
            context.refreshing = true;
            (
                glue.picker.input.text().to_string(),
                glue.id,
                context.scope.root.clone(),
                context.stamp.session,
            )
        };
        self.invalidate_search_review(session);
        let Some(plans) = plans else { return };
        // A search needs something to look for: filters alone
        // (`kind:fn`, `path:src NOT x`) explain themselves instead of
        // listing every line (0109 inconsistency 7).
        if !plans.has_positive_content() {
            if let Some(glue) = self.picker.as_mut() {
                if !query.is_empty() {
                    glue.picker.error =
                        Some("content search needs a text or regex expression".into());
                }
                if let Some(context) = glue.search.as_mut() {
                    context.refreshing = false;
                }
            }
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                // no identity: settle as not streaming; the next
                // keystroke retries with a fresh allocation
                if let Some(glue) = self.picker.as_mut() {
                    glue.picker.streaming = false;
                }
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: PickerKey {
                picker,
                cwd: root.path.clone(),
            },
        };
        // registration precedes launch (replay stops here)
        if let Some(glue) = self.picker.as_mut() {
            glue.active = Some(ticket.clone());
            glue.partial_coverage = false;
            glue.picker.streaming = true;
        }
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"picker","source":"grep","id":picker.0.get(),
                "request":request.get(),"query":query,"streaming":true,
            })
        });
        match self.tape.request(
            "picker-grep",
            &serde_json::json!({"ticket":ticket,"query":query,"scope":root}),
        ) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_picker_event(PickerEvent {
                    ticket,
                    msg: PickerMsg::Finished(strop_core::worker::Outcome::failed(
                        strop_core::worker::FailureKind::Protocol,
                        error.to_string(),
                    )),
                });
                return;
            }
        }
        let policy = self.query_policy();
        let tx = self.picker_source_sink(ticket.clone());
        let snapshots = self
            .docs
            .iter()
            .filter_map(|(_, document)| {
                if !document.buf.is_dirty() {
                    return None;
                }
                let location = document.file_target(&self.cwd)?.resource_location()?;
                if location.filesystem != root.filesystem || !location.path.starts_with(&root.path)
                {
                    return None;
                }
                Some(strop_picker::SourceSnapshot {
                    path: location.path,
                    text: document.buf.text().clone(),
                })
            })
            .collect();
        let host: Option<strop_picker::SearchHost> = match &root.filesystem {
            strop_workspace::Filesystem::Remote(endpoint) => {
                let endpoint = endpoint.clone();
                let client = self.remote_client();
                Some(Box::new(move |token| {
                    client.host(&endpoint, token).map_err(|error| {
                        strop_core::worker::Failure::new(
                            strop_core::worker::FailureKind::Unavailable,
                            error.to_string(),
                        )
                    })
                }))
            }
            _ => None,
        };
        let worker = match self.source_worker() {
            Ok(source) => source.search(plans, policy, root, snapshots, host, tx),
            Err(failure) => {
                let _ = tx.send(PickerMsg::Finished(strop_core::worker::Outcome::Failed {
                    failure,
                    partial: None,
                }));
                return;
            }
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.worker = Some(worker);
        }
    }
}

impl Editor {
    /// ctrl-space in a query field: manual suggestions from the query's
    /// own parse position (0051 R02), offering what the surface declares
    /// (0109). Plain lists, the replacement value and the remote address
    /// keep literal semantics.
    pub(crate) fn open_suggestions(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        let schema = glue.picker.kind.schema();
        let query_role = !schema.is_plain() && glue.picker.field == strop_picker::Field::Search;
        if !query_role || glue.picker.input_normal() {
            return;
        }
        let query = glue.query.get_or_insert_with(|| {
            Arc::new(SearchQuery::parse(
                glue.picker.input.text(),
                schema,
                Default::default(),
            ))
        });
        let items =
            strop_picker::query::suggest::suggest(query, glue.picker.input.cursor(), schema);
        if !items.is_empty() {
            glue.suggestions = Some(SuggestionList { items, selected: 0 });
        }
    }

    /// Accept the selected suggestion: replace its exact token span,
    /// never text after the caret or the rest of the query (0051 §4).
    pub(crate) fn accept_suggestion(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        let Some(list) = glue.suggestions.take() else {
            return;
        };
        let Some(suggestion) = list.items.get(list.selected) else {
            return;
        };
        let input = &mut glue.picker.input;
        let mut text = String::with_capacity(input.text().len() + suggestion.insert.len());
        text.push_str(&input.text()[..suggestion.range.start]);
        text.push_str(&suggestion.insert);
        let caret = text.len();
        text.push_str(&input.text()[suggestion.range.end..]);
        input.set_text(text);
        input.set_cursor(caret);
        self.picker_input_changed();
    }
}
