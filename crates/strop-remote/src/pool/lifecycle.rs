//! Published connection state and shutdown acknowledgment. Last-lease drop
//! only sets an atomic flag; explicit worker disconnect can await the result.
use crate::{ReadFailureKind, ReadStage, RemoteReadError};
use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Default)]
pub(crate) struct StopSignal {
    stopped: AtomicBool,
    outcome: Mutex<Option<Result<(), RemoteReadError>>>,
    finished: Condvar,
}
impl StopSignal {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn signal(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    pub(crate) fn signalled(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(crate) fn complete(&self, result: Result<(), RemoteReadError>) {
        let mut outcome = self.outcome.lock();
        if outcome.is_none() {
            *outcome = Some(result);
            self.finished.notify_all();
        }
    }
    pub(crate) fn wait(&self) -> Result<(), RemoteReadError> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut outcome = self.outcome.lock();
        loop {
            if let Some(result) = outcome.as_ref() {
                return result.clone();
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(RemoteReadError::bare(
                    ReadStage::Teardown,
                    ReadFailureKind::Deadline,
                    "session teardown was not acknowledged",
                ));
            }
            self.finished.wait_for(&mut outcome, remaining);
        }
    }
}
pub(crate) struct ActorExit(pub(crate) Arc<StopSignal>);
impl Drop for ActorExit {
    fn drop(&mut self) {
        self.0.signal();
        self.0.complete(Err(RemoteReadError::bare(
            ReadStage::Teardown,
            ReadFailureKind::Io,
            "session actor exited without a cleanup acknowledgment",
        )));
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum SessionState {
    Disconnected = 0,
    Connecting = 1,
    Connected = 2,
}
#[derive(Debug, Default)]
pub(crate) struct StatusCell(AtomicU8);
impl StatusCell {
    pub(crate) fn set(&self, state: SessionState) {
        self.0.store(state as u8, Ordering::Release);
    }
    pub(crate) fn get(&self) -> SessionState {
        match self.0.load(Ordering::Acquire) {
            1 => SessionState::Connecting,
            2 => SessionState::Connected,
            _ => SessionState::Disconnected,
        }
    }
}
