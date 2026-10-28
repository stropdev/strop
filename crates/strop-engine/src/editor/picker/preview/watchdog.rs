//! Preview liveness: one editor-owned watchdog for physical reader completion.
//! Logical cancellation never hides a blocked read; stalls settle once and
//! refuse further work until every blocked reader has physically returned.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
    Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use strop_core::worker::{Failure, FailureKind, Outcome, Ticket};

use super::{PreparedPreview, PreviewKey, PreviewResult};

const PREVIEW_STALL_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const PREVIEW_STALLED: &str = "preview worker is unavailable after a filesystem stall";

struct WatchEntry {
    ticket: Ticket<PreviewKey>,
    tx: Sender<PreviewResult>,
    terminal: Arc<AtomicBool>,
    deadline: Instant,
}
#[derive(Default)]
struct WatchState {
    requests: HashMap<u64, WatchEntry>,
    stalled: bool,
    closing: bool,
}
#[derive(Default)]
pub(crate) struct WatchShared {
    state: Mutex<WatchState>,
    wake: Condvar,
}

/// One physical watchdog per editor. Entries remain registered after logical
/// cancellation until their reader actually returns: a blocked read must not
/// become invisible merely because selection moved to another path.
pub(crate) struct PreviewWatchdog {
    pub(crate) shared: Arc<WatchShared>,
    thread: Option<JoinHandle<()>>,
    timeout: Duration,
}
impl Default for PreviewWatchdog {
    fn default() -> Self {
        Self::with_timeout(PREVIEW_STALL_TIMEOUT)
    }
}
impl PreviewWatchdog {
    fn with_timeout(timeout: Duration) -> Self {
        Self {
            shared: Arc::default(),
            thread: None,
            timeout,
        }
    }

    pub(super) fn is_stalled(&self) -> bool {
        self.shared.state.lock().stalled
    }

    pub(super) fn watch(
        &mut self,
        ticket: Ticket<PreviewKey>,
        tx: Sender<PreviewResult>,
        terminal: Arc<AtomicBool>,
    ) -> Result<(), Failure> {
        {
            let state = self.shared.state.lock();
            if state.stalled {
                return Err(Failure::new(FailureKind::Unavailable, PREVIEW_STALLED));
            }
            if state.closing {
                return Err(Failure::new(
                    FailureKind::Unavailable,
                    "preview worker is closing",
                ));
            }
        }
        if self.thread.is_none() {
            let shared = self.shared.clone();
            self.thread = Some(
                std::thread::Builder::new()
                    .name("strop-preview-watchdog".into())
                    .spawn(move || run_watchdog(shared))
                    .map_err(|error| Failure::new(FailureKind::ThreadStart, error.to_string()))?,
            );
        }
        let mut state = self.shared.state.lock();
        if state.stalled {
            return Err(Failure::new(FailureKind::Unavailable, PREVIEW_STALLED));
        }
        state.requests.insert(
            ticket.request.get(),
            WatchEntry {
                ticket,
                tx,
                terminal,
                deadline: Instant::now() + self.timeout,
            },
        );
        drop(state);
        self.shared.wake.notify_all();
        Ok(())
    }
}
impl Drop for PreviewWatchdog {
    fn drop(&mut self) {
        self.shared.state.lock().closing = true;
        self.shared.wake.notify_all();
    }
}

pub(crate) fn physical_complete(shared: &WatchShared, request: u64) {
    let mut state = shared.state.lock();
    state.requests.remove(&request);
    if state.requests.is_empty() {
        state.stalled = false;
    }
    drop(state);
    shared.wake.notify_all();
}

pub(crate) fn publish(
    terminal: &AtomicBool,
    tx: &Sender<PreviewResult>,
    ticket: Ticket<PreviewKey>,
    outcome: Outcome<PreparedPreview>,
) -> bool {
    if terminal
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    let _ = tx.send(PreviewResult { ticket, outcome });
    true
}

