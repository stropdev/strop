use super::mailbox::CompletionMailbox;
use super::model::{
    CandidateKey, CompletionDelivery, CompletionPayload, CompletionQuery, DeliveryScope,
    LanguageSuggestions,
};
use super::session::{Candidate, ProviderState, Selection, WorkerLifecycle};
use crate::editor::Editor;
use std::sync::{Arc, Weak};
use strop_core::id::{ByteColumn, LineIndex};
use strop_core::worker::{Failure, FailureKind, Outcome};
use strop_lsp::completion::{
    CompletionCall, CompletionResult, CompletionSink, CompletionTransportEvent, PendingCompletion,
};
use strop_lsp::{RequestInput, RequestKind, RequestRefusal, RequestStamp};

#[derive(serde::Serialize, serde::Deserialize)]
struct PreparedMetadata {
    stamp: RequestStamp,
    resolve_required: bool,
}

impl Editor {
    pub(super) fn completion_request_language(&mut self) {
        let Some(session) = &self.completion.session else {
            return;
        };
        if !session.query.context.invocation.language() || session.request.is_some() {
            return;
        }
        let query = session.query.clone();
        let trigger = session.trigger;
        if !query
            .context
            .still_owns(self, self.completion.settings_generation)
        {
            return;
        }
        let Some(target) = query.context.primary() else {
            return;
        };
        let document = target.source;
        let Some(location) = self.lsp_document_location(document) else {
            if let Some(session) = &mut self.completion.session {
                session.language =
                    ProviderState::Unavailable("no language service for a scratch buffer");
            }
            return;
        };
        if !self.lsp_state.bindings.contains_key(&document) {
            // A completed trust/install/discovery refusal is not an invitation
            // to rediscover or spawn on every typed character. Explicit trust
            // and the existing attachment lifecycle own those retries.
            let refused = self
                .lsp_state
                .attach
                .refused
                .keys()
                .any(|key| key.target == location.filesystem && key.path == location.path);
            let requested = self
                .completion
                .source
                .as_ref()
                .is_some_and(|source| source.language_attachment_requested);
            if !refused && !requested && self.lsp_state.attach.enabled {
                if let Some(source) = &mut self.completion.source {
                    source.language_attachment_requested = true;
                }
                self.lsp_attach_document(document);
            }
            self.lsp_did_open_document(document);
        }
        let Some(binding) = self.lsp_state.bindings.get(&document) else {
            let pending = self
                .lsp_state
                .attach
                .pending
                .keys()
                .any(|key| key.target == location.filesystem && key.path == location.path);
            if let Some(session) = &mut self.completion.session {
                session.language = if pending {
                    ProviderState::Initializing
                } else {
                    ProviderState::Unavailable("no language service for this source")
                };
            }
            return;
        };
        let owner = match super::context::LanguageContext::capture(
            binding,
            self.completion
                .source
                .as_ref()
                .and_then(|source| source.language_context.as_ref()),
        ) {
            Ok(owner) => owner,
            Err(refusal) => {
                if let Some(session) = &mut self.completion.session {
                    session.language = ProviderState::Failed(Failure::new(
                        FailureKind::InvalidInput,
                        refusal.to_string(),
                    ));
                }
                return;
            }
        };
        if let Some(source) = &mut self.completion.source {
            source.language_context = Some(owner.clone());
        }
        let server = binding.server;
        if binding.revision != target.revision {
            self.lsp_sync_changed();
        }
        let Some(input) = self.completion_input(&query, RequestKind::Completion) else {
            return;
        };
        let prefix_start = match input
            .byte_col
            .get()
            .checked_sub(target.caret - target.start)
        {
            Some(column) => ByteColumn::new(column),
            None => {
                self.completion_language_refused(RequestRefusal::InvalidInput);
                return;
            }
        };
        let call = CompletionCall::Query {
            trigger,
            prefix_start,
        };
        let mut pending = None;
        let args = (
            query.request,
            document,
            target.revision,
            input.line,
            input.byte_col,
            trigger,
            prefix_start,
        );
        let prepared = self.tape.call("completion.prepare", &args, || {
            let client = self
                .lsp_live_client(server)
                .ok_or(RequestRefusal::NotReady)?;
            if let WorkerLifecycle::Running { wake, .. } = &self.completion.worker {
                client.wake_completion_when_available(wake);
            }
            let request = client.prepare_completion(input, call)?;
            let metadata = PreparedMetadata {
                stamp: request.stamp,
                resolve_required: client.caps().completion_resolve(),
            };
            pending = Some(request);
            Ok::<_, RequestRefusal>(metadata)
        });
        let metadata = match prepared {
            Ok(Ok(metadata)) => metadata,
            Ok(Err(refusal)) => {
                self.completion_language_refused(refusal);
                return;
            }
            Err(error) => {
                self.message = format!("completion preparation diverged: {error}");
                return;
            }
        };
        if let Some(session) = &mut self.completion.session {
            session.request = Some(metadata.stamp);
            session.language = ProviderState::Pending;
        }
        let mailbox = self.completion.worker.mailbox().map(Arc::downgrade);
        let launched = self.tape.call(
            "completion.launch",
            &(query.request, metadata.stamp),
            || {
                let pending = pending.ok_or(RequestRefusal::NotReady)?;
                let mailbox = mailbox.ok_or(RequestRefusal::NotReady)?;
                let sink = query_sink(
                    mailbox,
                    query.clone(),
                    &pending,
                    metadata.resolve_required,
                    owner,
                );
                self.lsp_live_client(server)
                    .ok_or(RequestRefusal::NotReady)?
                    .launch_completion(pending, &sink)
            },
        );
        match launched {
            Ok(Ok(())) => {}
            Ok(Err(refusal)) => self.completion_language_refused(refusal),
            Err(error) => {
                if let Some(session) = &mut self.completion.session {
                    session.request = None;
                }
                self.message = format!("completion launch diverged: {error}");
            }
        }
    }

