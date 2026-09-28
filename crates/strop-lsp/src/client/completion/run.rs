use super::registry::{stopped_outcome, Control, Disposition, Reservation};
use super::CompletionClient;
use crate::client::outbound::Method;
use crate::completion::{
    CompletionCall, CompletionDecodeError, CompletionFilter, CompletionItem, CompletionResult,
    CompletionTrigger, PendingCompletion,
};
use crate::{RequestStamp, ServerColumn};
use async_lsp::lsp_types as lt;
use serde_json::Value;
use std::future::{poll_fn, Future};
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;
use strop_core::worker::{CancelReason, FailureKind, Outcome};
use tokio::sync::oneshot;
use tokio::time::Sleep;

const DEADLINE: Duration = Duration::from_secs(2);

enum QueryRequest {}
impl lt::request::Request for QueryRequest {
    type Params = lt::CompletionParams;
    type Result = Value;
    const METHOD: &'static str = "textDocument/completion";
}
enum ResolveRequest {}
impl lt::request::Request for ResolveRequest {
    type Params = Arc<CompletionItem>;
    type Result = Value;
    const METHOD: &'static str = "completionItem/resolve";
}

struct RequestOwner {
    client: Arc<CompletionClient>,
    stamp: Option<RequestStamp>,
}
impl RequestOwner {
    fn finish(mut self, outcome: Outcome<CompletionResult>) {
        if let Some(stamp) = self.stamp.take() {
            self.client.environment.queue.cancel_completion(stamp);
            self.client.environment.outbound.forget_request(stamp);
            self.client.registry.finish(stamp, outcome);
        }
    }
}
impl Drop for RequestOwner {
    fn drop(&mut self) {
        if let Some(stamp) = self.stamp.take() {
            self.client.environment.queue.cancel_completion(stamp);
            self.client.environment.outbound.forget_request(stamp);
            self.client.registry.finish(
                stamp,
                Outcome::failed(
                    FailureKind::Disconnected,
                    "completion task stopped with its language-service connection",
                ),
            );
        }
    }
}

pub(super) async fn request(
    client: Arc<CompletionClient>,
    pending: PendingCompletion,
    reservation: Reservation,
) {
    let owner = RequestOwner {
        client: client.clone(),
        stamp: Some(pending.stamp),
    };
    let control = reservation.control;
    let mut start = pin!(reservation.start);
    let mut deadline = pin!(tokio::time::sleep(DEADLINE));
    let ready = poll_fn(|context| {
        if control.poll(context) != Disposition::Active {
            return Poll::Ready(false);
        }
        if deadline.as_mut().poll(context).is_ready() {
            control.expire();
            return Poll::Ready(false);
        }
        match start.as_mut().poll(context) {
            Poll::Ready(Ok(())) => Poll::Ready(true),
            Poll::Ready(Err(_)) => {
                control.cancel(CancelReason::OwnerClosed);
                Poll::Ready(false)
            }
            Poll::Pending => Poll::Pending,
        }
    })
    .await;
    if !ready {
        owner.finish(stopped_outcome(control.disposition()).unwrap_or_else(|| {
            Outcome::failed(FailureKind::Disconnected, "completion queue closed")
        }));
        return;
    }
    let env = &client.environment;
    if !env.caps.supports(pending.input.kind) {
        owner.finish(Outcome::failed(
            FailureKind::Unavailable,
            "completion method is not supported by this language server",
        ));
        return;
    }
    let encoding = env.caps.encoding();
    let outcome = match &pending.call {
        CompletionCall::Query {
            trigger,
            prefix_start,
        } => {
            let position =
                match super::super::api::request_position(&env.workspace, encoding, &pending.input)
                {
                    Ok(position) => position,
                    Err(reason) => {
                        owner.finish(Outcome::failed(FailureKind::InvalidInput, reason));
                        return;
                    }
                };
            let server_caret = ServerColumn::new(position.position.character as usize);
            let (trigger_kind, trigger_character) = match trigger {
                CompletionTrigger::Invoked => (lt::CompletionTriggerKind::INVOKED, None),
                CompletionTrigger::Character(character) => (
                    lt::CompletionTriggerKind::TRIGGER_CHARACTER,
                    Some(character.to_string()),
                ),
                CompletionTrigger::Incomplete => (
                    lt::CompletionTriggerKind::TRIGGER_FOR_INCOMPLETE_COMPLETIONS,
                    None,
                ),
            };
            let params = lt::CompletionParams {
                text_document_position: position,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: Some(lt::CompletionContext {
                    trigger_kind,
                    trigger_character,
                }),
            };
            match exchange::<QueryRequest>(
                &client,
                &pending,
                &control,
                deadline.as_mut(),
                Method::Completion,
                params,
            )
            .await
            {
                Ok(value) => {
                    if let Some(stopped) = stopped_outcome(control.disposition()) {
                        stopped
                    } else {
                        let decoded = CompletionFilter::new(
                            &pending.input,
                            *prefix_start,
                            server_caret,
                            encoding,
                        )
                        .and_then(|filter| {
                            crate::completion::decode_list(value, &filter, || {
                                if tokio::time::Instant::now() >= deadline.deadline() {
                                    control.expire();
                                }
                                control.disposition() != Disposition::Active
                            })
                        });
                        match decoded {
                            Ok(items) => Outcome::Success(CompletionResult::List(items)),
                            Err(CompletionDecodeError::Cancelled) => {
                                stopped_outcome(control.disposition())
                                    .unwrap_or(Outcome::Cancelled(CancelReason::Superseded))
                            }
                            Err(error) => Outcome::failed(FailureKind::Protocol, error.to_string()),
                        }
                    }
                }
                Err(outcome) => outcome,
            }
        }
        CompletionCall::Resolve { item } => {
            match exchange::<ResolveRequest>(
                &client,
                &pending,
                &control,
                deadline.as_mut(),
                Method::Resolve,
                item.clone(),
            )
            .await
            {
                Ok(value) => {
                    if let Some(stopped) = stopped_outcome(control.disposition()) {
                        stopped
                    } else {
                        match crate::completion::decode_resolved(value, item) {
                            Ok(item) => Outcome::Success(CompletionResult::Resolved(item)),
                            Err(error) => Outcome::failed(FailureKind::Protocol, error.to_string()),
                        }
                    }
                }
                Err(outcome) => outcome,
            }
        }
    };
    owner.finish(outcome);
}

