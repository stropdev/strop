//! Logical cancellation and physical request retirement are different states.
//! A cancelled sent request keeps its slot until the peer answers or closes.
use crate::completion::{
    CompletionReply, CompletionResult, CompletionSink, CompletionTransportEvent,
};
use crate::{ReplyContext, RequestStamp};
use parking_lot::{Condvar, Mutex};
use std::sync::Arc;
use std::task::{Context, Waker};
use strop_core::worker::{CancelReason, FailureKind, Outcome};
use tokio::sync::oneshot;

pub(super) const MAX_PHYSICAL_REQUESTS: usize = 2;
pub(super) const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Disposition {
    Active,
    Cancelled(CancelReason),
    Expired,
    Finished,
}
struct ControlState {
    disposition: Disposition,
    wake: Option<Waker>,
    submitted: bool,
}
pub(super) struct Control {
    state: Mutex<ControlState>,
    framed: Condvar,
}
impl Control {
    fn new() -> Self {
        Self {
            state: Mutex::new(ControlState {
                disposition: Disposition::Active,
                wake: None,
                submitted: false,
            }),
            framed: Condvar::new(),
        }
    }
    pub fn poll(&self, context: &Context<'_>) -> Disposition {
        let mut state = self.state.lock();
        if state.disposition != Disposition::Finished
            && state
                .wake
                .as_ref()
                .is_none_or(|wake| !wake.will_wake(context.waker()))
        {
            state.wake = Some(context.waker().clone());
        }
        state.disposition
    }
    pub fn disposition(&self) -> Disposition {
        self.state.lock().disposition
    }
    pub fn cancel(&self, reason: CancelReason) {
        self.transition(Disposition::Cancelled(reason));
    }
    pub fn expire(&self) {
        self.transition(Disposition::Expired);
    }
    fn transition(&self, next: Disposition) {
        let mut state = self.state.lock();
        if state.disposition != Disposition::Active {
            return;
        }
        state.disposition = next;
        let wake = state.wake.take();
        drop(state);
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    fn finish(&self) -> Disposition {
        let mut state = self.state.lock();
        let before = state.disposition;
        state.disposition = Disposition::Finished;
        let wake = state.wake.take();
        drop(state);
        self.framed.notify_all();
        if let Some(wake) = wake {
            wake.wake();
        }
        before
    }
    pub fn submitted(&self) {
        self.state.lock().submitted = true;
        self.framed.notify_all();
    }
    /// Only the ordered wire thread waits here. Serialization and enqueueing
    /// therefore never hold the input-side document synchronization lock.
    fn wait_submitted(&self) {
        let mut state = self.state.lock();
        while !state.submitted && state.disposition != Disposition::Finished {
            self.framed.wait(&mut state);
        }
    }
}

#[derive(Clone, Copy)]
enum Terminal {
    Pending,
    Delivered,
}
struct Slot {
    context: ReplyContext,
    observer: CompletionSink,
    control: Arc<Control>,
    start: Option<oneshot::Sender<()>>,
    terminal: Terminal,
    capture_bytes: usize,
}
struct State {
    slots: Vec<Slot>,
    capture_bytes: usize,
    closed: bool,
    waiter: Option<std::sync::Weak<dyn Fn() + Send + Sync>>,
}

pub(super) struct Reservation {
    pub control: Arc<Control>,
    pub start: oneshot::Receiver<()>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdmissionError {
    Closed,
    Busy,
    Duplicate,
    Oversized,
}

pub(in crate::client) struct Registry(Mutex<State>);
impl Default for Registry {
    fn default() -> Self {
        Self(Mutex::new(State {
            slots: Vec::with_capacity(MAX_PHYSICAL_REQUESTS),
            capture_bytes: 0,
            closed: false,
            waiter: None,
        }))
    }
}
impl Registry {
    pub(super) fn reserve(
        &self,
        context: ReplyContext,
        observer: &CompletionSink,
        capture_bytes: usize,
    ) -> Result<Reservation, AdmissionError> {
        let mut state = self.0.lock();
        if state.closed {
            return Err(AdmissionError::Closed);
        }
        if capture_bytes > MAX_CAPTURE_BYTES {
            return Err(AdmissionError::Oversized);
        }
        if state
            .slots
            .iter()
            .any(|slot| slot.context.stamp == context.stamp)
        {
            return Err(AdmissionError::Duplicate);
        }
        if state.slots.len() >= MAX_PHYSICAL_REQUESTS
            || state.capture_bytes + capture_bytes > MAX_CAPTURE_BYTES
        {
            return Err(AdmissionError::Busy);
        }
        let control = Arc::new(Control::new());
        let (start, receiver) = oneshot::channel();
        state.capture_bytes += capture_bytes;
        state.slots.push(Slot {
            context,
            observer: observer.clone(),
            control: control.clone(),
            start: Some(start),
            terminal: Terminal::Pending,
            capture_bytes,
        });
        Ok(Reservation {
            control,
            start: receiver,
        })
    }