    fn completion_input(&self, query: &CompletionQuery, kind: RequestKind) -> Option<RequestInput> {
        let target = query.context.primary()?;
        let source = self.docs.get(target.source)?;
        let binding = self.lsp_state.bindings.get(&target.source)?;
        let line = source.buf.line_of(target.caret);
        Some(RequestInput {
            document: target.source,
            revision: target.revision,
            path: binding.path.clone(),
            line: LineIndex::new(line),
            byte_col: ByteColumn::new(target.caret - source.buf.line_start(line)),
            line_text: strop_lsp::FrozenLine::from_slice(
                source
                    .buf
                    .text()
                    .byte_slice(source.buf.line_start(line)..source.buf.line_end(line)),
            ),
            kind,
            rename_to: None,
            rename_files: None,
        })
    }

    fn completion_language_refused(&mut self, refusal: RequestRefusal) {
        if let Some(session) = &mut self.completion.session {
            session.request = None;
            session.language = match refusal {
                RequestRefusal::Overloaded | RequestRefusal::StaleRevision => {
                    ProviderState::WaitingCapacity
                }
                RequestRefusal::NotReady => ProviderState::Initializing,
                RequestRefusal::Unsupported => {
                    ProviderState::Unavailable("server does not provide completion")
                }
                RequestRefusal::NotOpen => {
                    ProviderState::Unavailable("source is not open on its language server")
                }
                RequestRefusal::Oversized => {
                    ProviderState::Unavailable("completion request exceeds the retained-data bound")
                }
                RequestRefusal::InvalidInput => ProviderState::Failed(Failure::new(
                    FailureKind::InvalidInput,
                    "invalid completion source position",
                )),
                RequestRefusal::IdentityExhausted => ProviderState::Failed(Failure::new(
                    FailureKind::IdentityExhausted,
                    "completion request identities exhausted",
                )),
            };
        }
    }

