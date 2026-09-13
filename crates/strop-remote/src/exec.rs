//! Supervised remote process execution for Git commands and language
//! servers (0036 "Remote execution and services").
//!
//! One [`RemoteCommand`] is a pure, cloned description: executable,
//! native argv and absolute remote cwd. Filenames never become
//! executable shell text — they travel base64-encoded inside a fixed
//! Python supervisor's spec and are executed remotely through
//! `os.chdir(bytes)` + `os.execvpe` with byte argv. Constructors have
//! no spawn side effect:
//!
//! - [`command`] builds the ssh invocation for an owned stdio client
//!   (language server): all three pipes are `Stdio::piped()` and the
//!   caller owns spawning, the stdin lease and reaping.
//! - [`command_supervised`] additionally hands back the
//!   [`SupervisionKey`] that identifies this session's supervisor
//!   records, and lets a caller pick finite stdin explicitly.
//! - [`run`] executes one finite command to completion on a worker
//!   under a [`CancelToken`], with bounded output and a deadline.
//!
//! ## Local + remote child ownership
//!
//! Local: spawn the returned command through
//! `strop_core::process::OwnedProcess` (or set `process_group(0)`
//! yourself when using another spawner). Cancellation SIGKILLs the
//! local ssh group and revokes the PID before reaping it; nothing here
//! holds editor state.
//!
//! Remote: SSH stdin is the lifetime lease. Keep the local stdin
//! writer open while the remote process should live; dropping it (or
//! the local ssh process dying) makes the remote supervisor SIGTERM
//! the worker's whole process group, escalate after a bounded grace,
//! SIGKILL it and reap — a worker that exited normally is cleaned up
//! the same way, because finite commands can leave descendants. A
//! graceful shutdown is therefore an application-level exchange first
//! (LSP `shutdown`/`exit`), *then* a lease close.
//!
//! The remote guarantee is conditional and stated honestly: cleanup
//! runs once the remote sshd observes the disconnection, so a network
//! partition delays it until sshd's own dead-peer detection fires;
//! descendants that create their own session escape a process-group
//! kill; and nothing survives a remote SIGKILL of the supervisor
//! itself. See `exec::supervisor` for the full topology and limits.

mod python;
mod run;
mod spec;
mod stream;
mod supervisor;
pub use stream::{stream, RemoteStreamError, RemoteStreamOutput};

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;
use strop_core::worker::CancelToken;
use strop_workspace::RemoteEndpoint;

/// Program identity is distinct from argv. Built-in scripts reuse the already
/// selected supervisor interpreter, never another PATH lookup for `python3`.
#[derive(Debug, Clone)]
pub enum RemoteProgram {
    Executable(OsString),
    SupervisorPython,
}
/// A checked description of one remote process. Pure data: nothing is
/// spawned by constructing, cloning or inspecting it.
#[derive(Debug, Clone)]
pub struct RemoteCommand {
    program: RemoteProgram,
    args: Vec<OsString>,
    cwd: PathBuf,
    deadline: Duration,
}

impl RemoteCommand {
    /// Admit one command. Refuses an empty program, NUL bytes in the
    /// program, arguments or working directory, a non-absolute working
    /// directory, and values that cannot be represented as native
    /// POSIX bytes. The program may be a bare name (remote `PATH`
    /// lookup) or contain `/` (direct path).
    pub fn new(
        program: impl Into<OsString>,
        args: Vec<OsString>,
        cwd: &Path,
    ) -> Result<Self, RemoteCommandError> {
        Self::admit(RemoteProgram::Executable(program.into()), args, cwd)
    }

    pub fn python(
        script: &str,
        args: Vec<OsString>,
        cwd: &Path,
    ) -> Result<Self, RemoteCommandError> {
        let mut arguments = Vec::with_capacity(args.len() + 2);
        arguments.push("-c".into());
        arguments.push(script.into());
        arguments.extend(args);
        Self::admit(RemoteProgram::SupervisorPython, arguments, cwd)
    }

