//! Owned native execution supervision: the in-process Rust port of the fixed
//! supervisor semantics that strop-remote's `exec/supervisor.rs` embedded in
//! Python (0036), now without an interpreter (0058 WK03/WK10).
//!
//! ## Topology
//!
//! ```text
//! worker thread (supervisor)
//!   └─ target          setsid(); its PID is the PGID of every descendant
//! ```
//!
//! The Python topology needed a separate anchor process to pin the process
//! group while the supervisor signaled it; in-process, the target itself
//! leads the session and its unreaped zombie holds the PID — and therefore
//! the PGID — reservation through teardown, the identical guarantee.
//!
//! ## Semantics preserved
//!
//! - **Owned target, byte-exact argv.** [`ExecSpec`] admission keeps native
//!   POSIX bytes (non-UTF8 names survive), refuses NUL bytes, a relative cwd
//!   and an empty program, and bounds each argv element. Construction is
//!   pure; only [`launch`] spawns.
//! - **Readiness.** `launch` returns once the target's exec handshake
//!   resolved: chdir failure (125), not-found (127) and not-executable (126)
//!   are distinct typed launch failures, never misreported as exits.
//! - **Revoke-before-reap.** Cancellation or lease close latches, then a
//!   300 ms last-chance drain lets an already-recorded exit win over the
//!   cancel; otherwise the group receives SIGTERM, the bounded grace
//!   elapses, SIGKILL follows, and only then is the target reaped. A target
//!   that exits on the TERM is still reported revoked — the drain runs
//!   before the TERM, matching the Python ordering exactly.
//! - **Descendant cleanup.** A target that exited normally still has its
//!   process group SIGKILLed before reaping: finite commands can leave
//!   children behind. Descendants that call `setsid()` themselves escape a
//!   process-group kill; only a cgroup or service manager contains those.
//! - **Nonce-marked status records.** Terminal states encode byte-compatibly
//!   with the Python anchor's record and render as `STROP-SUP-v1`-marked
//!   lines, so a stream mixing target output with supervisor records stays
//!   separable. The nonce stops accidental collisions; a target that
//!   deliberately echoes it is an honesty limit, not a secret.
//! - **Half-close is not revoke.** [`Running::close_stdin`] delivers EOF and
//!   lets the target finish; revoking its lease tears the group down.
//!
//! Limits stated honestly: nothing here survives the worker process dying
//! before teardown, and same-principal malice is out of scope — the worker
//! beside the files is the trust boundary.

mod record;
#[cfg(unix)]
mod supervisor;
#[cfg(all(test, unix))]
mod tests;

pub use record::{mark_line, records, StatusRecord, SupervisionOutcome, MARK};
#[cfg(unix)]
pub use supervisor::{launch, Running, Settlement};

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::time::Duration;

/// Default TERM→KILL grace on revocation, matching the remote supervisor.
pub const DEFAULT_GRACE: Duration = Duration::from_millis(2_000);
/// Hard cap on the revocation grace, as the remote spec enforced.
const GRACE_CAP: Duration = Duration::from_millis(600_000);
/// The remote spec's argc bound, kept so a native target never exceeds what
/// the supervised contract allowed.
const ARGV_LIMIT: usize = 4096;
/// Linux `MAX_ARG_STRLEN` (128 KiB) bounds one argv element or the cwd.
const ARG_BYTES_LIMIT: usize = 128 * 1024;

/// What the owned target's stdin connects to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdinMode {
    /// The target's stdin is `/dev/null`. Finite commands.
    Finite,
    /// The target's stdin is a pipe the caller writes and half-closes.
    /// Language servers and streamed input.
    Relayed,
}

/// A checked description of one owned target process. Pure data: nothing is
/// spawned by constructing, cloning or inspecting it.
#[derive(Debug, Clone)]
pub struct ExecSpec {
    argv: Vec<Vec<u8>>,
    cwd: Vec<u8>,
    stdin: StdinMode,
    grace: Duration,
    nonce: [u8; 16],
}