    pub(super) fn completion_resolve_selected(&mut self) {
        self.completion_refresh_language_owner();
        let Some(session) = &self.completion.session else {
            return;
        };
        if session.resolve.request().is_some() {
            return;
        }
        let Some((_, Candidate::Language { entry, suggestions })) = session.selected() else {
            return;
        };
        if !suggestions.resolve_required {
            return;
        }
        let Selection::Chosen { key, generation } = &session.selection else {
            return;
        };
        let query = session.query.clone();
        let candidate = key.clone();
        let scope = DeliveryScope::Selection(*generation);
        let server = suggestions.origin.stamp.server;
        let item = entry.item.clone();
        let Some(input) = self.completion_input(&query, RequestKind::CompletionResolve) else {
            return;
        };
        let mut pending = None;
        let prepared = self.tape.call(
            "completion.resolve.prepare",
            &(query.request, scope, &candidate),
            || {
                let client = self
                    .lsp_live_client(server)
                    .ok_or(RequestRefusal::NotReady)?;
                if let WorkerLifecycle::Running { wake, .. } = &self.completion.worker {
                    client.wake_completion_when_available(wake);
                }
                let request = client.prepare_completion(input, CompletionCall::Resolve { item })?;
                let stamp = request.stamp;
                pending = Some(request);
                Ok::<_, RequestRefusal>(stamp)
            },
        );
        let stamp = match prepared {
            Ok(Ok(stamp)) => stamp,
            Ok(Err(refusal)) => {
                self.completion_resolve_refused(refusal);
                return;
            }
            Err(error) => {
                self.message = format!("completion resolve diverged: {error}");
                return;
            }
        };
        if let Some(session) = &mut self.completion.session {
            session.resolve = super::session::ResolveState::Pending(stamp);
        }
        let mailbox = self.completion.worker.mailbox().map(Arc::downgrade);
        let launched = self.tape.call(
            "completion.resolve.launch",
            &(query.request, stamp, &candidate),
            || {
                let pending = pending.ok_or(RequestRefusal::NotReady)?;
                let sink = resolve_sink(
                    mailbox.ok_or(RequestRefusal::NotReady)?,
                    query.clone(),
                    scope,
                    candidate.clone(),
                );
                self.lsp_live_client(server)
                    .ok_or(RequestRefusal::NotReady)?
                    .launch_completion(pending, &sink)
            },
        );
        match launched {
            Ok(Ok(())) => {}
            Ok(Err(refusal)) => self.completion_resolve_refused(refusal),
            Err(error) => {
                if let Some(session) = &mut self.completion.session {
                    session.resolve = super::session::ResolveState::Idle;
                    session.acceptance = super::session::Acceptance::Idle;
                }
                self.message = format!("completion resolve launch diverged: {error}");
            }
        }
    }

    fn completion_resolve_refused(&mut self, refusal: RequestRefusal) {
        if let Some(session) = &mut self.completion.session {
            session.resolve = if matches!(
                refusal,
                RequestRefusal::Overloaded
                    | RequestRefusal::StaleRevision
                    | RequestRefusal::NotReady
            ) {
                super::session::ResolveState::WaitingCapacity
            } else {
                super::session::ResolveState::Failed(refusal)
            };
            if matches!(session.resolve, super::session::ResolveState::Failed(_)) {
                session.acceptance = super::session::Acceptance::Idle;
            }
        }
        self.message = match refusal {
            RequestRefusal::Overloaded => "completion resolve is waiting for server capacity",
            RequestRefusal::StaleRevision | RequestRefusal::NotReady => {
                "completion resolve is waiting for source synchronization"
            }
            RequestRefusal::Unsupported => "server no longer supports completion resolve",
            RequestRefusal::NotOpen => "completion resolve source is no longer open",
            RequestRefusal::Oversized => "completion resolve exceeds the retained-data bound",
            RequestRefusal::InvalidInput => "invalid completion resolve context",
            RequestRefusal::IdentityExhausted => "completion resolve identities exhausted",
        }
        .into();
    }

    pub(crate) fn completion_language_changed(&mut self) {
        self.completion_refresh_language_owner();
        self.completion_retry_capacity();
    }

    pub(super) fn completion_refresh_language_owner(&mut self) {
        let Some(session) = &self.completion.session else {
            return;
        };
        let owned = if let Some(language) = session.language() {
            language
                .owner
                .owns(self, &session.query.context, language.origin.stamp)
        } else {
            session.request.is_none_or(|stamp| {
                self.completion
                    .source
                    .as_ref()
                    .and_then(|source| source.language_context.as_ref())
                    .is_some_and(|owner| owner.owns(self, &session.query.context, stamp))
            })
        };
        if owned {
            return;
        }
        let selection = match &session.selection {
            Selection::Chosen {
                key: CandidateKey::Word(word),
                ..
            } => Some(CandidateKey::Word(word.clone())),
            _ => None,
        };
        let mut intent = self
            .completion
            .intent
            .take()
            .unwrap_or(super::session::RequestIntent {
                invocation: session.query.context.invocation,
                direction: None,
                trigger: strop_lsp::completion::CompletionTrigger::Invoked,
                cached_language: None,
                selection,
            });
        intent.cached_language = None;
        if matches!(intent.selection, Some(CandidateKey::Language { .. })) {
            intent.selection = None;
        }
        // Revoke the old query so its native snapshots can retire immediately.
        // The persistent source index survives; a chosen word keeps its identity.
        self.completion_close(strop_core::worker::CancelReason::OwnerClosed);
        self.completion.intent = Some(intent);
    }
}

