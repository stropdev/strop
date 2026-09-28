use super::acceptance::{AcceptanceRefusal, AcceptanceTarget};
use super::model::{CompletionDelivery, CompletionPayload, DeliveryScope};
use super::session::{Acceptance, Candidate, Selection};
use super::worker::{Preparation, PreparationOperation, WorkRefusal};
use crate::editor::Editor;
use strop_core::worker::CancelReason;

impl Editor {
    pub(super) fn completion_prepare_documentation(&mut self) {
        self.completion_refresh_language_owner();
        let Some(session) = &self.completion.session else {
            return;
        };
        let Some((_, Candidate::Language { entry, .. })) = session.selected() else {
            return;
        };
        let Selection::Chosen { key, generation } = &session.selection else {
            return;
        };
        let resolved = session.resolved_item(key).cloned();
        if entry.item.protocol().documentation.is_none()
            && entry.item.protocol().detail.is_none()
            && resolved.as_ref().is_none_or(|item| {
                item.protocol().documentation.is_none() && item.protocol().detail.is_none()
            })
        {
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let preparation = Preparation {
            query: session.query.clone(),
            candidate: key.clone(),
            selection: *generation,
            work: request,
            operation: PreparationOperation::Documentation {
                original: entry.item.clone(),
                resolved,
            },
        };
        if let Some(mailbox) = self.completion.worker.mailbox() {
            mailbox.prepare(request);
        }
        if let Some(session) = &mut self.completion.session {
            session.preparation = Some(request);
        }
        let args = (
            preparation.query.request,
            preparation.scope(),
            preparation.candidate.clone(),
        );
        let result = self.tape.call("completion.documentation", &args, || {
            self.completion
                .worker
                .native()
                .ok_or(WorkRefusal::Closed)?
                .prepare(preparation)
        });
        match result {
            Ok(Ok(())) => {}
            Ok(Err(WorkRefusal::Busy)) => {
                if let Some(session) = &mut self.completion.session {
                    session.preparation = None;
                }
            }
            Ok(Err(error)) => {
                if let Some(session) = &mut self.completion.session {
                    session.preparation = None;
                }
                self.message = error.to_string();
            }
            Err(error) => {
                if let Some(session) = &mut self.completion.session {
                    session.preparation = None;
                }
                self.message = format!("completion documentation diverged: {error}");
            }
        }
    }

    pub(super) fn completion_accept_selected(&mut self) {
        self.completion_refresh_language_owner();
        let Some(session) = &self.completion.session else {
            return;
        };
        if !session
            .query
            .context
            .still_owns(self, self.completion.settings_generation)
        {
            self.completion_close(CancelReason::Superseded);
            return;
        }
        if matches!(session.acceptance, Acceptance::Preparing { .. }) {
            return;
        }
        let Some((_, candidate)) = session.selected() else {
            self.message = "choose a completion before accepting".into();
            return;
        };
        let Selection::Chosen { key, generation } = &session.selection else {
            return;
        };
        let recorded_chars = self.recording_insert.as_ref().map_or(0, |text| {
            text.chars()
                .rev()
                .take(strop_lsp::completion::MAX_FILTER_BYTES)
                .count()
        });
        let operation = match candidate {
            Candidate::Word(word) => PreparationOperation::Word {
                text: word.text.clone(),
                recorded_chars,
            },
            Candidate::Language { entry, suggestions } => {
                if let Some(refusal) = super::acceptance::unsupported_item(&entry.item) {
                    self.message = refusal.to_string();
                    return;
                }
                let resolved = session.resolved_item(key).cloned();
                if suggestions.resolve_required && resolved.is_none() {
                    let key = key.clone();
                    if let Some(session) = &mut self.completion.session {
                        session.acceptance = Acceptance::Resolving { candidate: key };
                    }
                    self.completion_resolve_selected();
                    return;
                }
                let Some(primary) = session.query.context.primary() else {
                    return;
                };
                let Some(source) = self.docs.get(primary.source) else {
                    return;
                };
                PreparationOperation::Language {
                    source: source.buf.snapshot(),
                    original: entry.item.clone(),
                    resolved,
                    encoding: suggestions.origin.encoding,
                    resolve_required: suggestions.resolve_required,
                    recorded_chars,
                }
            }
        };
        let work = match self.worker_ids.allocate() {
            Ok(work) => work,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let preparation = Preparation {
            query: session.query.clone(),
            candidate: key.clone(),
            selection: *generation,
            work,
            operation,
        };
        let candidate = key.clone();
        if let Some(mailbox) = self.completion.worker.mailbox() {
            mailbox.prepare(work);
        }
        if let Some(session) = &mut self.completion.session {
            session.preparation = Some(work);
            session.acceptance = Acceptance::Preparing {
                candidate: candidate.clone(),
                work,
            };
        }
        let result = self.tape.call(
            "completion.accept.prepare",
            &(preparation.query.request, preparation.scope(), &candidate),
            || {
                self.completion
                    .worker
                    .native()
                    .ok_or(WorkRefusal::Closed)?
                    .prepare(preparation)
            },
        );
        match result {
            Ok(Ok(())) => {}
            Ok(Err(WorkRefusal::Busy)) => {
                if let Some(session) = &mut self.completion.session {
                    session.acceptance = Acceptance::WaitingCapacity { candidate, work };
                }
            }
            Ok(Err(error)) => {
                if let Some(session) = &mut self.completion.session {
                    session.acceptance = Acceptance::Idle;
                    session.preparation = None;
                }
                self.message = error.to_string();
            }
            Err(error) => {
                if let Some(session) = &mut self.completion.session {
                    session.acceptance = Acceptance::Idle;
                    session.preparation = None;
                }
                self.message = format!("completion acceptance diverged: {error}");
            }
        }
    }

    pub(super) fn completion_apply_prepared(&mut self, delivery: &CompletionDelivery) {
        if !self.completion_owns_delivery(delivery) {
            return;
        }
        let CompletionPayload::Acceptance {
            candidate,
            prepared,
        } = &delivery.payload
        else {
            return;
        };
        let DeliveryScope::Preparation { work, .. } = delivery.scope else {
            return;
        };
        let intended = self.completion.session.as_ref().is_some_and(|session| {
            matches!(&session.acceptance, Acceptance::Preparing { candidate: owner, work: request }
                if owner == candidate && *request == work)
        });
        if !intended {
            return;
        }
        if let Err(refusal) = prepared {
            if let Some(session) = &mut self.completion.session {
                session.acceptance = Acceptance::Idle;
                session.preparation = None;
            }
            self.message = refusal.to_string();
            return;
        }
        let plan = self.tape.call(
            "completion.accept.take",
            &(delivery.query.request, delivery.scope),
            || {
                self.completion.worker.mailbox().and_then(|mailbox| {
                    mailbox.take_prepared(delivery.query.request, delivery.scope)
                })
            },
        );
        let plan = match plan {
            Ok(Some(plan)) => plan,
            Ok(None) => {
                self.completion_close(CancelReason::Superseded);
                self.message = "prepared completion no longer owns its operation".into();
                return;
            }
            Err(error) => {
                self.completion_close(CancelReason::Superseded);
                self.message = format!("completion acceptance transfer diverged: {error}");
                return;
            }
        };
        let context = &delivery.query.context;
        let target_owned = match plan.target {
            AcceptanceTarget::View { document, revision } => {
                document == context.scope.view && revision == context.view_revision
            }
            AcceptanceTarget::Source { document, revision } => context
                .primary()
                .is_some_and(|primary| primary.source == document && primary.revision == revision),
        };
        if !target_owned || plan.caret_bytes.len() != context.targets.len() {
            self.completion_close(CancelReason::Superseded);
            self.message = "prepared completion differs from its source/selection owner".into();
            return;
        }
        let collection = match plan.target {
            AcceptanceTarget::Source { document, .. } if document != context.scope.view => {
                let Some(collection) = self.collections.get(&context.scope.view) else {
                    self.completion_close(CancelReason::Superseded);
                    self.message = AcceptanceRefusal::MissingSource.to_string();
                    return;
                };
                let mut excerpts = Vec::with_capacity(context.targets.len());
                for target in &context.targets {
                    let index = collection
                        .excerpts
                        .partition_point(|excerpt| excerpt.view_start <= target.selection.head)
                        .checked_sub(1);
                    let Some(index) =
                        index.filter(|index| collection.excerpts[*index].source == target.source)
                    else {
                        self.completion_close(CancelReason::Superseded);
                        self.message = AcceptanceRefusal::MissingSource.to_string();
                        return;
                    };
                    excerpts.push(index);
                }
                Some(excerpts)
            }
            _ => None,
        };
        let document = plan.target.document();
        let revision = plan.target.revision();
        let outcome = self.apply(
            document,
            revision,
            crate::editor::transact::ChangeSet {
                edits: plan.edits,
                undo_open: true,
            },
        );
        if let Err(error) = outcome {
            if let Some(session) = &mut self.completion.session {
                session.acceptance = Acceptance::Idle;
                session.preparation = None;
            }
            self.message = format!("completion was not applied: {error}");
            return;
        }
        let mut carets = plan.caret_bytes;
        if let Some(indices) = collection {
            let Some(collection) = self.collections.get_mut(&context.scope.view) else {
                unreachable!("source publication retains its owning collection");
            };
            if !collection
                .pending_commit
                .iter()
                .any(|(source, _)| *source == document)
            {
                collection.pending_commit.push((document, revision));
            }
            for (caret, index) in carets.iter_mut().zip(indices) {
                let excerpt = &collection.excerpts[index];
                debug_assert!(
                    excerpt.source == document && *caret >= excerpt.start && *caret <= excerpt.end,
                    "accepted source carets remain in their remapped excerpts"
                );
                *caret = excerpt.view_start + (*caret - excerpt.start);
            }
        }
        if let Some(primary) = carets.first().copied() {
            self.sels_mut().collapse_primary(primary);
            self.sels_mut().set_extras(carets.into_iter().skip(1));
        }
        if let Some(recording) = &mut self.recording_insert {
            for _ in 0..plan.recording.remove_chars {
                recording.pop();
            }
            recording.push_str(&plan.recording.append);
        }
        self.completion_close(CancelReason::Dismissed);
        self.message.clear();
    }
}
