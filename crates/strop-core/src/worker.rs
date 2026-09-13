use parking_lot::Mutex;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

mod effect;
pub use effect::spawn_effect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct WorkerId(u64);
impl WorkerId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct WorkerIds(u64);
impl WorkerIds {
    pub fn allocate(&mut self) -> Result<WorkerId, Failure> {
        let next = self.0.checked_add(1).ok_or_else(|| {
            Failure::new(
                FailureKind::IdentityExhausted,
                "worker request IDs exhausted",
            )
        })?;
        self.0 = next;
        Ok(WorkerId::new(next))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FailureKind {
    IdentityExhausted,
    ThreadStart,
    Panic,
    Io,
    Spawn,
    Wait,
    Exit,
    InvalidInput,
    Unavailable,
    Protocol,
    Disconnected,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
}
impl Failure {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CancelReason {
    Superseded,
    OwnerClosed,
    Dismissed,
    Shutdown,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Outcome<T> {
    Success(T),
    Failed {
        failure: Failure,
        partial: Option<T>,
    },
    Cancelled(CancelReason),
}
impl<T> Outcome<T> {
    pub fn failed(kind: FailureKind, message: impl Into<String>) -> Self {
        Self::Failed {
            failure: Failure::new(kind, message),
            partial: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ticket<K> {
    pub request: WorkerId,
    pub key: K,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Completion<K, T> {
    pub ticket: Ticket<K>,
    pub outcome: Outcome<T>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Load<K> {
    Idle,
    Running(Ticket<K>),
    Ready(K),
    Failed { key: K, failure: Failure },
    Cancelled { key: K, reason: CancelReason },
}
impl<K: PartialEq> Load<K> {
    pub fn owns(&self, ticket: &Ticket<K>) -> bool {
        matches!(self, Self::Running(current) if current == ticket)
    }
    pub fn covers(&self, key: &K) -> bool {
        match self {
            Self::Idle => false,
            Self::Running(t) => &t.key == key,
            Self::Ready(k) | Self::Failed { key: k, .. } | Self::Cancelled { key: k, .. } => {
                k == key
            }
        }
    }
    pub fn retry_failed(&mut self) {
        if matches!(self, Self::Failed { .. } | Self::Cancelled { .. }) {
            *self = Self::Idle;
        }
    }
}

type Resource = Box<dyn FnOnce() -> Result<(), Failure> + Send>;
#[derive(Default)]
struct Cancellation {
    cancelled: AtomicBool,
    resource: Mutex<Option<Resource>>,
}
#[derive(Clone)]
pub struct CancelToken(Arc<Cancellation>);
fn invoke(resource: Resource) -> Result<(), Failure> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(resource)) {
        Ok(result) => result,
        Err(_) => Err(Failure::new(
            FailureKind::Panic,
            "cancellation resource panicked",
        )),
    }
}
impl CancelToken {
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    /// Install the single optional resource hook before acquiring native resources.
    /// A late installation executes immediately and returns its failure to the
    /// installer. Hooks must be prompt: they must not join or wait for workers.
    /// A resource published after cancellation must also be revoked by its owner.
    pub fn register_cancel_resource(
        &self,
        resource: impl FnOnce() -> Result<(), Failure> + Send + 'static,
    ) -> Result<(), Failure> {
        let resource: Resource = Box::new(resource);
        {
            let mut slot = self.0.resource.lock();
            if !self.is_cancelled() {
                if slot.is_some() {
                    return Err(Failure::new(
                        FailureKind::Protocol,
                        "cancellation resource already registered",
                    ));
                }
                *slot = Some(resource);
                return Ok(());
            }
        }
        invoke(resource)
    }

    /// Call only after normal resource cleanup. An already detached callback
    /// may still run; the resource itself must serialize revocation and reuse.
    pub fn clear_cancel_resource(&self) {
        let resource = self.0.resource.lock().take();
        drop(resource);
    }

    fn cancel_resource(&self) -> Result<(), Failure> {
        let resource = {
            let mut slot = self.0.resource.lock();
            self.0.cancelled.store(true, Ordering::Release);
            slot.take()
        };
        match resource {
            Some(resource) => invoke(resource),
            None => Ok(()),
        }
    }
}
type Emitter<T> = Arc<Mutex<Option<Box<dyn FnOnce(Outcome<T>) + Send>>>>;
fn finish<T>(emitter: &Emitter<T>, outcome: Outcome<T>) {
    let emit = emitter.lock().take();
    if let Some(emit) = emit {
        emit(outcome);
    }
}
pub struct CancelHandle {
    cancel: Option<Box<dyn FnOnce(CancelReason) + Send>>,
}
impl CancelHandle {
    pub fn cancel(mut self, reason: CancelReason) {
        if let Some(cancel) = self.cancel.take() {
            cancel(reason);
        }
    }
}
impl Drop for CancelHandle {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel(CancelReason::OwnerClosed);
        }
    }
}
/// A registered work envelope without an eagerly spawned thread.
/// Retain a cancellation handle until it runs or is explicitly cancelled.
#[must_use]
pub struct PreparedWork<T> {
    token: CancelToken,
    emitter: Emitter<T>,
}

pub fn prepare<T: Send + 'static>(
    emit: impl FnOnce(Outcome<T>) + Send + 'static,
) -> PreparedWork<T> {
    PreparedWork {
        token: CancelToken(Arc::new(Cancellation::default())),
        emitter: Arc::new(Mutex::new(Some(Box::new(emit)))),
    }
}

impl<T: Send + 'static> PreparedWork<T> {
    /// Additional guards share one terminal reservation; dropping any live guard
    /// cancels the work. A scheduler can retain its own guard while the UI owns one.
    pub fn cancel_handle(&self) -> CancelHandle {
        let cancel_token = self.token.clone();
        let cancel_emitter = self.emitter.clone();
        CancelHandle {
            cancel: Some(Box::new(move |reason| {
                let emit = cancel_emitter.lock().take();
                if let Some(emit) = emit {
                    let outcome = match cancel_token.cancel_resource() {
                        Ok(()) => Outcome::Cancelled(reason),
                        Err(failure) => Outcome::Failed {
                            failure,
                            partial: None,
                        },
                    };
                    emit(outcome);
                }
            })),
        }
    }

    pub fn run(self, work: impl FnOnce(CancelToken) -> Outcome<T>) {
        if self.token.is_cancelled() {
            return;
        }
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            work(self.token.clone())
        })) {
            Ok(outcome) => outcome,
            Err(_) => match self.token.cancel_resource() {
                Ok(()) => Outcome::failed(FailureKind::Panic, "worker panicked"),
                Err(failure) => Outcome::Failed {
                    failure,
                    partial: None,
                },
            },
        };
        self.token.clear_cancel_resource();
        finish(&self.emitter, outcome);
    }
}