fn query_sink(
    mailbox: Weak<CompletionMailbox>,
    query: Arc<CompletionQuery>,
    pending: &PendingCompletion,
    resolve_required: bool,
    owner: Arc<super::context::LanguageContext>,
) -> CompletionSink {
    let line = pending.input.line_text.clone();
    let caret = pending.input.byte_col.get();
    Arc::new(move |event| {
        if let CompletionTransportEvent::Reply(reply) = &event {
            trace_terminal(reply);
        }
        let Some(mailbox) = mailbox.upgrade() else {
            return;
        };
        match event {
            CompletionTransportEvent::Reply(reply) => {
                let outcome = match reply.outcome {
                    Outcome::Success(CompletionResult::List(items)) => {
                        let ranges = query.context.primary().and_then(|target| {
                            let start = caret.checked_sub(target.caret - target.start)?;
                            let end = caret.checked_add(target.end - target.caret)?;
                            let text = line.as_slice();
                            Some((
                                text.get_byte_slice(start..caret)?.to_string(),
                                text.get_byte_slice(caret..end)?.to_string(),
                            ))
                        });
                        match ranges {
                            Some((prefix, suffix)) => Outcome::Success(LanguageSuggestions {
                                origin: reply.context,
                                owner: owner.clone(),
                                items,
                                resolve_required,
                                prefix: Arc::from(prefix),
                                suffix: Arc::from(suffix),
                            }),
                            None => Outcome::failed(
                                FailureKind::Protocol,
                                "completion prefix no longer fits its captured line",
                            ),
                        }
                    }
                    Outcome::Success(CompletionResult::Resolved(_)) => Outcome::failed(
                        FailureKind::Protocol,
                        "completion query received a resolve result",
                    ),
                    Outcome::Failed { failure, .. } => Outcome::Failed {
                        failure,
                        partial: None,
                    },
                    Outcome::Cancelled(reason) => Outcome::Cancelled(reason),
                };
                mailbox.publish(CompletionDelivery {
                    query: query.clone(),
                    scope: DeliveryScope::Query,
                    payload: CompletionPayload::Language(outcome),
                });
            }
            CompletionTransportEvent::Capacity { .. } | CompletionTransportEvent::Closed { .. } => {
                mailbox.capacity()
            }
        }
    })
}

fn resolve_sink(
    mailbox: Weak<CompletionMailbox>,
    query: Arc<CompletionQuery>,
    scope: DeliveryScope,
    candidate: CandidateKey,
) -> CompletionSink {
    Arc::new(move |event| {
        if let CompletionTransportEvent::Reply(reply) = &event {
            trace_terminal(reply);
        }
        let Some(mailbox) = mailbox.upgrade() else {
            return;
        };
        match event {
            CompletionTransportEvent::Reply(reply) => {
                mailbox.publish(CompletionDelivery {
                    query: query.clone(),
                    scope,
                    payload: CompletionPayload::Resolved {
                        candidate: candidate.clone(),
                        reply,
                    },
                });
            }
            CompletionTransportEvent::Capacity { .. } | CompletionTransportEvent::Closed { .. } => {
                mailbox.capacity()
            }
        }
    })
}

fn trace_terminal(reply: &strop_lsp::completion::CompletionReply) {
    strop_trace::record_with(strop_trace::EventKind::LspMessage, || {
        let outcome = match &reply.outcome {
            Outcome::Success(_) => "success",
            Outcome::Failed { .. } => "failed",
            Outcome::Cancelled(_) => "cancelled",
        };
        serde_json::json!({"service":"completion", "request":reply.context.stamp.request,
            "server":reply.context.stamp.server, "kind":reply.context.kind.label(), "outcome":outcome})
    });
}
