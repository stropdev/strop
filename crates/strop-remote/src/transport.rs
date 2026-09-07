//! Worker-side orchestration of one remote read: spawn a dedicated ssh(1)
//! connection to the `sftp` subsystem, drive the SFTP session on a private
//! current-thread Tokio runtime, and reap the child on every exit path —
//! failure, deadline, cancellation and panic unwinding included. The only
//! values that escape are a `Buffer` or a typed error; no async type, pipe
//! or process is ever visible to the editor, so input and render never
//! touch I/O.

mod error;

pub use error::{ReadFailureKind, ReadStage, RemoteReadError};

#[cfg(unix)]
mod command;
#[cfg(unix)]
mod session;
#[cfg(unix)]
mod stderr;
#[cfg(unix)]
mod wire;

use super::address::RemoteFile;
use strop_core::worker::CancelToken;
use strop_core::Buffer;

/// Read one remote file into an in-memory read-only buffer.
///
/// Must run on a worker (never the editor thread): it blocks for the
/// duration of the transfer, bounded by the session deadline plus bounded
/// cleanup. Cancellation kills the ssh process group promptly through the
/// token hook owned by the child.
pub fn read(file: &RemoteFile, token: &CancelToken) -> Result<Buffer, RemoteReadError> {
    #[cfg(not(unix))]
    {
        let _ = (file, token);
        return Err(RemoteReadError::bare(
            ReadStage::Spawn,
            ReadFailureKind::Unsupported,
            "remote reads require Unix process supervision",
        ));
    }
    #[cfg(unix)]
    unix::read(file, token)
}

#[cfg(unix)]
mod unix {
    use super::super::address::RemoteFile;
    use super::error::{Fault, ReadFailureKind, ReadStage, RemoteReadError};
    use super::{command, session, stderr, Buffer, CancelToken};
    use std::time::{Duration, Instant};
    use strop_core::process::OwnedProcess;
    use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
    use tokio::runtime::Runtime;
    use tokio::task::JoinHandle;

    /// Grace for a well-behaved ssh to exit once the session has closed
    /// its pipes; after this the group is killed and reaped.
    const REAP_GRACE: Duration = Duration::from_secs(5);
    /// Bound on the final stderr drain once the child is dead or dying.
    const STDERR_DRAIN: Duration = Duration::from_secs(1);
    /// Poll cadence for exit observation between sleeps.
    const POLL: Duration = Duration::from_millis(20);

