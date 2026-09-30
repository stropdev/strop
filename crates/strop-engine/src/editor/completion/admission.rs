use super::context::{self, Invocation};
use super::model::CompletionQuery;
use super::session::{CompletionState, ProviderState, Session, WorkerLifecycle};
use super::worker::WorkRefusal;
use crate::editor::Editor;
use std::sync::Arc;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{CancelReason, Failure, FailureKind};

impl Editor {
    /// Called at the shared action boundary, never during paint. An intent
    /// captures the post-edit source, not the stale pre-keystroke document.
    pub(crate) fn completion_after_action(&mut self) {
        self.completion_collect_retired();
        self.completion_refresh_language_owner();
        if self.completion.source.as_ref().is_some_and(|source| {
            !source.binding.matches(self)
                || self.docs.is_empty()
                || self
                    .source_edit_position(self.current(), self.head())
                    .is_none_or(|(document, _)| document != source.binding.document)
        }) {
            if let Some(source) = self.completion.source.take() {
                match self.tape.request(
                    "completion.forget",
                    &(source.generation, source.binding.document),
                ) {
                    Ok(true) => {
                        if let Some(worker) = self.completion.worker.native() {
                            worker.forget_source();
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        self.message = format!("completion source retirement diverged: {error}")
                    }
                }
            }
        }
        let Some(intent) = self.completion.intent.take() else {
            if self.completion.session.as_ref().is_some_and(|session| {
                // An applied live preview makes the context stale by design
                // (0059 §6.1); the revert re-captures it, so it is not a
                // supersession.
                session.preview.is_none()
                    && !session
                        .query
                        .context
                        .still_owns(self, self.completion.settings_generation)
            }) {
                self.completion_close(CancelReason::Superseded);
            }
            return;
        };
        if matches!(self.completion.worker, WorkerLifecycle::Stopping { .. }) {
            self.completion.intent = Some(intent);
            return;
        }
        if matches!(self.completion.worker, WorkerLifecycle::Failed(_)) {
            if !intent.invocation.manual() {
                return;
            }
            self.completion.worker = WorkerLifecycle::Idle;
        }
        let context = match context::capture(
            self,
            intent.invocation,
            self.completion.settings_generation,
            self.completion
                .session
                .as_ref()
                .map(|session| &session.query.context.scope),
        ) {
            Ok(context) => context,
            Err(refusal) => {
                self.completion_close(CancelReason::Superseded);
                if intent.invocation.manual() {
                    self.message = refusal.to_string();
                }
                return;
            }
        };
        // No unsolicited empty/one-character word popup. A server-declared
        // trigger character can request semantics at an empty keyword prefix.
        if intent.invocation == Invocation::Automatic
            && !matches!(
                intent.trigger,
                strop_lsp::completion::CompletionTrigger::Character(_)
            )
            && context.primary().is_none_or(|target| {
                self.docs.get(target.source).is_none_or(|source| {
                    source
                        .buf
                        .text()
                        .byte_slice(target.start..target.caret)
                        .chars()
                        .take(2)
                        .count()
                        < 2
                })
            })
        {
            self.completion_close(CancelReason::Superseded);
            return;
        }
        let cache_valid = self.completion.session.as_ref().is_some_and(|session| {
            let (Some(before), Some(after)) = (session.query.context.primary(), context.primary())
            else {
                return false;
            };
            self.completion.prefix_growth == Some((session.query.request, after.revision))
                && Arc::ptr_eq(&session.query.context.scope, &context.scope)
                && before.source == after.source
                && before.start == after.start
                && after.caret > before.caret
                && before.end - before.caret == after.end - after.caret
        });
        let cache = intent.cached_language.filter(|_| cache_valid);
        let selection = intent
            .selection
            .filter(|key| matches!(key, super::model::CandidateKey::Word(_)) || cache.is_some());
        self.completion_close(CancelReason::Superseded);
        if let Err(failure) = self.completion_worker() {
            self.message = failure.message;
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let query = Arc::new(CompletionQuery {
            request,
            context: Arc::new(context),
        });
        let mut session = Session::new(query, intent.direction);
        session.cached_language = cache;
        session.trigger = intent.trigger;
        if let Some(key) = selection {
            let generation = match self.worker_ids.allocate() {
                Ok(generation) => generation,
                Err(error) => {
                    self.message = error.message;
                    return;
                }
            };
            session.selection = super::session::Selection::Chosen { key, generation };
        }
        if let Some(mailbox) = self.completion.worker.mailbox() {
            mailbox.query(Some(request));
            if let super::session::Selection::Chosen { generation, .. } = session.selection {
                mailbox.select(Some(generation));
            }
        }
        self.completion.session = Some(session);
        match self.completion_offer_source() {
            Ok(()) | Err(WorkRefusal::Busy) => self.completion_query_worker(),
            Err(refusal) => {
                if let Some(session) = &mut self.completion.session {
                    session.words = ProviderState::Failed(Failure::new(
                        FailureKind::Unavailable,
                        refusal.to_string(),
                    ));
                    session.cached_language = None;
                }
            }
        }
        if self.completion.session.as_ref().is_some_and(|session| {
            session.query.context.invocation.language() && session.cached_language.is_none()
        }) {
            self.completion_request_language();
        }
    }

    pub(super) fn completion_query_worker(&mut self) {
        let Some(session) = &mut self.completion.session else {
            return;
        };
        if !session.query.context.invocation.words() && session.cached_language.is_none() {
            return;
        }
        let admitted = self.tape.call(
            "completion.query",
            &(session.query.request, session.cached_language.is_some()),
            || {
                self.completion
                    .worker
                    .native()
                    .ok_or(WorkRefusal::Closed)?
                    .query(&session.query, session.cached_language.as_ref())
            },
        );
        match admitted {
            Ok(Ok(())) => {
                session.query_waiting = false;
            }
            Ok(Err(WorkRefusal::Busy)) => {
                session.query_waiting = true;
            }
            Ok(Err(refusal)) => {
                session.query_waiting = false;
                session.words = ProviderState::Failed(Failure::new(
                    FailureKind::Unavailable,
                    refusal.to_string(),
                ));
            }
            Err(error) => {
                session.query_waiting = false;
                session.words =
                    ProviderState::Failed(Failure::new(FailureKind::Protocol, error.to_string()));
            }
        }
    }
}

impl CompletionState {
    /// Consume the same pre-edit publication journal as syntax analysis. A
    /// frozen replacement is admitted under the worker's retirement budget;
    /// refused snapshots remain explicitly pending in the UI model.
    pub(crate) fn edits(
        &mut self,
        tape: &strop_trace::replay::Tape,
        document: DocumentId,
        revision: BufferRevision,
        text: &ropey::Rope,
        changes: &[strop_core::Change],
    ) -> Result<(), String> {
        let Some(source) = self
            .source
            .as_mut()
            .filter(|source| source.binding.document == document)
        else {
            return Ok(());
        };
        if let Some(session) = &self.session {
            if self
                .intent
                .as_ref()
                .is_some_and(|intent| intent.cached_language.is_some())
            {
                self.prefix_growth = match (session.query.context.primary(), changes) {
                    (Some(target), [change])
                        if session.query.context.targets.len() == 1
                            && target.source == document
                            && target.revision.get().checked_add(1) == Some(revision.get())
                            && change.revision == revision
                            && change.edit.start_byte == target.caret
                            && change.edit.old_end_byte == target.caret
                            && change.edit.new_end_byte > target.caret =>
                    {
                        Some((session.query.request, revision))
                    }
                    _ => None,
                };
            }
        }
        let args = (
            source.generation,
            document,
            revision,
            text.len_bytes(),
            changes.len(),
        );
        let admitted = tape
            .call("completion.edits", &args, || {
                self.worker
                    .native()
                    .ok_or(WorkRefusal::Closed)?
                    .edits(document, revision, text, changes)
            })
            .map_err(|error| error.to_string())?;
        source.revision = revision;
        source.snapshot_waiting = admitted == Err(WorkRefusal::Busy);
        match admitted {
            Ok(()) | Err(WorkRefusal::Busy) => {}
            Err(WorkRefusal::SourceTooLarge | WorkRefusal::WrongSource | WorkRefusal::Closed) => {
                self.source = None
            }
        }
        Ok(())
    }
}