fn run_watchdog(shared: Arc<WatchShared>) {
    let mut state = shared.state.lock();
    loop {
        if state.closing {
            return;
        }
        if state.stalled {
            shared.wake.wait(&mut state);
            continue;
        }
        let now = Instant::now();
        let next = state.requests.values().map(|entry| entry.deadline).min();
        match next {
            None => shared.wake.wait(&mut state),
            Some(deadline) if deadline > now => {
                shared.wake.wait_for(&mut state, deadline - now);
            }
            Some(_) => {
                state.stalled = true;
                for entry in state.requests.values() {
                    publish(
                        &entry.terminal,
                        &entry.tx,
                        entry.ticket.clone(),
                        Outcome::failed(FailureKind::Unavailable, PREVIEW_STALLED),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use std::sync::mpsc::{channel, TryRecvError};
    use strop_core::worker::{CancelReason, Load};
    use strop_core::Buffer;
    use strop_picker::Kind;

    fn editor() -> Editor {
        let mut editor = Editor::new(Buffer::from_text("x\n"));
        editor.open_picker(Kind::Search);
        editor.wait_picker();
        editor
    }

    #[test]
    fn stalled_preview_recovers_after_physical_reader_returns() {
        let mut editor = editor();
        editor.preview_watchdog = PreviewWatchdog::with_timeout(Duration::from_millis(20));
        let first = strop_workspace::ResourceLocation::local(editor.cwd.join("blocked.txt"));
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        let (returned_tx, returned_rx) = channel();
        assert!(!editor.preview_ready_with(&first, move |_, _, _| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            returned_tx.send(()).unwrap();
            Outcome::Success(String::from("late").into())
        }));
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        let result = editor
            .preview_rx
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            &result.outcome,
            Outcome::Failed { failure, .. } if failure.kind == FailureKind::Unavailable
        ));
        editor.handle_preview(result);
        assert!(matches!(
            editor.preview_loads.get(&first),
            Some(Load::Failed { failure, .. }) if failure.kind == FailureKind::Unavailable
        ));

        let second = strop_workspace::ResourceLocation::local(editor.cwd.join("later.txt"));
        assert!(!editor.preview_ready_with(&second, |_, _, _| {
            panic!("a stalled editor spawned later preview work")
        }));
        let later = editor
            .preview_rx
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            &later.outcome,
            Outcome::Failed { failure, .. } if failure.kind == FailureKind::Unavailable
        ));
        editor.handle_preview(later);
        assert!(matches!(
            editor.preview_rx.as_ref().unwrap().try_recv(),
            Err(TryRecvError::Empty)
        ));

        release_tx.send(()).unwrap();
        returned_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            editor.preview_rx.as_ref().unwrap().try_recv(),
            Err(TryRecvError::Empty)
        ));

        let recovery_deadline = Instant::now() + Duration::from_secs(5);
        while editor.preview_watchdog.is_stalled() {
            assert!(
                Instant::now() < recovery_deadline,
                "watchdog did not recover after the physical reader returned"
            );
            std::thread::yield_now();
        }
        let recovered = strop_workspace::ResourceLocation::local(editor.cwd.join("recovered.txt"));
        assert!(!editor.preview_ready_with(&recovered, |_, _, _| {
            Outcome::Success(String::from("recovered").into())
        }));
        let ready = editor
            .preview_rx
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        editor.handle_preview(ready);
        assert_eq!(editor.previews[&recovered].rope.to_string(), "recovered");
        editor.close_picker();
    }

    #[test]
    fn moving_paths_cancels_the_previous_preview_before_launch() {
        let mut editor = editor();
        let first = strop_workspace::ResourceLocation::local(editor.cwd.join("first.txt"));
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        assert!(!editor.preview_ready_with(&first, move |_, _, _| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Outcome::Success(String::from("stale").into())
        }));
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let first_ticket = match editor.preview_loads.get(&first) {
            Some(Load::Running(ticket)) => ticket.clone(),
            load => panic!("first preview was not registered: {load:?}"),
        };

        let second = strop_workspace::ResourceLocation::local(editor.cwd.join("second.txt"));
        assert!(!editor.preview_ready_with(&second, |_, _, _| {
            Outcome::Success(String::from("current").into())
        }));
        assert!(!editor.preview_loads.contains_key(&first));
        assert!(!editor.worker_handles.contains_key(&first_ticket.request));
        let cancelled = editor
            .preview_rx
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(cancelled.ticket, first_ticket);
        assert!(matches!(
            cancelled.outcome,
            Outcome::Cancelled(CancelReason::Superseded)
        ));
        editor.handle_preview(cancelled);
        let ready = editor
            .preview_rx
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        editor.handle_preview(ready);
        assert_eq!(editor.previews[&second].rope.to_string(), "current");
        assert!(!editor.previews.contains_key(&first));

        release_tx.send(()).unwrap();
        editor.close_picker();
    }
}
