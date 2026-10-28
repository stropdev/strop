//! One physical source job and one replaceable pending request per editor.
use super::{
    flow::{BatchProgress, Flow, SourceSink, StreamSender},
    PickerMsg, SelectionPolicy, SourceSnapshot,
};
use crate::query::SearchQuery;
use parking_lot::{Condvar, Mutex};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Weak,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use strop_core::worker::{
    self, CancelHandle, CancelReason, CancelToken, Failure, FailureKind, Outcome, PreparedWork,
};
use strop_remote::worker_transport::RemoteWorker;
use strop_workspace::{Filesystem, ResourceLocation};

pub type WorkerAdmission = Box<dyn FnOnce(&CancelToken) -> Result<RemoteWorker, Failure> + Send>;

/// Authority for a remote search. The admission callback runs on the
/// picker source thread, never on input→render; browsing alone does not
/// deploy a worker. Once admitted, subsequent queries reuse the lease.
pub enum SearchWorker {
    Admitted(RemoteWorker),
    Admit(WorkerAdmission),
}

const STALL_TIMEOUT: Duration = Duration::from_secs(20);
const STALL_WARNING: &str = "source made no batch progress for 20s; cancelled the stalled request";
const STALLED_FAILURE: &str = "source worker is unavailable after a filesystem stall";

type Work = Box<dyn FnOnce(CancelToken) -> Outcome<()> + Send>;
struct Pending {
    request: u64,
    progress: Arc<BatchProgress>,
    tx: StreamSender,
    prepared: PreparedWork<()>,
    cancel: CancelHandle,
    work: Work,
}
struct Running {
    request: u64,
    progress: Arc<BatchProgress>,
    tx: StreamSender,
    cancel: Option<CancelHandle>,
    stalled: bool,
}
#[derive(Default)]
struct State {
    pending: Option<Pending>,
    running: Option<Running>,
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    watchdog_wake: Condvar,
    next_request: AtomicU64,
    busy: AtomicBool,
    closing: AtomicBool,
    stalled: AtomicBool,
    backlog: Arc<Flow>,
    /// Retained per-scope baselines (0063 residual): hint invalidation
    /// from the editor lands here; the source thread refreshes against
    /// it. Never locked across filesystem I/O.
    cache: Arc<Mutex<super::cache::Caches>>,
}

pub struct SourceWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    _watchdog: Option<JoinHandle<()>>,
}
fn complete_request(shared: &Weak<Shared>, request: u64) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    let finished = {
        let mut state = shared.state.lock();
        let finished = if state
            .running
            .as_ref()
            .is_some_and(|running| running.request == request)
        {
            state.running.take()
        } else {
            None
        };
        if finished.is_some() {
            shared.stalled.store(false, Ordering::Release);
            shared
                .busy
                .store(state.pending.is_some(), Ordering::Release);
        }
        finished
    };
    drop(finished);
    shared.watchdog_wake.notify_all();
}

fn run_watchdog(shared: Arc<Shared>, timeout: Duration) {
    let mut observed: Option<(u64, usize, Instant)> = None;
    let mut state = shared.state.lock();
    loop {
        if shared.closing.load(Ordering::Acquire) {
            return;
        }
        let Some(running) = state.running.as_ref() else {
            observed = None;
            shared.watchdog_wake.wait(&mut state);
            continue;
        };
        if running.stalled {
            observed = None;
            shared.watchdog_wake.wait(&mut state);
            continue;
        }
        let now = Instant::now();
        let progress = running.progress.get();
        if running.progress.delivery_waiting() {
            observed = Some((running.request, progress, now));
            shared.watchdog_wake.wait_for(&mut state, timeout);
            continue;
        }
        let unchanged = observed
            .is_some_and(|(request, ticks, _)| request == running.request && ticks == progress);
        if unchanged
            && observed.is_some_and(|(_, _, since)| now.saturating_duration_since(since) >= timeout)
        {
            if running.progress.get() != progress || running.progress.delivery_waiting() {
                observed = Some((running.request, running.progress.get(), now));
                continue;
            }
            let running = state.running.as_mut().expect("observed request is running");
            running.stalled = true;
            let tx = running.tx.clone();
            let cancel = running.cancel.take();
            let pending = state.pending.take();
            shared.stalled.store(true, Ordering::Release);
            drop(state);
            let _ = tx.control(PickerMsg::Warning(STALL_WARNING.into()));
            if let Some(cancel) = cancel {
                cancel.cancel(CancelReason::OwnerClosed);
            }
            if let Some(pending) = pending {
                pending.cancel.cancel(CancelReason::OwnerClosed);
            }
            state = shared.state.lock();
            continue;
        }
        if !unchanged {
            observed = Some((running.request, progress, now));
        }
        let elapsed = observed.as_ref().map_or(Duration::ZERO, |(_, _, since)| {
            now.saturating_duration_since(*since)
        });
        shared
            .watchdog_wake
            .wait_for(&mut state, timeout.saturating_sub(elapsed));
    }
}

