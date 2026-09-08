//! Tests enter the same worker ownership boundary as production callers.
use strop_core::worker::{self, CancelToken, Outcome};
pub(crate) fn in_worker<T: Send + 'static>(
    work: impl FnOnce(CancelToken) -> T + Send + 'static,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let owner = worker::spawn(
        "remote-oracle",
        move |outcome| {
            let _ = tx.send(outcome);
        },
        move |token| Outcome::Success(work(token)),
    );
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(40))
        .expect("remote oracle must complete");
    drop(owner);
    match result {
        Outcome::Success(value) => value,
        Outcome::Failed { failure, .. } => panic!("remote oracle failed: {}", failure.message),
        Outcome::Cancelled(reason) => panic!("remote oracle cancelled: {reason:?}"),
    }
}
