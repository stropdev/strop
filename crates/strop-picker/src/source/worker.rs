//! One physical source job and one replaceable pending request per editor.
use super::{
    flow::{Flow, SourceSink, StreamSender},
    PickerMsg, SelectionPolicy, SourceSnapshot,
};
use crate::query::SearchQuery;
use parking_lot::{Condvar, Mutex};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
};
use strop_core::worker::{
    self, CancelHandle, CancelReason, CancelToken, Failure, FailureKind, Outcome, PreparedWork,
};
use strop_workspace::{Filesystem, ResourceLocation};

type Work = Box<dyn FnOnce(CancelToken) -> Outcome<()> + Send>;
struct Pending {
    prepared: PreparedWork<()>,
    cancel: CancelHandle,
    work: Work,
}
#[derive(Default)]
struct State {
    pending: Option<Pending>,
    running: Option<CancelHandle>,
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    busy: AtomicBool,
    closing: AtomicBool,
    backlog: Arc<Flow>,
}

pub struct SourceWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl SourceWorker {
    pub fn new() -> Result<Self, Failure> {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("picker-source".into())
            .spawn(move || loop {
                let (prepared, work) = {
                    let mut state = worker.state.lock();
                    while state.pending.is_none() && !worker.closing.load(Ordering::Acquire) {
                        worker.wake.wait(&mut state);
                    }
                    if worker.closing.load(Ordering::Acquire) {
                        return;
                    }
                    let Some(Pending {
                        prepared,
                        cancel,
                        work,
                    }) = state.pending.take()
                    else {
                        continue;
                    };
                    debug_assert!(state.running.is_none());
                    state.running = Some(cancel);
                    (prepared, work)
                };
                prepared.run(work);
                let finished = {
                    let mut state = worker.state.lock();
                    let finished = state.running.take();
                    worker
                        .busy
                        .store(state.pending.is_some(), Ordering::Release);
                    finished
                };
                drop(finished);
            })
            .map_err(|error| Failure::new(FailureKind::ThreadStart, error.to_string()))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    fn submit(
        &self,
        sender: impl Into<SourceSink>,
        work: impl FnOnce(StreamSender, CancelToken) -> Outcome<()> + Send + 'static,
    ) -> CancelHandle {
        let tx = StreamSender::new(sender.into(), self.shared.backlog.clone());
        let terminal = tx.clone();
        let prepared = worker::prepare(move |outcome| {
            let _ = terminal.control(PickerMsg::Finished(outcome));
        });
        let handle = prepared.cancel_handle();
        let cancel = prepared.cancel_handle();
        let pending = Pending {
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
            let retired = state.pending.replace(pending);
            let running = state.running.take();
            self.shared.busy.store(true, Ordering::Release);
            (retired, running)
        };
        if let Some(retired) = retired {
            retired.cancel.cancel(CancelReason::Superseded);
        }
        if let Some(running) = running {
            running.cancel(CancelReason::Superseded);
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
    pub fn search(
        &self,
        query: Arc<SearchQuery>,
        policy: SelectionPolicy,
        root: ResourceLocation,
        snapshots: Vec<SourceSnapshot>,
        sender: impl Into<SourceSink>,
    ) -> CancelHandle {
        self.submit(sender, move |tx, token| match &root.filesystem {
            Filesystem::Local => super::grep::run(query, policy, root.path, snapshots, tx, token),
            Filesystem::Remote(_) => {
                super::remote::run_search(query, policy, root, snapshots, tx, token)
            }
            Filesystem::Container(_) => Outcome::failed(
                FailureKind::Unavailable,
                "container search is unsupported; no local fallback",
            ),
        })
    }

    pub fn busy(&self) -> bool {
        if self.shared.closing.load(Ordering::Acquire) {
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
            running.cancel(CancelReason::Shutdown);
        }
        self.shared.wake.notify_one();
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
