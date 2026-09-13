use super::*;
use crate::worker::CancelReason;
use std::sync::mpsc::channel;

#[test]
fn cancellation_waits_for_the_actual_committed_effect_receipt() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("effect");
    let target = file.clone();
    let (ready, entered) = channel();
    let (release, wait) = channel();
    let (tx, rx) = channel();
    let handle = spawn_effect(
        "observed-effect",
        move |outcome| {
            tx.send(outcome).unwrap();
        },
        move |_| {
            std::fs::write(&target, "committed").unwrap();
            ready.send(()).unwrap();
            wait.recv().unwrap();
            Outcome::Success(std::fs::metadata(target).unwrap().len())
        },
    );
    entered.recv().unwrap();
    handle.cancel(CancelReason::Dismissed);
    assert!(
        rx.try_recv().is_err(),
        "cancellation must not fabricate an early terminal result"
    );
    release.send(()).unwrap();
    assert!(matches!(rx.recv().unwrap(), Outcome::Success(9)));
    assert_eq!(std::fs::read(file).unwrap(), b"committed");
}

#[test]
fn cancellation_cleanup_failure_preserves_the_observed_value() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("effect");
    let target = file.clone();
    let (ready, entered) = channel();
    let (cleanup_started, cleanup_entered) = channel();
    let (cleanup_release, cleanup_wait) = channel();
    let (work_release, work_wait) = channel();
    let (tx, rx) = channel();
    let handle = spawn_effect(
        "effect-cleanup",
        move |outcome| {
            tx.send(outcome).unwrap();
        },
        move |token| {
            token
                .register_cancel_resource(move || {
                    cleanup_started.send(()).unwrap();
                    cleanup_wait.recv().unwrap();
                    Err(Failure::new(FailureKind::Io, "injected cleanup failure"))
                })
                .unwrap();
            std::fs::write(&target, "committed").unwrap();
            ready.send(()).unwrap();
            work_wait.recv().unwrap();
            Outcome::Success(std::fs::metadata(target).unwrap().len())
        },
    );
    entered.recv().unwrap();
    let cancelling = std::thread::spawn(move || handle.cancel(CancelReason::Dismissed));
    cleanup_entered.recv().unwrap();
    work_release.send(()).unwrap();
    assert!(rx.try_recv().is_err());
    cleanup_release.send(()).unwrap();
    cancelling.join().unwrap();
    assert!(
        matches!(rx.recv().unwrap(), Outcome::Failed { failure, partial: Some(9) } if failure.kind == FailureKind::Io)
    );
    assert_eq!(std::fs::read(file).unwrap(), b"committed");
}
