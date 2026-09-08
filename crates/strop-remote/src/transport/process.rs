//! Physical-connection lifecycle for the pooled actors: spawn the dedicated
//! ssh(1) sftp subsystem connection, convert its pipes into a private
//! current-thread Tokio runtime, negotiate the session under the job's
//! cancellation, and reap the child on every exit path — failure, deadline,
//! cancellation and panic unwinding included. One actor owns a
//! [`Physical`] at a time; nothing here is ever visible to the editor.

use super::error::{Fault, ReadFailureKind, ReadStage, RemoteReadError};
use super::session;
use super::stderr;
use super::wire::ReadOnlySftp;
use crate::pool::StopSignal;
use std::cell::Cell;
use std::process::Command;
use std::time::Duration;
use strop_core::process::OwnedProcess;
use strop_core::worker::CancelToken;
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/// Bound on the final stderr drain once the child is dead or dying.
const STDERR_DRAIN: Duration = Duration::from_secs(1);

/// One live SSH/SFTP connection: the actor's entire process ownership.
pub(crate) struct Physical {
    /// Monotonic within the actor; every connection attempt — including
    /// failures during authentication — allocates the next one.
    pub(crate) epoch: u64,
    pub(crate) runtime: Runtime,
    pub(crate) child: OwnedProcess,
    pub(crate) codec: ReadOnlySftp<ChildStdin, ChildStdout>,
    log: stderr::SharedTail,
    pub(crate) drain: Option<JoinHandle<()>>,
    pub(crate) advertised: super::wire::Advertised,
}

impl Physical {
    pub(crate) fn diagnostics(&self) -> Option<String> {
        self.log.lock().render()
    }
    /// Establish one physical connection. `command` comes from the single
    /// crate SSH policy; `token` is the job that asked for it, so its
    /// cancellation hook can kill the group promptly while the negotiation
    /// is in flight. The actor clears that hook once the job finishes so a
    /// later cancellation cannot kill a connection now serving other jobs.
    pub(crate) fn connect(
        command: &mut Command,
        token: &CancelToken,
        stop: &StopSignal,
        epoch: u64,
        remote: &str,
    ) -> Result<Self, RemoteReadError> {
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
                .remote(remote))
            }
        };
        let mut child = match OwnedProcess::spawn(command, token) {
            Ok(child) => child,
            Err(failure) => {
                let kind = if token.is_cancelled() {
                    ReadFailureKind::Cancelled
                } else {
                    ReadFailureKind::Spawn
                };
                return Err(
                    RemoteReadError::bare(ReadStage::Spawn, kind, failure.message).remote(remote),
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
            Err(fault) => return Err(diagnose(runtime, child, log, None, fault, remote)),
        };
        let stage = Cell::new(ReadStage::Connect);
        let negotiated = runtime.block_on(session::guarded(
            ReadOnlySftp::connect(stdin, stdout),
            token,
            stop,
            &stage,
        ));
        match negotiated {
            Ok((codec, advertised)) => Ok(Self {
                epoch,
                runtime,
                child,
                codec,
                log,
                drain: Some(drain),
                advertised,
            }),
            Err(fault) => Err(diagnose(runtime, child, log, Some(drain), fault, remote)),
        }
    }

    /// Tear the connection down: kill the process group, reap the leader
    /// (revoke-before-reap), give stderr a bounded final drain, then drop
    /// codec and runtime so the pipes are released. Never blocks unbounded.
    pub(crate) fn retire(mut self) -> Result<(), RemoteReadError> {
        self.child.terminate().map_err(|failure| {
            RemoteReadError::bare(ReadStage::Teardown, ReadFailureKind::Io, failure.message)
        })?;
        self.child.wait().map_err(|failure| {
            RemoteReadError::bare(ReadStage::Teardown, ReadFailureKind::Io, failure.message)
        })?;
        // The drain task completes at EOF — when ssh and its local
        // descendants are gone — so a killed group needs only the bounded
        // grace; an already-finished handle returns immediately.
        if let Some(drain) = self.drain.take() {
            self.runtime
                .block_on(async { tokio::time::timeout(STDERR_DRAIN, drain).await })
                .map_err(|error| {
                    RemoteReadError::bare(
                        ReadStage::Teardown,
                        ReadFailureKind::Deadline,
                        error.to_string(),
                    )
                })?
                .map_err(|error| {
                    RemoteReadError::bare(
                        ReadStage::Teardown,
                        ReadFailureKind::Io,
                        error.to_string(),
                    )
                })?;
        }
        Ok(())
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
    let stdin = ChildStdin::from_std(stdin_pipe).map_err(|error| rejected("ssh stdin", error))?;
    let stdout_pipe = child.take_stdout().ok_or_else(|| missing("ssh stdout"))?;
    let stdout =
        ChildStdout::from_std(stdout_pipe).map_err(|error| rejected("ssh stdout", error))?;
    Ok((stdin, stdout, drain))
}

/// Failure exit: the child is dying regardless. Kill or reap it, give
/// stderr a bounded final drain, then assemble the typed error with
/// every diagnostic that survived.
fn diagnose(
    runtime: Runtime,
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
    drop(child);
    drop(runtime);
    let mut error = RemoteReadError::fault(remote, fault)
        .stderr(log.lock().render())
        .exit(exit);
    if error.kind() == ReadFailureKind::Connect {
        error.refine_connect();
    }
    error
}