    fn admit(
        program: RemoteProgram,
        args: Vec<OsString>,
        cwd: &Path,
    ) -> Result<Self, RemoteCommandError> {
        let command = Self {
            program,
            args,
            cwd: cwd.to_path_buf(),
            deadline: run::DEFAULT_DEADLINE,
        };
        // Re-run the full validation so later mutations can never
        // bypass admission; it is pure and cheap.
        spec::Spec::encode(
            StdinMode::Finite,
            [0u8; 16],
            &command.program,
            &command.args,
            &command.cwd,
        )
        .map_err(|error| match error {
            RemoteCommandError::ArgvTooLarge { .. } => RemoteCommandError::Invalid {
                detail: "program and arguments are too large for a remote command line".into(),
            },
            other => other,
        })?;
        Ok(command)
    }

    pub fn program(&self) -> &RemoteProgram {
        &self.program
    }

    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Wall-clock budget for [`run`]. Defaults to 120 seconds.
    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    /// Override the wall-clock budget. Still a pure description.
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }
}

/// What the supervised ssh connection does with the local stdin lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdinMode {
    /// The worker's stdin is `/dev/null`; the supervisor drains SSH
    /// stdin only to observe the lease. Finite Git commands.
    Finite,
    /// SSH stdin is relayed byte-for-byte into the worker's stdin with
    /// a bounded buffer. Language servers.
    Relayed,
}

/// The remote worker's termination outcome, when the supervisor
/// reported it. Non-zero codes are ordinary results — exit codes are
/// data for Git (`diff --quiet` exits 1), not transport failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteExitStatus {
    /// The worker exited with code 0–255.
    Exited(u32),
    /// The worker was terminated by the named signal.
    Signaled(u32),
}

impl RemoteExitStatus {
    pub fn code(&self) -> Option<u32> {
        match *self {
            RemoteExitStatus::Exited(code) => Some(code),
            RemoteExitStatus::Signaled(_) => None,
        }
    }

    pub fn signal(&self) -> Option<u32> {
        match *self {
            RemoteExitStatus::Exited(_) => None,
            RemoteExitStatus::Signaled(signal) => Some(signal),
        }
    }

    pub fn success(&self) -> bool {
        *self == RemoteExitStatus::Exited(0)
    }
}

/// Bounded captured output of one finished remote command. `stdout`
/// keeps its first `STDOUT_LIMIT` bytes; `stderr` its first
/// `STDERR_LIMIT` bytes plus the last `STDERR_TAIL` bytes (where the
/// supervisor's status record lives). The dropped counters say how
/// many further bytes arrived; treat any non-zero counter as
/// truncation, never as silence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: RemoteExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_dropped: u64,
    pub stderr_dropped: u64,
    /// Any upload failure is retained even when the program returned diagnostics.
    pub stdin_error: Option<String>,
}

/// Identifies one supervised session's status records: the supervisor
/// writes `STROP-SUP-v1 <nonce> ...` lines to stderr, and only lines
/// carrying this session's nonce are its. Not a secret — it travels
/// inside the spec and is visible in remote process listings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisionKey {
    nonce: [u8; 16],
}

impl SupervisionKey {
    pub(crate) fn generate() -> Self {
        Self {
            nonce: spec::nonce(),
        }
    }

    pub(crate) fn nonce(&self) -> [u8; 16] {
        self.nonce
    }

    /// Every supervisor record found in a captured stderr buffer, in
    /// order. An empty result means the supervision layer never
    /// reported; ssh's exit code is then the only evidence.
    pub fn records(&self, stderr: &[u8]) -> Vec<SupervisionOutcome> {
        supervisor::records(stderr, &self.hex())
    }

    /// Remove only this session's framing, including the delimiter that the
    /// supervisor inserts before a record. Worker stderr remains byte-exact.
    pub(crate) fn remove_records(&self, stderr: &mut Vec<u8>) {
        let marker = format!("STROP-SUP-v1 {} ", self.hex());
        let mut cursor = 0;
        while cursor < stderr.len() {
            let Some(relative) = stderr[cursor..]
                .windows(marker.len())
                .position(|bytes| bytes == marker.as_bytes())
            else {
                break;
            };
            let start = cursor + relative;
            if start != 0 && stderr[start - 1] != b'\n' {
                cursor = start + marker.len();
                continue;
            }
            let Some(length) = stderr[start..].iter().position(|&byte| byte == b'\n') else {
                break;
            };
            let end = start + length + 1;
            if !self.records(&stderr[start..end]).is_empty() {
                let first = start.saturating_sub(1);
                stderr.drain(first..end);
                cursor = first;
            } else {
                cursor = end;
            }
        }
    }

