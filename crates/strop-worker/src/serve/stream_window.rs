//! Consumer-driven flow control for file reads, exec output and PTY VT bytes.
//!
//! Each stream begins with 32 chunk credits, below the client's 64-chunk
//! inbound bound. Only the producing pump parks when exhausted; the
//! reader still admits credit, health and cancellation. A discarded
//! process stream drains its child without retaining more output.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use strop_core::worker::CancelToken;
use strop_worker_protocol::{StreamId, STREAM_WINDOW_CHUNKS};

use super::SessionState;

struct WindowState {
    credits: usize,
    abandoned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowUse {
    Emit,
    Discard,
    Stopped,
}

pub(super) struct StreamWindow {
    state: Mutex<WindowState>,
    available: Condvar,
}

impl StreamWindow {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(WindowState {
                credits: STREAM_WINDOW_CHUNKS,
                abandoned: false,
            }),
            available: Condvar::new(),
        }
    }

    pub(super) fn grant(&self, count: usize) {
        let mut state = self.state.lock();
        if !state.abandoned {
            state.credits = state
                .credits
                .saturating_add(count)
                .min(STREAM_WINDOW_CHUNKS);
            self.available.notify_one();
        }
    }

    pub(super) fn abandon(&self) {
        self.state.lock().abandoned = true;
        self.available.notify_all();
    }

    pub(super) fn take(&self, token: &CancelToken, stop: &AtomicBool) -> WindowUse {
        let mut state = self.state.lock();
        loop {
            if token.is_cancelled() || stop.load(Ordering::Acquire) {
                return WindowUse::Stopped;
            }
            if state.abandoned {
                return WindowUse::Discard;
            }
            if state.credits > 0 {
                state.credits -= 1;
                return WindowUse::Emit;
            }
            // Producers park; control still admits credit, abandon,
            // cancellation and health. The timeout bounds retirement
            // if cancellation races the last credit.
            self.available
                .wait_for(&mut state, Duration::from_millis(10));
        }
    }
}

/// Own one session-scoped stream's window until EOF or cancellation.
/// Late credits for a removed stream cannot reach a later incarnation.
pub(super) struct StreamRegistration {
    shared: Arc<SessionState>,
    stream: StreamId,
    window: Arc<StreamWindow>,
}

impl StreamRegistration {
    pub(super) fn new(shared: &Arc<SessionState>, stream: StreamId) -> Self {
        let window = Arc::new(StreamWindow::new());
        let prior = shared
            .stream_windows
            .lock()
            .insert(stream, Arc::clone(&window));
        debug_assert!(prior.is_none(), "stream window ids must be unique");
        Self {
            shared: Arc::clone(shared),
            stream,
            window,
        }
    }

    pub(super) fn take(&self, token: &CancelToken) -> WindowUse {
        self.window.take(token, &self.shared.stop)
    }
}

impl Drop for StreamRegistration {
    fn drop(&mut self) {
        self.shared.stream_windows.lock().remove(&self.stream);
    }
}
