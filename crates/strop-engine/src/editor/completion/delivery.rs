use super::mailbox::CompletionDrain;
use super::model::{CompletionDelivery, CompletionPayload, DeliveryScope};
use super::session::{Acceptance, ProviderState, RetirementStage, Selection, WorkerLifecycle};
use crate::editor::Editor;
use std::sync::Arc;
use strop_core::worker::{CancelReason, Outcome};

impl Editor {
    pub(super) fn completion_owns_delivery(&self, delivery: &CompletionDelivery) -> bool {
        let Some(session) = &self.completion.session else {
            return false;
        };
        if !(Arc::ptr_eq(&session.query, &delivery.query) || session.query == delivery.query)
            || !delivery
                .query
                .context
                .still_owns(self, self.completion.settings_generation)
        {
            return false;
        }
        let candidate = match &delivery.payload {
            CompletionPayload::Resolved { candidate, .. }
            | CompletionPayload::Documentation { candidate, .. }
            | CompletionPayload::Acceptance { candidate, .. } => Some(candidate),
            _ => None,
        };
        if matches!(candidate, Some(super::model::CandidateKey::Language { .. }))
            && session.language().is_none_or(|language| {
                !language
                    .owner
                    .owns(self, &session.query.context, language.origin.stamp)
            })
        {
            return false;
        }
        match (delivery.scope, &session.selection) {
            (DeliveryScope::Query, _) => true,
            (DeliveryScope::Selection(owner), Selection::Chosen { generation, .. }) => {
                owner == *generation
            }
            (
                DeliveryScope::Preparation { selection, work },
                Selection::Chosen { generation, .. },
            ) => selection == *generation && session.preparation == Some(work),
            _ => false,
        }
    }

    pub(crate) fn handle_completion(&mut self) {
        let drained = self.tape.observe_owned("completion.drain", &(), || {
            self.completion
                .worker
                .mailbox()
                .map_or_else(CompletionDrain::default, |mailbox| mailbox.take())
        });
        let CompletionDrain {
            deliveries,
            capacity_changed,
            stopped,
            usage,
        } = match drained {
            Ok(drained) => drained,
            Err(error) => {
                self.message = format!("completion delivery diverged: {error}");
                return;
            }
        };
        self.completion.metrics = usage;
        if let Some(outcome) = stopped {
            if matches!(self.completion.worker, WorkerLifecycle::Running { .. }) {
                self.completion_close(CancelReason::OwnerClosed);
                self.completion.source = None;
                let lifecycle =
                    std::mem::replace(&mut self.completion.worker, WorkerLifecycle::Idle);
                if let WorkerLifecycle::Running {
                    native, mailbox, ..
                } = lifecycle
                {
                    self.completion.worker = WorkerLifecycle::Stopping {
                        native,
                        mailbox,
                        stage: RetirementStage::ThreadExit,
                    };
                }
            } else if let WorkerLifecycle::Stopping { stage, .. } = &mut self.completion.worker {
                *stage = RetirementStage::ThreadExit;
            }
            if let Outcome::Failed { failure, .. } = outcome {
                self.message = failure.message;
            }
        }
        for delivery in deliveries {
            if !self.completion_owns_delivery(&delivery) {
                continue;
            }
            match &delivery.payload {
                CompletionPayload::Words { .. } if delivery.scope == DeliveryScope::Query => {
                    if let Some(session) = &mut self.completion.session {
                        session.words = ProviderState::Settled(delivery);
                    }
                }
                CompletionPayload::Language(outcome) if delivery.scope == DeliveryScope::Query => {
                    if let Outcome::Success(suggestions) = outcome {
                        let valid = self.completion.session.as_ref().is_some_and(|session| {
                            let request = session.request == Some(suggestions.origin.stamp)
                                || session.cached_language.as_ref().is_some_and(|cached|
                                    matches!(&cached.payload, CompletionPayload::Language(Outcome::Success(previous))
                                        if previous.origin == suggestions.origin && previous.owner == suggestions.owner && previous.reusable()));
                            request && suggestions.owner.owns(self, &session.query.context, suggestions.origin.stamp)
                        });
                        if !valid {
                            if let Some(session) = &mut self.completion.session {
                                session.language =
                                    ProviderState::Unavailable("language context changed");
                            }
                            continue;
                        }
                    }
                    if let Some(session) = &mut self.completion.session {
                        session.request = None;
                        session.cached_language = None;
                        session.language = ProviderState::Settled(delivery);
                    }
                }
                CompletionPayload::Resolved { candidate, reply } => {
                    let valid = self.completion.session.as_ref().is_some_and(|session| {
                        session.resolve.request() == Some(reply.context.stamp)
                            && session
                                .selected()
                                .is_some_and(|(_, selected)| selected.matches(candidate))
                    });
                    if !valid {
                        continue;
                    }
                    let accepting = self.completion.session.as_ref().is_some_and(|session|
                        matches!(&session.acceptance, Acceptance::Resolving { candidate: wanted } if wanted == candidate));
                    if let Outcome::Failed { failure, .. } = &reply.outcome {
                        if accepting {
                            self.message = failure.message.clone();
                        }
                    }
                    let resolved = matches!(
                        reply.outcome,
                        Outcome::Success(strop_lsp::completion::CompletionResult::Resolved(_))
                    );
                    if let Some(session) = &mut self.completion.session {
                        session.resolve = super::session::ResolveState::Complete(delivery);
                        if accepting {
                            session.acceptance = Acceptance::Idle;
                        }
                    }
                    if accepting && resolved {
                        self.completion_accept_selected();
                    } else if resolved {
                        self.completion_prepare_documentation();
                    }
                }
                CompletionPayload::Documentation { candidate, .. } => {
                    if let Some(session) = self.completion.session.as_mut().filter(|session| {
                        session
                            .selected()
                            .is_some_and(|(_, selected)| selected.matches(candidate))
                    }) {
                        session.documentation = Some(delivery);
                    }
                }
                CompletionPayload::Acceptance { .. } => self.completion_apply_prepared(&delivery),
                _ => {}
            }
        }
        // Apply both provider snapshots before choosing an awaited first row;
        // a later provider cannot move an already deliberate selection.
        self.completion_reconcile_selection();
        self.completion.released();
        self.completion_collect_retired();
        if capacity_changed {
            self.completion_retry_capacity();
        }
        self.completion_after_action();
    }

    pub(super) fn completion_retry_capacity(&mut self) {
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
        if self
            .completion
            .source
            .as_ref()
            .is_some_and(|source| source.snapshot_waiting)
        {
            if let Err(refusal) = self.completion_offer_source() {
                if refusal != super::worker::WorkRefusal::Busy {
                    self.message = refusal.to_string();
                }
            }
        }
        if self
            .completion
            .session
            .as_ref()
            .is_some_and(|session| session.query_waiting)
        {
            self.completion_query_worker();
        }
        if self.completion.session.as_ref().is_some_and(|session| {
            matches!(
                session.language,
                ProviderState::WaitingCapacity | ProviderState::Initializing
            )
        }) {
            self.completion_request_language();
        }
        if self.completion.session.as_ref().is_some_and(|session| {
            matches!(
                session.resolve,
                super::session::ResolveState::WaitingCapacity
            )
        }) {
            self.completion_resolve_selected();
        }
        if self
            .completion
            .session
            .as_ref()
            .is_some_and(|session| matches!(session.acceptance, Acceptance::WaitingCapacity { .. }))
        {
            self.completion_accept_selected();
        }
    }
}