    /// The ordered wire grants this only after earlier document frames flush.
    pub fn start(&self, stamp: RequestStamp) {
        let target = self
            .0
            .lock()
            .slots
            .iter_mut()
            .find(|slot| slot.context.stamp == stamp)
            .and_then(|slot| slot.start.take().map(|start| (start, slot.control.clone())));
        if let Some((start, control)) = target {
            if start.send(()).is_ok() {
                control.wait_submitted();
            }
        }
    }

    pub fn cancel(&self, stamp: RequestStamp, reason: CancelReason) {
        let control = self
            .0
            .lock()
            .slots
            .iter()
            .find(|slot| slot.context.stamp == stamp)
            .map(|slot| slot.control.clone());
        if let Some(control) = control {
            control.cancel(reason);
        }
    }

    /// Complete logical ownership without freeing an unacknowledged wire slot.
    pub(super) fn stop_logical(&self, stamp: RequestStamp, disposition: Disposition) {
        if matches!(disposition, Disposition::Active | Disposition::Finished) {
            return;
        }
        let target = {
            let mut state = self.0.lock();
            state
                .slots
                .iter_mut()
                .find(|slot| slot.context.stamp == stamp)
                .filter(|slot| matches!(slot.terminal, Terminal::Pending))
                .map(|slot| {
                    slot.terminal = Terminal::Delivered;
                    (slot.context, slot.observer.clone())
                })
        };
        if let Some((context, observer)) = target {
            let Some(outcome) = stopped_outcome(disposition) else {
                unreachable!("only cancelled/expired ownership reaches settlement");
            };
            observer(CompletionTransportEvent::Reply(CompletionReply {
                context,
                outcome,
            }));
        }
    }

    pub fn finish(&self, stamp: RequestStamp, outcome: Outcome<CompletionResult>) {
        let (slot, closed) = {
            let mut state = self.0.lock();
            let slot = state
                .slots
                .iter()
                .position(|slot| slot.context.stamp == stamp)
                .map(|index| state.slots.swap_remove(index));
            if let Some(slot) = &slot {
                state.capture_bytes -= slot.capture_bytes;
            }
            (slot, state.closed)
        };
        let Some(slot) = slot else { return };
        let disposition = slot.control.finish();
        if matches!(slot.terminal, Terminal::Pending) {
            let outcome = stopped_outcome(disposition).unwrap_or(outcome);
            (slot.observer)(CompletionTransportEvent::Reply(CompletionReply {
                context: slot.context,
                outcome,
            }));
        }
        if !closed {
            (slot.observer)(CompletionTransportEvent::Capacity {
                server: stamp.server,
            });
        }
        self.wake_available();
    }

    /// Runtime teardown: settle every remaining owner once, then release its
    /// callback/data owners here on the service thread, never the input thread.
    pub fn close(&self) {
        let slots = {
            let mut state = self.0.lock();
            state.closed = true;
            state.capture_bytes = 0;
            std::mem::take(&mut state.slots)
        };
        for slot in slots {
            let disposition = slot.control.finish();
            if matches!(slot.terminal, Terminal::Pending) {
                let outcome = stopped_outcome(disposition).unwrap_or_else(|| {
                    Outcome::failed(
                        FailureKind::Disconnected,
                        "language-service connection closed",
                    )
                });
                (slot.observer)(CompletionTransportEvent::Reply(CompletionReply {
                    context: slot.context,
                    outcome,
                }));
            }
            (slot.observer)(CompletionTransportEvent::Closed {
                server: slot.context.stamp.server,
            });
        }
        self.wake_available();
    }

    pub fn wake_when_available(&self, wake: &crate::completion::CompletionWake) {
        self.0.lock().waiter = Some(Arc::downgrade(wake));
    }
    pub fn wake_available(&self) {
        let waiter = self.0.lock().waiter.take();
        if let Some(wake) = waiter.and_then(|waiter| waiter.upgrade()) {
            wake();
        }
    }

    pub fn usage(&self) -> crate::completion::CompletionUsage {
        let state = self.0.lock();
        crate::completion::CompletionUsage {
            physical_requests: state.slots.len(),
            captured_bytes: state.capture_bytes,
        }
    }
}

pub(super) fn stopped_outcome(disposition: Disposition) -> Option<Outcome<CompletionResult>> {
    match disposition {
        Disposition::Cancelled(reason) => Some(Outcome::Cancelled(reason)),
        Disposition::Expired => Some(Outcome::failed(
            FailureKind::Unavailable,
            "completion timed out",
        )),
        Disposition::Active | Disposition::Finished => None,
    }
}