    pub(super) fn read(file: &RemoteFile, token: &CancelToken) -> Result<Buffer, RemoteReadError> {
        if token.is_cancelled() {
            return Err(RemoteReadError::bare(
                ReadStage::Spawn,
                ReadFailureKind::Cancelled,
                "the read was cancelled before the ssh client started",
            ));
        }
        let remote = file.to_string();
        let mut command = command::sftp_subsystem(file);
        // The runtime exists before the child: pipe registration happens
        // under `enter()` so the runtime's IO driver owns every pipe, and
        // dropping the runtime — on any path — is what releases them.
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                return Err(RemoteReadError::bare(
                    ReadStage::Spawn,
                    ReadFailureKind::Io,
                    format!("private tokio runtime: {error}"),
                )
                .remote(&remote))
            }
        };
        let mut child = match OwnedProcess::spawn(&mut command, token) {
            Ok(child) => child,
            Err(failure) => {
                let kind = if token.is_cancelled() {
                    ReadFailureKind::Cancelled
                } else {
                    ReadFailureKind::Spawn
                };
                return Err(
                    RemoteReadError::bare(ReadStage::Spawn, kind, failure.message).remote(&remote),
                );
            }
        };
        let log = stderr::SharedTail::default();
        let pipes = {
            let _context = runtime.enter();
            convert(&mut child, log.clone())
        };
        let (stdin, stdout, drain) = match pipes {
            Ok(pipes) => pipes,
            Err(fault) => return Err(read_failure(&runtime, child, log, None, fault, &remote)),
        };
        match runtime.block_on(session::run(stdin, stdout, file.path(), token)) {
            Ok(text) => {
                // The session closed its pipes inside the deadline; reap
                // the child, killing the group only if ssh lingers.
                if let Err(fault) = runtime.block_on(reap_after_shutdown(&mut child)) {
                    return Err(read_failure(
                        &runtime,
                        child,
                        log,
                        Some(drain),
                        fault,
                        &remote,
                    ));
                }
                drop(runtime);
                // Success is published only if no cancellation reserved
                // the outcome while the runtime ran; a stale read never
                // wins against its owner's cancel.
                if token.is_cancelled() {
                    return Err(RemoteReadError::fault(
                        &remote,
                        Fault::cancelled(ReadStage::Teardown),
                    ));
                }
                let mut buffer = Buffer::from_text(&text);
                // A remote snapshot is never writable; provenance beyond
                // this flag is the caller's document identity.
                buffer.readonly = true;
                Ok(buffer)
            }
            Err(fault) => Err(read_failure(
                &runtime,
                child,
                log,
                Some(drain),
                fault,
                &remote,
            )),
        }
    }

    /// Convert the child's pipes to the runtime's IO driver. Stderr is
    /// converted first so its concurrent drain exists for every later
    /// failure, including stdin/stdout conversion errors.
    fn convert(
        child: &mut OwnedProcess,
        log: stderr::SharedTail,
    ) -> Result<(ChildStdin, ChildStdout, JoinHandle<()>), Fault> {
        let missing = |what: &str| {
            Fault::new(
                ReadStage::Connect,
                ReadFailureKind::Io,
                format!("{what} pipe missing"),
            )
        };
        let rejected = |what: &str, error: std::io::Error| {
            Fault::new(
                ReadStage::Connect,
                ReadFailureKind::Io,
                format!("{what}: {error}"),
            )
        };
        let stderr_pipe = child.take_stderr().ok_or_else(|| missing("ssh stderr"))?;
        let stderr_pipe =
            ChildStderr::from_std(stderr_pipe).map_err(|error| rejected("ssh stderr", error))?;
        let drain = stderr::spawn_drain(stderr_pipe, log);
        let stdin_pipe = child.take_stdin().ok_or_else(|| missing("ssh stdin"))?;
        let stdin =
            ChildStdin::from_std(stdin_pipe).map_err(|error| rejected("ssh stdin", error))?;
        let stdout_pipe = child.take_stdout().ok_or_else(|| missing("ssh stdout"))?;
        let stdout =
            ChildStdout::from_std(stdout_pipe).map_err(|error| rejected("ssh stdout", error))?;
        Ok((stdin, stdout, drain))
    }

    /// Failure exit: the child is dying regardless. Kill or reap it, give
    /// stderr a bounded final drain, then assemble the typed error with
    /// every diagnostic that survived.
    fn read_failure(
        runtime: &Runtime,
        mut child: OwnedProcess,
        log: stderr::SharedTail,
        drain: Option<JoinHandle<()>>,
        fault: Fault,
        remote: &str,
    ) -> RemoteReadError {
        // A child that already failed on its own keeps its real exit line;
        // a live one is terminated first — `wait` only follows a signal or
        // an observed exit, never a live child.
        let exited = child.has_exited().unwrap_or(false);
        let signalled = child.terminate().is_ok();
        let exit = if exited || signalled {
            child.wait().ok().map(|status| status.to_string())
        } else {
            // OwnedProcess::drop still terminates and reaps.
            None
        };
        if let Some(drain) = drain {
            // EOF follows death; this only decides how much trailing
            // stderr is worth one more bounded second.
            let _ = runtime.block_on(async { tokio::time::timeout(STDERR_DRAIN, drain).await });
        }
        let mut error = RemoteReadError::fault(remote, fault)
            .stderr(log.lock().render())
            .exit(exit);
        if error.kind() == ReadFailureKind::Connect {
            error.refine_connect();
        }
        error
    }

    /// Drive stderr while waiting for exit. Success requires a clean child exit;
    /// the unreaped leader reserves the group until descendants are terminated.
    async fn reap_after_shutdown(child: &mut OwnedProcess) -> Result<(), Fault> {
        let start = Instant::now();
        loop {
            let failed = |failure: strop_core::worker::Failure| {
                Fault::new(ReadStage::Teardown, ReadFailureKind::Io, failure.message)
            };
            if child.has_exited().map_err(failed)? {
                child.terminate().map_err(failed)?;
                let status = child.wait().map_err(failed)?;
                return if status.success() {
                    Ok(())
                } else {
                    Err(Fault::new(
                        ReadStage::Teardown,
                        ReadFailureKind::Io,
                        format!("ssh {status}"),
                    ))
                };
            }
            if start.elapsed() >= REAP_GRACE {
                return Err(Fault::new(
                    ReadStage::Teardown,
                    ReadFailureKind::Deadline,
                    "ssh did not exit after SFTP shutdown",
                ));
            }
            tokio::time::sleep(POLL).await;
        }
    }
}