impl SourceWorker {
    pub fn new() -> Result<Self, Failure> {
        Self::with_stall_timeout(STALL_TIMEOUT)
    }

    fn with_stall_timeout(stall_timeout: Duration) -> Result<Self, Failure> {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("picker-source".into())
            .spawn(move || loop {
                let (request, prepared, work) = {
                    let mut state = worker.state.lock();
                    while state.pending.is_none() && !worker.closing.load(Ordering::Acquire) {
                        worker.wake.wait(&mut state);
                    }
                    if worker.closing.load(Ordering::Acquire) {
                        return;
                    }
                    let Some(Pending {
                        request,
                        progress,
                        tx,
                        prepared,
                        cancel,
                        work,
                    }) = state.pending.take()
                    else {
                        continue;
                    };
                    debug_assert!(state.running.is_none());
                    state.running = Some(Running {
                        request,
                        progress,
                        tx,
                        cancel: Some(cancel),
                        stalled: false,
                    });
                    (request, prepared, work)
                };
                worker.watchdog_wake.notify_all();
                prepared.run(work);
                complete_request(&Arc::downgrade(&worker), request);
            })
            .map_err(|error| Failure::new(FailureKind::ThreadStart, error.to_string()))?;
        let watchdog_shared = shared.clone();
        let watchdog = match std::thread::Builder::new()
            .name("picker-source-watchdog".into())
            .spawn(move || run_watchdog(watchdog_shared, stall_timeout))
        {
            Ok(watchdog) => watchdog,
            Err(error) => {
                shared.closing.store(true, Ordering::Release);
                shared.wake.notify_one();
                let _ = thread.join();
                return Err(Failure::new(FailureKind::ThreadStart, error.to_string()));
            }
        };
        Ok(Self {
            shared,
            thread: Some(thread),
            _watchdog: Some(watchdog),
        })
    }

    /// Push coverage for one scope changed (0058 S7): the editor's
    /// subscription was established (`true`) or lost/refused (`false`).
    /// Losing coverage conservatively invalidates the retained baseline.
    pub fn set_watching(&self, root: &std::path::Path, watching: bool) {
        self.shared.cache.lock().set_watching(root, watching);
    }

    /// Notification hints for one covered scope (0058 S7): paths are
    /// relative to `root`; `rescan` is the conservative overflow/loss
    /// obligation. The next search over each covered scope rescans the
    /// invalidated subtrees and reuses the rest.
    pub fn invalidate(&self, root: &std::path::Path, paths: &[PathBuf], rescan: bool) {
        self.shared.cache.lock().invalidate(root, paths, rescan);
    }

    /// Last refresh accounting for a scope (0058 S7 test evidence).
    #[cfg(test)]
    pub(crate) fn scan_stats(&self, root: &std::path::Path) -> Option<super::cache::BaselineStats> {
        self.shared.cache.lock().stats(root)
    }