pub fn spawn<T: Send + 'static>(
    name: &'static str,
    emit: impl FnOnce(Outcome<T>) + Send + 'static,
    work: impl FnOnce(CancelToken) -> Outcome<T> + Send + 'static,
) -> CancelHandle {
    let prepared = prepare(emit);
    let handle = prepared.cancel_handle();
    let emitter = prepared.emitter.clone();
    if let Err(error) = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || prepared.run(work))
    {
        finish(
            &emitter,
            Outcome::failed(FailureKind::ThreadStart, error.to_string()),
        );
    }
    handle
}

/// Scoped readers use the same envelope, and their scope joins them before a
/// provider can release physical ownership or start its successor.
pub fn spawn_scoped<'scope, 'env, T: Send + 'static>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    name: &'static str,
    emit: impl FnOnce(Outcome<T>) + Send + 'static,
    work: impl FnOnce(CancelToken) -> Outcome<T> + Send + 'scope,
) -> CancelHandle {
    let prepared = prepare(emit);
    let handle = prepared.cancel_handle();
    let emitter = prepared.emitter.clone();
    if let Err(error) = std::thread::Builder::new()
        .name(name.into())
        .spawn_scoped(scope, move || prepared.run(work))
    {
        finish(
            &emitter,
            Outcome::failed(FailureKind::ThreadStart, error.to_string()),
        );
    }
    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn cancellation_reserves_terminal_before_callback_and_worker_success() {
        let (ready_tx, ready_rx) = channel();
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel();
        let (work_tx, work_rx) = channel();
        let (done_tx, done_rx) = channel();
        let (tx, rx) = channel();
        let handle = spawn(
            "race",
            move |result| {
                tx.send(result).unwrap();
            },
            move |token| {
                token
                    .register_cancel_resource(move || {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Err(Failure::new(FailureKind::Io, "cleanup failed"))
                    })
                    .unwrap();
                ready_tx.send(()).unwrap();
                work_rx.recv().unwrap();
                done_tx.send(()).unwrap();
                Outcome::Success(())
            },
        );
        ready_rx.recv().unwrap();
        let cancel = std::thread::spawn(move || handle.cancel(CancelReason::Dismissed));
        entered_rx.recv().unwrap();
        work_tx.send(()).unwrap();
        done_rx.recv().unwrap();
        assert!(rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        cancel.join().unwrap();
        assert!(
            matches!(rx.recv().unwrap(), Outcome::Failed { failure, .. } if failure.kind == FailureKind::Io)
        );
        assert!(rx.recv().is_err());
    }

    #[test]
    fn late_registration_runs_immediately_and_success_wins_when_already_published() {
        let token = CancelToken(Arc::new(Cancellation::default()));
        token.cancel_resource().unwrap();
        let (tx, rx) = channel();
        token
            .register_cancel_resource(move || {
                tx.send(()).unwrap();
                Ok(())
            })
            .unwrap();
        rx.recv().unwrap();
        let (tx, rx) = channel();
        let handle = spawn(
            "success",
            move |result| {
                tx.send(result).unwrap();
            },
            |_| Outcome::Success(7),
        );
        assert!(matches!(rx.recv().unwrap(), Outcome::Success(7)));
        drop(handle);
        assert!(rx.recv().is_err());
    }

    #[test]
    fn queued_cancellation_prevents_work_from_starting() {
        let (tx, rx) = channel();
        let prepared: PreparedWork<()> = prepare(move |outcome| {
            tx.send(outcome).unwrap();
        });
        prepared.cancel_handle().cancel(CancelReason::Superseded);
        prepared.run(|_| panic!("cancelled queued work executed"));
        assert!(matches!(
            rx.recv().unwrap(),
            Outcome::Cancelled(CancelReason::Superseded)
        ));
        assert!(rx.recv().is_err());
    }

    #[test]
    fn scoped_worker_cancellation_does_not_skip_physical_join() {
        let (control_tx, control_rx) = channel();
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel();
        let (exited_tx, exited_rx) = channel();
        let (tx, rx) = channel();
        let scope_owner = std::thread::spawn(move || {
            std::thread::scope(|scope| {
                let handle = spawn_scoped(
                    scope,
                    "scoped-reader",
                    move |outcome| {
                        tx.send(outcome).unwrap();
                    },
                    move |_| {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Outcome::Success(())
                    },
                );
                assert!(control_tx.send(handle).is_ok());
            });
            exited_tx.send(()).unwrap();
        });
        let handle = control_rx.recv().unwrap();
        entered_rx.recv().unwrap();
        handle.cancel(CancelReason::Dismissed);
        assert!(matches!(
            rx.recv().unwrap(),
            Outcome::Cancelled(CancelReason::Dismissed)
        ));
        assert!(exited_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        exited_rx.recv().unwrap();
        scope_owner.join().unwrap();
    }
}
