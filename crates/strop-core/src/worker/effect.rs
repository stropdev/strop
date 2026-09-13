//! Admitted effects finish with their observed result, not an eager cancel fiction.
use super::{CancelHandle, CancelToken, Cancellation, Failure, FailureKind, Outcome};
use parking_lot::Mutex;
use std::sync::Arc;

type Emit<T> = Box<dyn FnOnce(Outcome<T>) + Send>;
struct State<T> {
    emit: Option<Emit<T>>,
    outcome: Option<Outcome<T>>,
    cancelling: bool,
    cleanup_error: Option<Failure>,
}
impl<T> State<T> {
    fn ready(&mut self) -> Option<(Emit<T>, Outcome<T>)> {
        if self.cancelling || self.outcome.is_none() {
            return None;
        }
        let emit = self.emit.take()?;
        let outcome = self.outcome.take()?;
        let outcome = match (outcome, self.cleanup_error.take()) {
            (Outcome::Success(value), Some(failure)) => Outcome::Failed {
                failure,
                partial: Some(value),
            },
            (
                Outcome::Failed {
                    mut failure,
                    partial,
                },
                Some(cleanup),
            ) => {
                failure
                    .message
                    .push_str(&format!("; cancellation cleanup: {}", cleanup.message));
                Outcome::Failed { failure, partial }
            }
            (Outcome::Cancelled(_), Some(failure)) => Outcome::Failed {
                failure,
                partial: None,
            },
            (outcome, None) => outcome,
        };
        Some((emit, outcome))
    }
}
struct Shared<T> {
    token: CancelToken,
    state: Mutex<State<T>>,
}
impl<T> Shared<T> {
    fn complete(&self, outcome: Outcome<T>) {
        let delivery = {
            let mut state = self.state.lock();
            state.outcome = Some(outcome);
            state.ready()
        };
        if let Some((emit, outcome)) = delivery {
            emit(outcome);
        }
    }
    fn request_cancel(&self) {
        {
            let mut state = self.state.lock();
            if state.emit.is_none() || state.cancelling || self.token.is_cancelled() {
                return;
            }
            state.cancelling = true;
        }
        let failure = self.token.cancel_resource().err();
        let delivery = {
            let mut state = self.state.lock();
            state.cancelling = false;
            state.cleanup_error = failure;
            state.ready()
        };
        if let Some((emit, outcome)) = delivery {
            emit(outcome);
        }
    }
}

/// Cancellation requests stop resources promptly but do not replace a mutation
/// receipt. The work closure always runs and must check its token before effects,
/// allowing it to return exact per-item cancellations even before native launch.
/// Cleanup failure preserves a successful observed value in `Failed.partial`.
pub fn spawn_effect<T: Send + 'static>(
    name: &'static str,
    emit: impl FnOnce(Outcome<T>) + Send + 'static,
    work: impl FnOnce(CancelToken) -> Outcome<T> + Send + 'static,
) -> CancelHandle {
    let shared = Arc::new(Shared {
        token: CancelToken(Arc::new(Cancellation::default())),
        state: Mutex::new(State {
            emit: Some(Box::new(emit)),
            outcome: None,
            cancelling: false,
            cleanup_error: None,
        }),
    });
    let cancellation = shared.clone();
    let handle = CancelHandle {
        cancel: Some(Box::new(move |_| cancellation.request_cancel())),
    };
    let worker = shared.clone();
    let started = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                work(worker.token.clone())
            })) {
                Ok(outcome) => outcome,
                Err(_) => match worker.token.cancel_resource() {
                    Ok(()) => Outcome::failed(FailureKind::Panic, "effect worker panicked"),
                    Err(failure) => Outcome::Failed {
                        failure,
                        partial: None,
                    },
                },
            };
            worker.token.clear_cancel_resource();
            worker.complete(outcome);
        });
    if let Err(error) = started {
        shared.complete(Outcome::failed(FailureKind::ThreadStart, error.to_string()));
    }
    handle
}

#[cfg(test)]
mod tests;