enum Transmission {
    Unsent,
    AwaitingIdentity(oneshot::Receiver<async_lsp::RequestId>),
    Identified(async_lsp::RequestId),
    CancellationSent,
}

async fn exchange<R: lt::request::Request<Result = Value>>(
    client: &CompletionClient,
    pending: &PendingCompletion,
    control: &Control,
    mut deadline: Pin<&mut Sleep>,
    method: Method,
    params: R::Params,
) -> Result<Value, Outcome<CompletionResult>> {
    let env = &client.environment;
    let mut response = pin!(env.socket.request::<R>(params));
    let mut transmission = Transmission::Unsent;
    poll_fn(|context| {
        let mut disposition = control.poll(context);
        if disposition == Disposition::Finished {
            return Poll::Ready(Err(Outcome::failed(
                FailureKind::Disconnected,
                "completion connection closed",
            )));
        }
        if disposition == Disposition::Active && deadline.as_mut().poll(context).is_ready() {
            control.expire();
            disposition = control.disposition();
        }
        let polled = if matches!(transmission, Transmission::Unsent) {
            if let Some(stopped) = stopped_outcome(disposition) {
                return Poll::Ready(Err(stopped));
            }
            // The wire worker keeps this barrier until submitted() or finish().
            // Later changes can be admitted without waiting for serialization,
            // but cannot enter async-lsp ahead of this request.
            let owns = env
                .sync
                .lock()
                .owns(env.server, &pending.stamp, &pending.input.path);
            if !owns {
                control.cancel(CancelReason::Superseded);
                return Poll::Ready(Err(Outcome::Cancelled(CancelReason::Superseded)));
            }
            let identity = match env.outbound.submitted(pending.stamp, method) {
                Ok(identity) => identity,
                Err(error) => {
                    return Poll::Ready(Err(Outcome::failed(
                        FailureKind::Unavailable,
                        error.to_string(),
                    )))
                }
            };
            transmission = Transmission::AwaitingIdentity(identity);
            let polled = response.as_mut().poll(context);
            control.submitted();
            polled
        } else {
            response.as_mut().poll(context)
        };
        if let Poll::Ready(result) = polled {
            return Poll::Ready(
                result
                    .map_err(|error| Outcome::failed(FailureKind::Protocol, error_message(error))),
            );
        }
        if let Transmission::AwaitingIdentity(identity) = &mut transmission {
            match Pin::new(identity).poll(context) {
                Poll::Ready(Ok(identity)) => transmission = Transmission::Identified(identity),
                Poll::Ready(Err(_)) => {
                    return Poll::Ready(Err(Outcome::failed(
                        FailureKind::Disconnected,
                        "completion frame identity closed with the transport",
                    )))
                }
                Poll::Pending => {}
            }
        }
        if disposition != Disposition::Active {
            client.registry.stop_logical(pending.stamp, disposition);
            if matches!(transmission, Transmission::Identified(_)) {
                let Transmission::Identified(identity) =
                    std::mem::replace(&mut transmission, Transmission::CancellationSent)
                else {
                    unreachable!("wire identity checked above")
                };
                let id = match identity {
                    async_lsp::RequestId::Number(number) => lt::NumberOrString::Number(number),
                    async_lsp::RequestId::String(text) => lt::NumberOrString::String(text),
                };
                if let Err(error) = env
                    .socket
                    .notify::<lt::notification::Cancel>(lt::CancelParams { id })
                {
                    return Poll::Ready(Err(Outcome::failed(
                        FailureKind::Disconnected,
                        error_message(error),
                    )));
                }
            }
        }
        // Advisory cancellation does not release this future or its physical
        // slot. An ignoring server can make completion busy, not unbounded.
        Poll::Pending
    })
    .await
}

fn error_message(error: impl std::fmt::Display) -> String {
    let mut message = format!("completion failed: {error}");
    if message.len() > 4096 {
        let mut end = 4096;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push_str(" … (message truncated)");
    }
    message
}
