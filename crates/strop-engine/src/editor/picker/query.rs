//! Shared query evaluation, scoped source requests and manual query suggestions.
use super::*;
use std::sync::Arc;
use strop_picker::query::SearchQuery;

impl Editor {
    fn query_policy(&self) -> strop_picker::SelectionPolicy {
        strop_picker::SelectionPolicy {
            hidden: self.config.search_show_hidden,
            respect_ignore: self.config.search_respect_ignore,
        }
    }
    pub(super) fn picker_query_eval(&mut self, kind: Kind) -> Option<Arc<SearchQuery>> {
        let glue = self.picker.as_mut()?;
        let query = Arc::new(SearchQuery::parse(&glue.picker.input.text));
        glue.query = Some(query.clone());
        glue.query_highlights = query.highlights.clone();
        let hidden = query.hidden.unwrap_or(self.config.search_show_hidden);
        let ignored = query.ignored.unwrap_or(!self.config.search_respect_ignore);
        let expression_mode = if kind == Kind::Files && !query.exact_file_expression {
            "fuzzy"
        } else if matches!(
            query.content,
            Some(strop_picker::query::ContentExpr::Regex(_))
        ) {
            "regex"
        } else {
            "literal"
        };
        glue.query_summary = format!(
            "local · {expression_mode} · hidden {} ({}) · ignored {} ({})",
            if hidden { "on" } else { "off" },
            if query.hidden.is_some() {
                "query"
            } else {
                "session"
            },
            if ignored { "on" } else { "off" },
            if query.ignored.is_some() {
                "query"
            } else {
                "session"
            }
        );
        match query.state {
            strop_picker::query::QueryState::Ready => {}
            strop_picker::query::QueryState::Incomplete
            | strop_picker::query::QueryState::Invalid => {
                let diagnostic = query.diagnostics.first();
                glue.picker.warning = None;
                glue.picker.error = diagnostic
                    .map(|d| {
                        let mut text = d.message.clone();
                        if let Some(suggestion) = &d.suggestion {
                            text.push_str(&format!(" — {suggestion}"));
                        }
                        text
                    })
                    .or_else(|| Some("incomplete query".into()));
                return None;
            }
        }
        glue.picker.error = None;
        glue.picker.warning = None;
        // the rank needle is the free text only — qualifiers never rank
        let needle = match &query.content {
            Some(strop_picker::query::ContentExpr::Literal(text)) => text.clone(),
            Some(strop_picker::query::ContentExpr::Regex(pattern)) => pattern.clone(),
            None => String::new(),
        };
        glue.picker.rank_query = Some(needle);
        let case = query.case.unwrap_or(strop_picker::query::CaseMode::Smart);
        glue.picker.rank_mode = match (
            &query.content,
            kind == Kind::Files && query.exact_file_expression,
        ) {
            (Some(expression), true) => {
                strop_picker::rank::MatchMode::Exact(Arc::new(expression.clone()), case)
            }
            _ => strop_picker::rank::MatchMode::Fuzzy(case),
        };
        Some(query)
    }

