//! Stream remote stdout without retaining it; keep the same supervised stdin lease.
use super::{RemoteCommand, RemoteCommandError, RemoteExitStatus, StdinMode};
use strop_core::process::{self, StreamError, StreamPolicy};
use strop_core::worker::CancelToken;
use strop_workspace::RemoteEndpoint;

pub struct RemoteStreamOutput {
    pub status: RemoteExitStatus,
    pub stderr: Vec<u8>,
    pub stderr_dropped: u64,
}
#[derive(Debug)]
pub enum RemoteStreamError<E> {
    Remote(RemoteCommandError),
    Consumer(E),
}
impl<E> From<RemoteCommandError> for RemoteStreamError<E> {
    fn from(error: RemoteCommandError) -> Self {
        Self::Remote(error)
    }
}

pub fn stream<E>(
    endpoint: &RemoteEndpoint,
    command: &RemoteCommand,
    token: &CancelToken,
    consume: impl FnMut(&[u8]) -> Result<(), E>,
) -> Result<RemoteStreamOutput, RemoteStreamError<E>> {
    let (mut process, key) = super::run::supervised(endpoint, command, StdinMode::Finite)?;
    let output = process::stream_with(
        &mut process,
        token,
        &StreamPolicy {
            stderr_limit: super::run::STDERR_LIMIT,
            stderr_tail: super::run::STDERR_TAIL,
            deadline: command.deadline(),
            hold_stdin: true,
        },
        consume,
    )
    .map_err(|error| match error {
        StreamError::Consumer(error) => RemoteStreamError::Consumer(error),
        StreamError::Spawn(message) => {
            RemoteStreamError::Remote(RemoteCommandError::Spawn { message })
        }
        StreamError::Cancelled => RemoteStreamError::Remote(RemoteCommandError::Cancelled {
            diagnostics: "stream cancellation observed".into(),
        }),
        StreamError::TimedOut(deadline) => RemoteStreamError::Remote(RemoteCommandError::Timeout {
            seconds: deadline.as_secs(),
        }),
        StreamError::Failure(error) => RemoteStreamError::Remote(RemoteCommandError::Local {
            message: error.message,
        }),
    })?;
    // The shared classifier consumes status and bounded stderr. Stdout has already
    // been delivered to the caller and is deliberately absent from this result API.
    let output = super::run::classify(
        process::CommandOutput {
            status: output.status,
            stdout: Vec::new(),
            stdout_dropped: 0,
            stderr: output.stderr,
            stderr_dropped: output.stderr_dropped,
            stdin_error: None,
        },
        &key,
    )?;
    Ok(RemoteStreamOutput {
        status: output.status,
        stderr: output.stderr,
        stderr_dropped: output.stderr_dropped,
    })
}