    fn submit(
        &self,
        sender: impl Into<SourceSink>,
        work: impl FnOnce(StreamSender, CancelToken) -> Outcome<()> + Send + 'static,
    ) -> CancelHandle {
        let request = self.shared.next_request.fetch_add(1, Ordering::AcqRel);
        let progress = Arc::new(BatchProgress::default());
        let tx = StreamSender::new(sender.into(), self.shared.backlog.clone(), progress.clone());
        let terminal = tx.clone();
        let prepared = worker::prepare(move |outcome| {
            let _ = terminal.control(PickerMsg::Finished(outcome));
        });
        let handle = prepared.cancel_handle();
        let cancel = prepared.cancel_handle();
        let pending = Pending {
            request,
            progress,
            tx: tx.clone(),
            prepared,
            cancel,
            work: Box::new(move |token| work(tx, token)),
        };
        let (retired, running) = {
            let mut state = self.shared.state.lock();
            if self.shared.closing.load(Ordering::Acquire) {
                drop(state);
                pending.cancel.cancel(CancelReason::Shutdown);
                return handle;
            }
            if self.shared.stalled.load(Ordering::Acquire) {
                drop(state);
                pending
                    .prepared
                    .run(|_| Outcome::failed(FailureKind::Unavailable, STALLED_FAILURE));
                return handle;
            }
            let retired = state.pending.replace(pending);
            let running = state.running.take();
            self.shared.busy.store(true, Ordering::Release);
            (retired, running)
        };
        if let Some(retired) = retired {
            retired.cancel.cancel(CancelReason::Superseded);
        }
        if let Some(running) = running {
            if let Some(cancel) = running.cancel {
                cancel.cancel(CancelReason::Superseded);
            }
        }
        self.shared.wake.notify_one();
        handle
    }

    pub fn files(
        &self,
        root: PathBuf,
        query: Arc<SearchQuery>,
        policy: SelectionPolicy,
        sender: impl Into<SourceSink>,
    ) -> CancelHandle {
        self.submit(sender, move |tx, token| {
            super::run_files(root, query, policy, tx, token)
        })
    }

    /// One-shot: every declaration in the opened scope (0063 §2).
    /// The item list is static — the picker ranks locally per keystroke.
    pub fn workspace_symbols(
        &self,
        root: PathBuf,
        query: Arc<SearchQuery>,
        policy: SelectionPolicy,
        sender: impl Into<SourceSink>,
    ) -> CancelHandle {
        let cache = Arc::clone(&self.shared.cache);
        self.submit(sender, move |tx, token| {
            super::run_workspace_symbols(root, query, policy, cache, tx, token)
        })
    }

    pub fn search(
        &self,
        query: Arc<SearchQuery>,
        policy: SelectionPolicy,
        root: ResourceLocation,
        snapshots: Vec<SourceSnapshot>,
        lease: Option<SearchWorker>,
        sender: impl Into<SourceSink>,
    ) -> CancelHandle {
        let cache = Arc::clone(&self.shared.cache);
        self.submit(sender, move |tx, token| match &root.filesystem {
            Filesystem::Local => {
                super::grep::run(query, policy, root.path, snapshots, cache, tx, token)
            }
            Filesystem::Remote(_) => {
                let admitted = match lease {
                    Some(SearchWorker::Admitted(worker)) => Some(worker),
                    Some(SearchWorker::Admit(admit)) => match admit(&token) {
                        Ok(worker) => Some(worker),
                        Err(failure) => {
                            return Outcome::Failed {
                                failure,
                                partial: None,
                            }
                        }
                    },
                    None => None,
                };
                super::remote::run_search(query, policy, root, snapshots, admitted, tx, token)
            }
            Filesystem::Container(_) => Outcome::failed(
                FailureKind::Unavailable,
                "container search is unsupported; no local fallback",
            ),
        })
    }

    pub fn busy(&self) -> bool {
        if self.shared.closing.load(Ordering::Acquire)
            || self.shared.stalled.load(Ordering::Acquire)
        {
            self.thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
        } else {
            self.shared.busy.load(Ordering::Acquire)
        }
    }

