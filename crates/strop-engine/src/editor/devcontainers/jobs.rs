use super::model::{OperationError, OperationKey};
use super::WorkspaceKey;
use std::sync::mpsc::Sender;
use strop_containers::devcontainer::{DiscoveryReport, JobOutcome, OwnedJob};
use strop_core::worker::CancelToken;

pub(crate) enum InternalEvent {
    Discovery {
        workspace: WorkspaceKey,
        incarnation: u64,
        result: Result<DiscoveryReport, OperationError>,
    },
    Operation {
        key: OperationKey,
        result: Box<Result<InternalOperationOutcome, OperationError>>,
    },
}

pub(crate) enum InternalOperationOutcome {
    Probe(strop_containers::devcontainer::ReferenceCli),
    Read {
        configuration: strop_containers::devcontainer::CliConfiguration,
    },
    Lifecycle(Box<JobOutcome>),
}

/// Run an explicit CLI/read/discovery job to actual settlement. Cancellation
/// only affects the token; no synthetic completion races the owned work.
pub(crate) fn spawn<T>(
    name: &'static str,
    sender: Sender<InternalEvent>,
    wake: Sender<()>,
    token: CancelToken,
    event: impl FnOnce(Result<T, OperationError>) -> InternalEvent + Send + 'static,
    work: impl FnOnce(&CancelToken) -> Result<T, OperationError> + Send + 'static,
) -> Result<(), OperationError>
where
    T: Send + 'static,
{
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let mut result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&token)))
                    .unwrap_or(Err(OperationError::WorkerPanic));
            if token.is_cancelled() {
                result = Err(OperationError::Cancelled);
            }
            let _ = sender.send(event(result));
            let _ = wake.send(());
        })
        .map(|_| ())
        .map_err(|error| OperationError::ThreadStart(error.to_string()))
}

/// OwnedJob has its own cancellation token and terminal outcome. In particular,
/// a cancelled successful `up` must still be parsed for partial-resource proof.
pub(crate) fn spawn_owned(
    sender: Sender<InternalEvent>,
    wake: Sender<()>,
    key: OperationKey,
    job: OwnedJob,
) -> Result<(), OperationError> {
    std::thread::Builder::new()
        .name("devcontainer-owned-job".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run()))
                .map(|outcome| InternalOperationOutcome::Lifecycle(Box::new(outcome)))
                .map_err(|_| OperationError::WorkerPanic);
            let _ = sender.send(InternalEvent::Operation {
                key,
                result: Box::new(result),
            });
            let _ = wake.send(());
        })
        .map(|_| ())
        .map_err(|error| OperationError::ThreadStart(error.to_string()))
}