    fn hex(&self) -> String {
        self.nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// One parsed supervisor record. [`SupervisionOutcome::LaunchFailure`]
/// outranks a later exit record: the program never started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisionOutcome {
    Exited(u32),
    Signaled(u32),
    Cancelled,
    LaunchFailure(String),
    SupervisorError(String),
}

/// Why a remote command could not be admitted, spawned, supervised or
/// completed. Every variant is descriptive; none guesses success.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoteCommandError {
    #[error("remote command refused: {detail}")]
    Invalid { detail: String },
    #[error("cannot spawn ssh: {message}")]
    Spawn { message: String },
    #[error(
        "remote command line cannot carry {bytes} encoded bytes (argv/cwd too large for one ssh command)"
    )]
    ArgvTooLarge { bytes: usize },
    #[error(
        "remote execution needs compatible Python 3.8+ on remote PATH or STROP_REMOTE_PYTHON: {diagnostics}"
    )]
    MissingPython { diagnostics: String },
    #[error("remote program could not start: {diagnostics}")]
    Launch { diagnostics: String },
    #[error("remote supervisor failed at {stage}: {diagnostics}")]
    Supervisor { stage: String, diagnostics: String },
    #[error("ssh transport failed (exit {exit:?}): {diagnostics}")]
    Transport {
        exit: Option<i32>,
        diagnostics: String,
    },
    #[error("remote command cancelled before completion: {diagnostics}")]
    Cancelled { diagnostics: String },
    #[error("remote command did not finish within {seconds} seconds")]
    Timeout { seconds: u64 },
    #[error("local process supervision failed: {message}")]
    Local { message: String },
}

/// The ssh invocation for an owned stdio client — a language server.
/// Relayed stdin, all three pipes piped, no spawn. The caller owns the
/// local child (see the module docs for the lease and cancel story)
/// and may parse the supervisor's stderr records via
/// [`command_supervised`].
pub fn command(
    endpoint: &RemoteEndpoint,
    command: &RemoteCommand,
) -> Result<std::process::Command, RemoteCommandError> {
    command_supervised(endpoint, command, StdinMode::Relayed).map(|(process, _)| process)
}

/// [`command`] with the leash visible: choose the stdin mode explicitly
/// and keep the [`SupervisionKey`] for parsing status records out of
/// the session's stderr tail.
pub fn command_supervised(
    endpoint: &RemoteEndpoint,
    command: &RemoteCommand,
    mode: StdinMode,
) -> Result<(std::process::Command, SupervisionKey), RemoteCommandError> {
    run::supervised(endpoint, command, mode)
}

/// Run one finite remote command to completion. Worker-only: it blocks
/// for the whole exchange, bounded by the command's deadline. Each run
/// uses its own dedicated, noninteractive ssh connection — no pooled
/// session or lease is involved. Output is bounded per
/// [`CommandOutput`]; cancellation kills the local ssh group and the
/// remote supervisor tears down the remote group.
pub fn run(
    endpoint: &RemoteEndpoint,
    command: &RemoteCommand,
    token: &CancelToken,
) -> Result<CommandOutput, RemoteCommandError> {
    run::run(endpoint, command, token)
}

/// Worker-only framed input. Chunks are borrowed; no whole-rope copy is needed.
/// The stdin lease remains open after the final chunk until the program exits.
pub fn run_with_input(
    endpoint: &RemoteEndpoint,
    command: &RemoteCommand,
    token: &CancelToken,
    chunks: &[&[u8]],
) -> Result<CommandOutput, RemoteCommandError> {
    run::run_input(endpoint, command, token, Some(chunks))
}

#[cfg(all(test, unix))]
mod tests;