/// Native POSIX bytes for a path or argument. Unix keeps arbitrary non-NUL
/// bytes; other platforms require UTF-8 rather than a lossy stand-in that
/// could name the wrong file.
fn os_bytes(value: &OsStr) -> Result<Vec<u8>, ExecError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(value.as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        value
            .to_str()
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| ExecError::Invalid {
                detail: "argument is not representable as native POSIX bytes on this platform"
                    .into(),
            })
    }
}

impl ExecSpec {
    /// Admit one owned target: byte-exact argv and an absolute cwd. Refuses
    /// an empty program, NUL bytes, a relative cwd, overlong elements and
    /// overlong argv. The program may be a bare name (PATH lookup) or
    /// contain `/` (direct path).
    pub fn new(
        program: &OsStr,
        args: impl IntoIterator<Item = OsString>,
        cwd: &Path,
    ) -> Result<Self, ExecError> {
        let program = os_bytes(program)?;
        if program.is_empty() {
            return Err(ExecError::Invalid {
                detail: "program is empty".into(),
            });
        }
        let mut argv = Vec::with_capacity(8);
        argv.push(program);
        for argument in args {
            argv.push(os_bytes(&argument)?);
        }
        if argv.len() > ARGV_LIMIT {
            return Err(ExecError::Invalid {
                detail: format!("argv exceeds the {}-element bound", ARGV_LIMIT),
            });
        }
        let cwd = os_bytes(cwd.as_os_str())?;
        if !cwd.starts_with(b"/") {
            return Err(ExecError::Invalid {
                detail: format!(
                    "working directory must be absolute, not {:?}",
                    String::from_utf8_lossy(&cwd)
                ),
            });
        }
        if cwd.contains(&0) || argv.iter().any(|argument| argument.contains(&0)) {
            return Err(ExecError::Invalid {
                detail: "program, arguments and working directory cannot contain a NUL byte".into(),
            });
        }
        if argv
            .iter()
            .chain(std::iter::once(&cwd))
            .any(|item| item.len() >= ARG_BYTES_LIMIT)
        {
            return Err(ExecError::Invalid {
                detail: "an argv element or the working directory exceeds the 128 KiB native bound"
                    .into(),
            });
        }
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|error| ExecError::Supervisor {
            stage: "nonce".into(),
            diagnostics: error.to_string(),
        })?;
        Ok(Self {
            argv,
            cwd,
            stdin: StdinMode::Finite,
            grace: DEFAULT_GRACE,
            nonce,
        })
    }

    /// Connect the target's stdin to a caller-owned pipe instead of
    /// `/dev/null`.
    pub fn relayed_stdin(mut self) -> Self {
        self.stdin = StdinMode::Relayed;
        self
    }

    /// Override the TERM→KILL revocation grace. Clamped to the 600 s cap.
    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace.min(GRACE_CAP);
        self
    }

    /// Override the session nonce. Tests pin it; production mints fresh.
    pub fn with_nonce(mut self, nonce: [u8; 16]) -> Self {
        self.nonce = nonce;
        self
    }

    pub fn argv(&self) -> &[Vec<u8>] {
        &self.argv
    }
    pub fn cwd(&self) -> &[u8] {
        &self.cwd
    }
    pub fn stdin_mode(&self) -> StdinMode {
        self.stdin
    }
    pub fn grace(&self) -> Duration {
        self.grace
    }
    /// The nonce marking this session's status records. Not a secret.
    pub fn nonce(&self) -> [u8; 16] {
        self.nonce
    }
}

/// Why an owned target could not be admitted, spawned, supervised or
/// settled. Every variant is descriptive; none guesses success.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecError {
    #[error("execution spec refused: {detail}")]
    Invalid { detail: String },
    #[error("cannot spawn the owned target: {message}")]
    Spawn { message: String },
    #[error("the owned target could not start: {diagnostics}")]
    Launch { diagnostics: String },
    #[error("supervision failed at {stage}: {diagnostics}")]
    Supervisor { stage: String, diagnostics: String },
    #[error("execution was cancelled before the target launched")]
    Cancelled,
    #[error("owned process supervision requires Unix")]
    Unavailable,
}