    /// The files walk as an owned request. Registration precedes
    /// launch: the worker can only post onto its stream, and nothing
    /// reaches the model until the ticket is the active owner. Replay
    /// mode stops after registration (Main's service seam).
    fn launch_files_request(&mut self, parsed: Arc<SearchQuery>) {
        let Some(picker) = self.picker.as_ref().map(|glue| glue.id) else {
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
                cwd: self.cwd.clone(),
            },
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.active = Some(ticket.clone());
            glue.picker.streaming = true;
        }
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"picker","source":"files","id":picker.0.get(),
                "request":request.get(),"cwd":self.cwd.to_string_lossy(),
            })
        });
        match self
            .tape
            .request("picker-files", &serde_json::json!({"ticket":ticket}))
        {
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
        let (tx, rx) = channel();
        let worker = spawn_files(self.cwd.clone(), parsed, policy, tx);
        if let Some(glue) = self.picker.as_mut() {
            glue.worker = Some(PickerWorker::Files(worker));
        }
        self.attach_picker_stream(ticket, rx);
    }

    pub(super) fn picker_input_changed(&mut self) {
        let Some(kind) = self.picker.as_ref().map(|glue| glue.picker.kind) else {
            return;
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.suggestions = None;
            glue.accept_when_ranked = false;
            if kind == Kind::Search && glue.picker.field == strop_picker::Field::Replace {
                self.search_intent_changed();
                return;
            }
        }
        if kind == Kind::Files {
            let Some(plans) = self.picker_query_eval(kind) else {
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
            return;
        }
        if kind == Kind::Search {
            self.restart_search_query();
            return;
        }
        if let Some(glue) = self.picker.as_mut() {
            if kind == Kind::RemoteAddress {
                glue.picker.error = None;
                return;
            }
        }
        self.request_picker_ranking();
    }

    pub(super) fn restart_search_query(&mut self) {
        let plans = self.picker_query_eval(Kind::Search);
        let (query, picker, root, session) = {
            let Some(glue) = self.picker.as_mut() else {
                return;
            };
            glue.suggestions = None;
            glue.accept_when_ranked = false;
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
            let policy = (
                glue.query
                    .as_ref()
                    .and_then(|query| query.hidden)
                    .unwrap_or(self.config.search_show_hidden),
                !glue
                    .query
                    .as_ref()
                    .and_then(|query| query.ignored)
                    .unwrap_or(!self.config.search_respect_ignore),
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
                glue.picker.input.text.clone(),
                glue.id,
                context.scope.root.path.clone(),
                context.stamp.session,
            )
        };
        self.invalidate_search_review(session);
        let Some(plans) = plans else { return };
        if plans.content.is_none() {
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
                cwd: root.clone(),
            },
        };
        // registration precedes launch (replay stops here)
        if let Some(glue) = self.picker.as_mut() {
            glue.active = Some(ticket.clone());
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
            &serde_json::json!({"ticket":ticket,"query":query}),
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
        let (tx, rx) = channel();
        let snapshots = self
            .docs
            .iter()
            .filter_map(|(_, document)| {
                if !document.buf.dirty
                    || !matches!(
                        document.source,
                        crate::editor::document::DocumentSource::File
                    )
                {
                    return None;
                }
                Some(strop_picker::SourceSnapshot {
                    path: self.cwd.join(
                        document
                            .buf
                            .file_identity()
                            .or(document.buf.path.as_deref())?,
                    ),
                    text: document.buf.text().clone(),
                })
            })
            .collect();
        let worker = GrepWorker::spawn(plans, policy, &root, snapshots, tx);
        if let Some(glue) = self.picker.as_mut() {
            glue.worker = Some(PickerWorker::Grep(worker));
        }
        self.attach_picker_stream(ticket, rx);
    }
}

impl Editor {
    /// ctrl-space in a query field: manual suggestions from the query's
    /// own parse position (0051 R02). Search-role fields only — the
    /// replacement value and remote address keep literal semantics.
    pub(crate) fn open_suggestions(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        let query_role = glue.picker.kind == Kind::Files
            || (glue.picker.kind == Kind::Search
                && glue.picker.field == strop_picker::Field::Search);
        if !query_role || glue.picker.input_normal() {
            return;
        }
        let query = glue
            .query
            .get_or_insert_with(|| Arc::new(SearchQuery::parse(&glue.picker.input.text)));
        let items = strop_picker::query::suggest::suggest(query, glue.picker.input.cursor);
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
        let mut text = String::with_capacity(input.text.len() + suggestion.insert.len());
        text.push_str(&input.text[..suggestion.range.start]);
        text.push_str(&suggestion.insert);
        let caret = text.len();
        text.push_str(&input.text[suggestion.range.end..]);
        input.text = text;
        input.cursor = caret;
        self.picker_input_changed();
    }
}
