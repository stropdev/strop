//! Loom campaign over the worker envelope (0108 D3): the job finishing
//! races its owner cancelling, over the real `prepare` / `cancel_handle`
//! / `run` control flow with loom's instrumented Mutex/AtomicBool/Arc.
//!
//! For every interleaving: exactly one outcome is emitted, and a
//! registered cancellation resource is disposed of exactly once —
//! either revoked (its hook runs) or retired (normal cleanup drops the
//! hook unrun), never both and never twice. A success is never revoked,
//! and a revocation that fails is what the owner hears.

use super::*;
use loom::sync::atomic::AtomicUsize;

/// How the owner gives up on the job.
#[derive(Clone, Copy)]
enum Owner {
    Cancels,
    DropsHandle,
}

/// What the resource's revocation hook reports.
#[derive(Clone, Copy)]
enum Revocation {
    Clean,
    Fails,
}

#[derive(Default)]
struct Ledger {
    registered: AtomicUsize,
    revoked: AtomicUsize,
    retired: AtomicUsize,
}

/// The cancellation resource: its hook either runs (revoked) or is
/// dropped unrun (retired).
struct Resource {
    ledger: Arc<Ledger>,
    fired: bool,
}

impl Drop for Resource {
    fn drop(&mut self) {
        if !self.fired {
            self.ledger.retired.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn campaign(owner: Owner, revocation: Revocation) {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(3);
    builder.check(move || {
        let ledger = Arc::new(Ledger::default());
        let emitted: Arc<Mutex<Vec<Outcome<()>>>> = Arc::new(Mutex::new(Vec::new()));

        let sink = emitted.clone();
        let prepared = prepare(move |outcome| lock(&sink).push(outcome));
        let handle = prepared.cancel_handle();

        let work_ledger = ledger.clone();
        let worker = loom::thread::spawn(move || {
            prepared.run(move |token| {
                let mut resource = Resource {
                    ledger: work_ledger.clone(),
                    fired: false,
                };
                work_ledger.registered.fetch_add(1, Ordering::SeqCst);
                let registration = token.register_cancel_resource(move || {
                    resource.fired = true;
                    resource.ledger.revoked.fetch_add(1, Ordering::SeqCst);
                    match revocation {
                        Revocation::Clean => Ok(()),
                        Revocation::Fails => Err(Failure::new(FailureKind::Io, "revoke failed")),
                    }
                });
                match registration {
                    Ok(()) => Outcome::Success(()),
                    Err(failure) => Outcome::Failed {
                        failure,
                        partial: None,
                    },
                }
            })
        });
        match owner {
            Owner::Cancels => handle.cancel(CancelReason::Dismissed),
            Owner::DropsHandle => drop(handle),
        }
        worker.join().unwrap();

        let emitted = lock(&emitted);
        assert_eq!(emitted.len(), 1, "exactly one outcome per job");
        let registered = ledger.registered.load(Ordering::SeqCst);
        let revoked = ledger.revoked.load(Ordering::SeqCst);
        let retired = ledger.retired.load(Ordering::SeqCst);
        assert!(registered <= 1);
        assert_eq!(
            revoked + retired,
            registered,
            "a registered resource is disposed of exactly once"
        );
        let reason = match owner {
            Owner::Cancels => CancelReason::Dismissed,
            Owner::DropsHandle => CancelReason::OwnerClosed,
        };
        match &emitted[0] {
            Outcome::Success(()) => assert_eq!(revoked, 0, "a success is never revoked"),
            Outcome::Cancelled(cancelled) => assert_eq!(*cancelled, reason),
            Outcome::Failed { failure, .. } => {
                assert!(matches!(revocation, Revocation::Fails));
                assert_eq!(failure.kind, FailureKind::Io);
                assert_eq!(revoked, 1, "only a revocation can fail here");
            }
        }
        if matches!(revocation, Revocation::Clean) {
            assert!(!matches!(emitted[0], Outcome::Failed { .. }));
        }
    });
}

#[test]
fn loom_worker_cancel_races_finish_exactly_once() {
    campaign(Owner::Cancels, Revocation::Clean);
}

#[test]
fn loom_worker_dropped_handle_races_finish_with_failing_revocation() {
    campaign(Owner::DropsHandle, Revocation::Fails);
}