    pub fn close(&self) {
        let (pending, running) = {
            let mut state = self.shared.state.lock();
            self.shared.closing.store(true, Ordering::Release);
            (state.pending.take(), state.running.take())
        };
        if let Some(pending) = pending {
            pending.cancel.cancel(CancelReason::Shutdown);
        }
        if let Some(running) = running {
            if let Some(cancel) = running.cancel {
                cancel.cancel(CancelReason::Shutdown);
            }
        }
        self.shared.wake.notify_one();
        self.shared.watchdog_wake.notify_all();
    }
}
impl Drop for SourceWorker {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, TryRecvError};
    use std::time::Duration;

    fn terminal(receiver: &Receiver<PickerMsg>) -> Outcome<()> {
        match receiver.recv_timeout(Duration::from_secs(5)).unwrap() {
            PickerMsg::Finished(outcome) => outcome,
            event => panic!("unexpected source event: {event:?}"),
        }
    }

    #[test]
    fn only_the_latest_pending_request_runs_after_physical_work_drains() {
        let mut worker = SourceWorker::new().unwrap();
        let (first_tx, first_rx) = channel();
        let (started, entered) = channel();
        let (release, released) = channel();
        let _first = worker.submit(first_tx, move |_, _| {
            started.send(()).unwrap();
            released.recv().unwrap();
            Outcome::Success(())
        });
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let (second_tx, second_rx) = channel();
        let _second = worker.submit(second_tx, |_, _| panic!("superseded pending work ran"));
        let (third_tx, third_rx) = channel();
        let (third_started, third_entered) = channel();
        let _third = worker.submit(third_tx, move |_, _| {
            third_started.send(()).unwrap();
            Outcome::Success(())
        });
        assert!(matches!(
            terminal(&first_rx),
            Outcome::Cancelled(CancelReason::Superseded)
        ));
        assert!(matches!(
            terminal(&second_rx),
            Outcome::Cancelled(CancelReason::Superseded)
        ));
        assert!(worker.busy());
        assert_eq!(third_entered.try_recv(), Err(TryRecvError::Empty));
        release.send(()).unwrap();
        third_entered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(terminal(&third_rx), Outcome::Success(())));
        worker.close();
        worker.thread.take().unwrap().join().unwrap();
    }

    #[test]
    fn a_stalled_source_recovers_after_physical_work_returns() {
        let mut worker = SourceWorker::with_stall_timeout(Duration::from_millis(20)).unwrap();
        let (first_tx, first_rx) = channel();
        let (started, entered) = channel();
        let (release, released) = channel();
        let _first = worker.submit(first_tx, move |_, _| {
            started.send(()).unwrap();
            released.recv().unwrap();
            Outcome::Success(())
        });
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        match first_rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            PickerMsg::Warning(message) => assert!(message.contains("no batch progress")),
            event => panic!("a stall must be visible: {event:?}"),
        }
        assert!(matches!(
            terminal(&first_rx),
            Outcome::Cancelled(CancelReason::OwnerClosed)
        ));
        assert!(matches!(first_rx.try_recv(), Err(TryRecvError::Empty)));
        assert!(worker.busy(), "the stalled physical owner is still running");

        let (later_tx, later_rx) = channel();
        let _later = worker.submit(later_tx, |_, _| panic!("stalled worker ran later work"));
        assert!(matches!(
            terminal(&later_rx),
            Outcome::Failed { failure, .. }
                if failure.kind == FailureKind::Unavailable
        ));

        release.send(()).unwrap();
        let mut state = worker.shared.state.lock();
        while worker.shared.stalled.load(Ordering::Acquire) {
            worker.shared.watchdog_wake.wait(&mut state);
        }
        assert!(state.running.is_none());
        drop(state);
        assert!(!worker.busy());

        let (recovered_tx, recovered_rx) = channel();
        let (recovered, recovered_entered) = channel();
        let _recovered = worker.submit(recovered_tx, move |_, _| {
            recovered.send(()).unwrap();
            Outcome::Success(())
        });
        recovered_entered
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(terminal(&recovered_rx), Outcome::Success(())));
        worker.close();
        worker.thread.take().unwrap().join().unwrap();
        worker._watchdog.take().unwrap().join().unwrap();
        assert!(matches!(
            first_rx.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn shutdown_retires_pending_work_but_waits_for_the_running_owner() {
        let mut worker = SourceWorker::new().unwrap();
        let (first_tx, first_rx) = channel();
        let (started, entered) = channel();
        let (release, released) = channel();
        let _first = worker.submit(first_tx, move |_, _| {
            started.send(()).unwrap();
            released.recv().unwrap();
            Outcome::Success(())
        });
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let (pending_tx, pending_rx) = channel();
        let _pending = worker.submit(pending_tx, |_, _| panic!("shutdown pending work ran"));
        let _ = terminal(&first_rx);
        worker.close();
        assert!(matches!(
            terminal(&pending_rx),
            Outcome::Cancelled(CancelReason::Shutdown)
        ));
        assert!(
            worker.busy(),
            "logical cancellation is not physical completion"
        );
        release.send(()).unwrap();
        worker.thread.take().unwrap().join().unwrap();
        assert!(matches!(
            pending_rx.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Disconnected)
        ));
    }
}
