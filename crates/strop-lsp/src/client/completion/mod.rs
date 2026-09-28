//! Completion has an independent bounded request owner, but uses the same
//! synchronized document table, ordered wire and native worker-backed socket.
mod registry;
mod run;
use registry::AdmissionError;
pub(super) use registry::Registry;

use crate::completion::{
    CompletionCall, CompletionSink, CompletionUsage, PendingCompletion, MAX_COMPLETION_ITEM_BYTES,
    MAX_COMPLETION_ITEM_NODES, MAX_FILTER_BYTES,
};
use crate::{ReplyContext, RequestInput, RequestKind, RequestRefusal, RequestStamp, ServerId};
use parking_lot::Mutex;
use std::sync::Arc;
use strop_core::worker::CancelReason;

pub(super) struct Environment {
    pub server: ServerId,
    pub socket: async_lsp::ServerSocket,
    pub handle: tokio::runtime::Handle,
    pub caps: crate::ServerCaps,
    pub sync: Arc<Mutex<super::sync::SyncState>>,
    pub workspace: crate::target::Workspace,
    pub outbound: Arc<super::outbound::Outbound>,
    pub queue: super::queue::WireControl,
}

pub(super) struct CompletionClient {
    pub registry: Arc<Registry>,
    environment: Environment,
}
impl CompletionClient {
    pub fn new(registry: Arc<Registry>, environment: Environment) -> Self {
        Self {
            registry,
            environment,
        }
    }
    fn spawn(self: &Arc<Self>, pending: PendingCompletion, reservation: registry::Reservation) {
        let owner = self.clone();
        self.environment.handle.spawn(async move {
            run::request(owner, pending, reservation).await;
        });
    }
}

fn request_kind(call: &CompletionCall) -> RequestKind {
    match call {
        CompletionCall::Query { .. } => RequestKind::Completion,
        CompletionCall::Resolve { .. } => RequestKind::CompletionResolve,
    }
}

fn captured_bytes(input: &RequestInput, call: &CompletionCall) -> Option<usize> {
    let retained = input
        .line_text
        .as_slice()
        .len_bytes()
        .checked_add(input.path.as_os_str().len())?;
    match call {
        CompletionCall::Query { .. } => Some(retained),
        // Resolve items came from the bounded decoder. Charge their maximum
        // payload and Value-node storage without walking opaque data on input.
        CompletionCall::Resolve { .. } => retained
            .checked_add(MAX_COMPLETION_ITEM_BYTES)?
            .checked_add(MAX_COMPLETION_ITEM_NODES * 128)
            .and_then(|bytes| {
                bytes.checked_add(std::mem::size_of::<crate::completion::CompletionItem>())
            }),
    }
}

impl super::Client {
    pub fn prepare_completion(
        &self,
        mut input: RequestInput,
        call: CompletionCall,
    ) -> Result<PendingCompletion, RequestRefusal> {
        if self.quitting.load(std::sync::atomic::Ordering::Relaxed) || !self.sync.lock().ready {
            return Err(RequestRefusal::NotReady);
        }
        if input.rename_to.is_some() {
            return Err(RequestRefusal::InvalidInput);
        }
        input.kind = request_kind(&call);
        if let CompletionCall::Query { prefix_start, .. } = &call {
            let line = input.line_text.as_slice();
            let start = prefix_start.get();
            let caret = input.byte_col.get();
            let valid = start <= caret
                && caret - start <= MAX_FILTER_BYTES
                && line
                    .try_byte_to_char(start)
                    .ok()
                    .is_some_and(|at| line.char_to_byte(at) == start)
                && line
                    .try_byte_to_char(caret)
                    .ok()
                    .is_some_and(|at| line.char_to_byte(at) == caret);
            if !valid {
                return Err(RequestRefusal::InvalidInput);
            }
        }
        if captured_bytes(&input, &call).is_none_or(|bytes| bytes > registry::MAX_CAPTURE_BYTES) {
            return Err(RequestRefusal::Oversized);
        }
        let stamp = self.capture_request_stamp(&input)?;
        Ok(PendingCompletion { stamp, input, call })
    }

    /// Preparing captures authority; this step admits physical work. A refusal
    /// creates no task, wire frame or callback. The caller owns the refusal.
    pub fn launch_completion(
        &self,
        pending: PendingCompletion,
        observer: &CompletionSink,
    ) -> Result<(), RequestRefusal> {
        if self.quitting.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(RequestRefusal::NotReady);
        }
        if pending.input.kind != request_kind(&pending.call)
            || pending.stamp.document != pending.input.document
            || pending.stamp.revision != pending.input.revision
        {
            return Err(RequestRefusal::InvalidInput);
        }
        let bytes =
            captured_bytes(&pending.input, &pending.call).ok_or(RequestRefusal::Oversized)?;
        let state = self.sync.lock();
        if !state.ready {
            return Err(RequestRefusal::NotReady);
        }
        if !state.owns(self.id, &pending.stamp, &pending.input.path) {
            return Err(RequestRefusal::StaleRevision);
        }
        if !self.caps.supports(pending.input.kind) {
            return Err(RequestRefusal::Unsupported);
        }
        let context = ReplyContext {
            stamp: pending.stamp,
            encoding: self.caps.encoding(),
            kind: pending.input.kind,
        };
        let reservation = self.queue.admit_completion(pending.stamp, || {
            self.completion
                .registry
                .reserve(context, observer, bytes)
                .map_err(|error| match error {
                    AdmissionError::Closed => RequestRefusal::NotReady,
                    AdmissionError::Busy => RequestRefusal::Overloaded,
                    AdmissionError::Duplicate => RequestRefusal::InvalidInput,
                    AdmissionError::Oversized => RequestRefusal::Oversized,
                })
        })?;
        drop(state);
        self.completion.spawn(pending, reservation);
        Ok(())
    }

    pub fn cancel_completion(&self, stamp: RequestStamp, reason: CancelReason) {
        self.completion.registry.cancel(stamp, reason);
        self.completion.environment.queue.cancel_completion(stamp);
    }

    /// Arm BEFORE attempting admission, closing the refusal/retirement race.
    /// The caller retains the wake only while it owns completion. Wire progress
    /// also wakes a newest snapshot/request waiting on synchronization capacity.
    pub fn wake_completion_when_available(&self, wake: &crate::completion::CompletionWake) {
        self.completion.registry.wake_when_available(wake);
    }

    pub fn completion_usage(&self) -> CompletionUsage {
        self.completion.registry.usage()
    }
}
