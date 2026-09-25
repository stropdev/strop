//! Supervised in-container execution (0037 DC1b, rebuilt for 0056 AR07).
//!
//! An execution request is *frozen* before it is admitted: the selected
//! [`EngineRef`] (a probe-pinned connection), the incarnation-pinned
//! [`ContainerRef`], the principal, the working directory, the program
//! and its argv, and explicit environment overlays. Admission
//! ([`ExecSpec::admit`]) re-checks the canonical id and the `StartedAt`
//! incarnation against the engine, so a recycled or restarted container
//! never receives a stale request — the refusal is typed.
//!
//! ## The in-container lease
//!
//! The program never runs bare. `docker exec` launches a fixed POSIX sh
//! *supervisor* (carried as argv data, never interpolated) which owns the
//! program inside the exec session's process group, relays stdin where
//! the mode requires it, and treats stdin EOF as the lifetime lease:
//! local close, local client death, or daemon teardown all end with the
//! supervisor SIGTERMing the whole group and escalating to SIGKILL after
//! a bounded grace. A program that exits normally still has its group
//! swept — finite commands can leave descendants behind. Launch, exit
//! and termination are reported as nonce-marked stderr records; the
//! session's [`SessionKey`] parses them.
//!
//! Honest limits, mirroring strop-remote's SSH supervisor:
//!
//! - Cleanup runs once the daemon observes the client's disconnect;
//!   against the local socket that is prompt, but it is not synchronous
//!   with the local kill, and nothing survives a daemon crash.
//! - A descendant that calls `setsid(2)` itself escapes the group kill;
//!   only a cgroup contains those, and `docker exec` offers none.
//! - Zombies under a container init that never `wait(2)`s are that
//!   init's to reap; the lease kills, it cannot reap another session's
//!   dead.
//! - The supervisor requires a POSIX `sh` in the image. Distroless
//!   images have none: the run then fails as a typed
//!   [`ContainerError::ExecLaunch`] naming the missing capability —
//!   never a silent fallback to unsupervised exec, and never an implicit
//!   install or provisioning step. There is no pre-probe for the program
//!   either: the supervisor classifies the launch truthfully from inside
//!   the selected namespace (`command -v` first, the real exec error
//!   otherwise), so a found-but-unusable toolchain shim is a typed
//!   diagnostic. Readiness for a real consumer is that consumer's own
//!   bounded version/handshake run through this same channel — never
//!   universal pre-probing.
//!
//! A container name is not an identity, and a host `docker top` PID is
//! not a namespace-local one: the supervisor's `launched` record carries
//! the in-container PID, and nothing here maps one namespace onto the
//! other.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::engine::{refresh, stderr_tail, Captured, EngineRef};
use crate::identity::ContainerRef;
use crate::ContainerError;
use strop_core::process::OwnedProcess;
use strop_core::worker::CancelToken;
use strop_workspace::ContainerId;

/// TERM→KILL grace the in-container supervisor applies on lease close,
/// in seconds. Owned locally so a policy change never needs an image
/// update.
const GRACE_SECONDS: u32 = 2;

/// Wall-clock budget for one bounded in-container command run.
const EXEC_DEADLINE: Duration = Duration::from_secs(30);

/// Poll cadence of the local capture loop.
const POLL: Duration = Duration::from_millis(20);

/// Stderr retention around an exec run: a head for diagnostics plus a
/// tail, because the supervisor's status records are the last lines
/// written and must survive a chatty program.
const EXEC_STDERR_HEAD: u64 = 64 * 1024;
const EXEC_STDERR_TAIL: u64 = 4 * 1024;

/// The fixed supervisor program. POSIX sh, builtins plus `cat`/`sleep`;
/// every value it operates on arrives as a positional parameter — inert
/// data, never parsed as shell syntax. Empirically qualified against
/// busybox ash (the compose `container-test` fixture image).
///
/// Topology: each `docker exec` session is its own process group (the
/// exec'd process is its group leader), so the supervisor can signal the
/// whole session — program, relay, watcher and descendants — with one
/// group kill and never touch the container's init or sibling sessions.
const SUPERVISOR_SOURCE: &str = r#"sup=$$
nonce=$1 grace=$2 mode=$3 prog=$4
shift 4

note() { echo "STROP-EXEC-v1 $nonce $*" >&2; }

# In-launch classification inside the selected namespace: a program that
# does not resolve here is a named refusal, never a silent fallback.
resolved=$(command -v "$prog" 2>/dev/null) || {
  note "exec-error not-found $prog"
  exit 127
}
case $resolved in
  */*)
    [ -x "$resolved" ] || {
      note "exec-error not-executable $prog"
      exit 126
    }
    ;;
esac

# stdin is the lifetime lease; in relay mode it is also the byte stream
# the program consumes. Non-interactive shells strip stdin from
# background jobs, so the lease is preserved on fd 3 first. Lease EOF ->
# TERM the supervisor, whose trap runs the group cleanup below.
exec 3<&0
if [ "$mode" = relay ]; then
  { cat <&3; note lease-closed; kill -TERM "$sup" 2>/dev/null; exit 0; } |
    { exec "$prog" "$@"; } &
else
  { cat <&3 >/dev/null; note lease-closed; kill -TERM "$sup" 2>/dev/null; exit 0; } &
  "$prog" "$@" < /dev/null &
fi
worker=$!
note "launched $worker"

term() {
  trap '' TERM HUP INT
  kill -TERM -"$sup" 2>/dev/null
  note terminated
  { sleep "$grace"; kill -KILL -"$sup" 2>/dev/null; } >/dev/null 2>&1 &
  wait "$worker" 2>/dev/null
  exit 245
}
trap term TERM HUP INT

wait "$worker"
status=$?
trap '' TERM HUP INT
# A finished program can still leave descendants behind (and the relay
# or watcher may be parked on the lease): TERM the whole group now. The
# delayed killer sweeps whatever survives; being itself a group member,
# it pins the pgid until the moment it fires.
kill -TERM -"$sup" 2>/dev/null
{ while kill -0 "$sup" 2>/dev/null; do sleep 1; done
  kill -KILL -"$sup" 2>/dev/null; } >/dev/null 2>&1 &
note "exit $status"
exit "$status"
"#;

/// What the supervisor does with the lease's byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Finite command: the program reads `/dev/null`; the lease is pure
    /// lifetime.
    Null,
    /// Interactive protocol channel: lease bytes are relayed to the
    /// program's stdin.
    Relay,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Relay => "relay",
        }
    }
}

/// One frozen execution request: selected engine/connection,
/// incarnation-pinned container, principal, working directory, program,
/// argv and environment overlays. Pure data until [`ExecSpec::admit`] —
/// construction, cloning and inspection have no engine effect.
#[derive(Debug, Clone)]
pub struct ExecSpec {
    engine: EngineRef,
    container: ContainerRef,
    program: String,
    args: Vec<String>,
    cwd: String,
    env: Vec<(String, String)>,
    user: Option<String>,
}

impl ExecSpec {
    /// Freeze a request against an already incarnation-pinned reference.
    /// Pure validation — nothing is probed or spawned.
    pub fn new(
        engine: &EngineRef,
        container: &ContainerRef,
        program: &str,
        args: &[String],
        cwd: &Path,
    ) -> Result<Self, ContainerError> {
        if program.is_empty() || program.contains('\0') {
            return Err(ContainerError::CapabilityRefused {
                what: "exec: empty or NUL-carrying program name".into(),
            });
        }
        if args.iter().any(|arg| arg.contains('\0')) {
            return Err(ContainerError::CapabilityRefused {
                what: "exec: NUL byte in argv".into(),
            });
        }
        let Some(cwd_text) = cwd.to_str() else {
            return Err(ContainerError::CapabilityRefused {
                what: "exec: the working directory is not UTF-8".into(),
            });
        };
        Ok(Self {
            engine: engine.clone(),
            container: container.clone(),
            program: program.to_string(),
            args: args.to_vec(),
            cwd: cwd_text.to_string(),
            env: Vec::new(),
            user: None,
        })
    }

    /// Freeze an explicit principal (`docker exec --user`); without one
    /// the container's configured user applies. Never elevation: no flag
    /// here grants privileges the container does not already have.
    pub fn with_user(mut self, user: &str) -> Result<Self, ContainerError> {
        if user.is_empty() || user.starts_with('-') || user.contains('\0') {
            return Err(ContainerError::CapabilityRefused {
                what: format!("exec: invalid principal {user:?}"),
            });
        }
        self.user = Some(user.to_string());
        Ok(self)
    }

    /// Freeze explicit environment overlays (`docker exec --env`); the
    /// image's own environment applies underneath. Overlay names carry
    /// no `=`; nothing here mutates any global environment.
    pub fn with_env(mut self, pairs: &[(String, String)]) -> Result<Self, ContainerError> {
        for (name, value) in pairs {
            if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
                return Err(ContainerError::CapabilityRefused {
                    what: format!("exec: invalid environment overlay {name:?}"),
                });
            }
        }
        self.env = pairs.to_vec();
        Ok(self)
    }

    /// Resolve a bare canonical id to the container's *current*
    /// incarnation and freeze+admit the request against it — for callers
    /// whose workspace carries no pinned incarnation. Selection happens
    /// here: inspect pins id and `StartedAt`, admission re-checks both.
    pub fn resolve(
        engine: &EngineRef,
        id: &ContainerId,
        program: &str,
        args: &[String],
        cwd: &Path,
        token: &CancelToken,
    ) -> Result<AdmittedExec, ContainerError> {
        let identity = crate::inspect(engine, id.as_str(), token)?;
        let reference = ContainerRef::of(&identity)?;
        Self::new(engine, &reference, program, args, cwd)?.admit(token)
    }

    /// Admit the frozen request: the pinned container must still exist,
    /// still be the same incarnation (`StartedAt`) and still be running.
    /// One bounded inspect — the price of never executing in a recycled
    /// container as if it were the pinned one.
    pub fn admit(&self, token: &CancelToken) -> Result<AdmittedExec, ContainerError> {
        refresh(&self.engine, &self.container, token)?;
        Ok(AdmittedExec {
            spec: self.clone(),
            key: SessionKey::new(),
        })
    }
}

/// An admitted execution request: the incarnation check passed and the
/// session key is minted. Dropping it un-spawned has no engine effect —
/// admission is read-only.
#[derive(Debug, Clone)]
pub struct AdmittedExec {
    spec: ExecSpec,
    key: SessionKey,
}

impl AdmittedExec {
    /// This session's key: parses the supervisor's records out of the
    /// session's stderr tail.
    pub fn key(&self) -> &SessionKey {
        &self.key
    }

    /// The interactive supervised stdio channel (DC1b): language servers
    /// and other long-lived protocol programs. All three pipes are
    /// piped; the caller owns spawning, supervision and the stdin lease —
    /// closing stdin (or the local client's death) ends the in-container
    /// group through the supervisor, so dropping the local process is
    /// teardown, not a leak.
    pub fn command(&self) -> Command {
        self.build(Mode::Relay)
    }

    /// The strop worker's own exec channel for images without a POSIX
    /// sh (0058 WK08, the shellless preinstalled case): the verified
    /// worker binary IS the supervisor — it serves the protocol on
    /// stdin/stdout and its own session teardown reaps what it
    /// launches, so the fixed sh supervisor adds nothing it needs.
    /// All three pipes are piped; stdin is the lifetime lease exactly
    /// as in [`Self::command`]. Only the worker takes this path:
    /// ordinary programs keep the typed distroless [`ContainerError::ExecLaunch`]
    /// refusal, never an unsupervised fallback.
    pub fn worker_command(&self) -> Command {
        let mut command = self.exec_prefix();
        command
            .arg(self.spec.container.id().as_str())
            .arg(&self.spec.program)
            .args(&self.spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// One bounded in-container command run to completion (Git and
    /// friends): deadline-bounded, caller-cancelled, stdout retained up
    /// to `stdout_limit`. Launch classification is typed: a program that
    /// never launched is [`ContainerError::ExecLaunch`], never a fake
    /// exit code; a real program's own exit code stays data (Git's
    /// `diff --quiet` exiting 1 is a result, not a transport failure).
    pub fn capture(
        &self,
        stdout_limit: u64,
        token: &CancelToken,
    ) -> Result<Captured, ContainerError> {
        let mut command = self.build(Mode::Null);
        let output = lease_capture(&mut command, stdout_limit, EXEC_DEADLINE, token)?;
        let records = self.key.records(&output.stderr);
        let refused = records.iter().find_map(|record| match record {
            ExecRecord::LaunchFailed { program, cause } => Some(format!("{program}: {cause}")),
            _ => None,
        });
        if let Some(detail) = refused {
            return Err(ContainerError::ExecLaunch { detail });
        }
        // The daemon reports a missing `sh` with a non-zero exit and no
        // records; under `-i` the message lands on stdout (the exec
        // stream), on older daemons on stderr — both are the daemon's
        // own words, and a real program's lookalike output is excluded
        // by the empty-records guard (a started supervisor always
        // records `launched`).
        if records.is_empty()
            && output.code != Some(0)
            && (supervisor_refused(&output.stderr) || supervisor_refused(&output.stdout))
        {
            let detail = if output.stderr.is_empty() {
                stderr_tail(&output.stdout)
            } else {
                stderr_tail(&output.stderr)
            };
            return Err(ContainerError::ExecLaunch {
                detail: format!(
                    "the supervisor itself could not start (the image carries no POSIX sh): {detail}"
                ),
            });
        }
        Ok(output)
    }

    /// The pinned `docker exec` invocation prefix: probe-pinned engine,
    /// argv-only, the selected workdir/principal/environment. Everything
    /// after the container id is the payload argv — inert data to the
    /// CLI, never shell-interpreted.
    fn exec_prefix(&self) -> Command {
        let spec = &self.spec;
        let mut command = Command::new("docker");
        for arg in spec.engine.context_args() {
            command.arg(arg);
        }
        command
            .arg("exec")
            .arg("-i")
            .arg("--workdir")
            .arg(&spec.cwd);
        if let Some(user) = &spec.user {
            command.arg("--user").arg(user);
        }
        for (name, value) in &spec.env {
            command.arg("--env").arg(format!("{name}={value}"));
        }
        command
    }

    /// The pinned `docker exec` invocation: argv-only, fixed supervisor
    /// source, all three stdio pipes. Positional parameters after the
    /// container id are inert data to the CLI.
    fn build(&self, mode: Mode) -> Command {
        let mut command = self.exec_prefix();
        command
            .arg(self.spec.container.id().as_str())
            .arg("sh")
            .arg("-c")
            .arg(SUPERVISOR_SOURCE)
            .arg("strop-supervisor")
            .arg(self.key.nonce_hex())
            .arg(GRACE_SECONDS.to_string())
            .arg(mode.as_str())
            .arg(&self.spec.program)
            .args(&self.spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
}

/// The daemon's own report that the supervisor's `sh` never started.
/// Only consulted when the session produced no records at all and a
/// non-zero exit, so a program printing lookalike text is never
/// misclassified — its records exist.
fn supervisor_refused(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr);
    text.contains("OCI runtime exec failed") || text.contains("executable file not found")
}

/// Identifies one supervised session's stderr records: the supervisor
/// writes `STROP-EXEC-v1 <nonce> ...` lines, and only lines carrying
/// this session's nonce are its. Not a secret — the nonce travels in
/// argv, visible in process listings — and not a defense against a
/// program that deliberately echoes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKey {
    nonce: [u8; 16],
}

impl SessionKey {
    /// 128 bits from the process-seeded SipHash keys (the strop-remote
    /// spec nonce idiom).
    fn new() -> Self {
        use std::hash::{BuildHasher, Hasher};
        let mut nonce = [0u8; 16];
        let mut left = std::collections::hash_map::RandomState::new().build_hasher();
        left.write_u64(0x5354_524f_505f_4558); // "STROP_EX"
        nonce[..8].copy_from_slice(&left.finish().to_le_bytes());
        let mut right = std::collections::hash_map::RandomState::new().build_hasher();
        right.write_u64(0x4543_5f53_5550_4552); // "EC_SUPER"
        nonce[8..].copy_from_slice(&right.finish().to_le_bytes());
        Self { nonce }
    }

    #[cfg(test)]
    fn fixed(nonce: [u8; 16]) -> Self {
        Self { nonce }
    }

    fn nonce_hex(&self) -> String {
        self.nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Extract this session's records from captured stderr, in order.
    /// Lines not carrying this nonce — including a program's own
    /// lookalike output — are ignored.
    pub fn records(&self, stderr: &[u8]) -> Vec<ExecRecord> {
        let mark = format!("STROP-EXEC-v1 {} ", self.nonce_hex());
        let text = String::from_utf8_lossy(stderr);
        text.lines()
            .filter_map(|line| {
                let start = line.find(mark.as_str())?;
                let rest = &line[start + mark.len()..];
                if let Some(pid) = rest.strip_prefix("launched ") {
                    return pid
                        .trim()
                        .parse()
                        .ok()
                        .map(|pid| ExecRecord::Launched { pid });
                }
                if let Some(code) = rest.strip_prefix("exit ") {
                    return code
                        .trim()
                        .parse()
                        .ok()
                        .map(|code| ExecRecord::Exit { code });
                }
                match rest.trim_end() {
                    "lease-closed" => Some(ExecRecord::LeaseClosed),
                    "terminated" => Some(ExecRecord::Terminated),
                    _ => rest.strip_prefix("exec-error ").and_then(|failure| {
                        match failure.split_once(' ') {
                            Some(("not-found", program)) => Some(ExecRecord::LaunchFailed {
                                program: program.to_string(),
                                cause: LaunchCause::NotFound,
                            }),
                            Some(("not-executable", program)) => Some(ExecRecord::LaunchFailed {
                                program: program.to_string(),
                                cause: LaunchCause::NotExecutable,
                            }),
                            _ => None,
                        }
                    }),
                }
            })
            .collect()
    }
}

/// Why the program never launched — a typed launch diagnostic, not a
/// silent fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchCause {
    /// Does not resolve on the container's PATH.
    NotFound,
    /// Resolves but is not executable in this namespace.
    NotExecutable,
}

impl std::fmt::Display for LaunchCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "not found in the container"),
            Self::NotExecutable => write!(f, "not executable in the container"),
        }
    }
}

/// One parsed supervisor record. [`ExecRecord::LaunchFailed`] outranks
/// any exit code: the program never ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecRecord {
    /// The program launched; the PID is namespace-local — never confuse
    /// it with a host `docker top` PID, which numbers a different
    /// namespace.
    Launched { pid: u32 },
    /// The program exited with this code (128+n for a signal, sh style).
    Exit { code: i32 },
    /// The lease's byte stream closed (observed by the supervisor).
    LeaseClosed,
    /// The lease closed and the supervisor terminated the whole group.
    Terminated,
    /// The program never launched; the supervisor classified why.
    LaunchFailed { program: String, cause: LaunchCause },
}

/// Bounded retention for one pipe: the first `head` bytes plus the last
/// `tail` bytes, counting everything dropped in between. Mirrors
/// strop-core's `read_pipe`, duplicated here because the capture loop
/// below — not the pipe policy — is what differs.
struct Retained {
    bytes: Vec<u8>,
    dropped: u64,
}

fn retain(mut pipe: impl Read, head: u64, tail: u64) -> io::Result<Retained> {
    let mut kept = Vec::new();
    let mut window: Vec<u8> = Vec::new();
    let mut dropped = 0u64;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(seen) => {
                let mut data = &chunk[..seen];
                let room = (head as usize).saturating_sub(kept.len());
                if room > 0 {
                    let take = room.min(data.len());
                    kept.extend_from_slice(&data[..take]);
                    data = &data[take..];
                }
                if !data.is_empty() {
                    if tail > 0 {
                        window.extend_from_slice(data);
                        let over = window.len().saturating_sub(tail as usize);
                        if over > 0 {
                            window.drain(..over);
                            dropped += over as u64;
                        }
                    } else {
                        dropped += data.len() as u64;
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    kept.extend_from_slice(&window);
    Ok(Retained {
        bytes: kept,
        dropped,
    })
}

/// One drained pipe's outcome.
enum Drained {
    Stdout(io::Result<Retained>),
    Stderr(io::Result<Retained>),
}

/// Run one admitted supervised command to completion.
///
/// The local side mirrors `strop_core::process::capture_with` with one
/// deliberate difference: the stdin lease writer is held until the
/// session's pipes EOF — that is, until the supervisor stack has
/// actually exited — and only then closed, because the docker CLI holds
/// an `-i` exec session open until its stdin closes. Cancellation or
/// the deadline instead SIGKILL the local process group
/// ([`OwnedProcess`]); the daemon then closes the session's stdin and
/// the in-container supervisor's lease watch performs the real group
/// teardown.
fn lease_capture(
    command: &mut Command,
    stdout_limit: u64,
    deadline: Duration,
    token: &CancelToken,
) -> Result<Captured, ContainerError> {
    std::thread::scope(|scope| {
        let mut process = OwnedProcess::spawn(command, token).map_err(|failure| {
            if failure.kind == strop_core::worker::FailureKind::Spawn {
                ContainerError::EngineUnavailable {
                    detail: failure.message,
                }
            } else {
                ContainerError::Io {
                    detail: failure.message,
                }
            }
        })?;
        let mut lease = process.take_stdin();
        let stdout = process
            .take_stdout()
            .ok_or_else(|| ContainerError::Protocol {
                detail: "exec child is missing stdout".into(),
            })?;
        let stderr = process
            .take_stderr()
            .ok_or_else(|| ContainerError::Protocol {
                detail: "exec child is missing stderr".into(),
            })?;
        let (tx, rx) = channel();
        let out_tx = tx.clone();
        std::thread::Builder::new()
            .name("exec-stdout".into())
            .spawn_scoped(scope, move || {
                let _ = out_tx.send(Drained::Stdout(retain(stdout, stdout_limit, 0)));
            })
            .map_err(|error| ContainerError::Io {
                detail: format!("exec stdout drainer: {error}"),
            })?;
        std::thread::Builder::new()
            .name("exec-stderr".into())
            .spawn_scoped(scope, move || {
                let _ = tx.send(Drained::Stderr(retain(
                    stderr,
                    EXEC_STDERR_HEAD,
                    EXEC_STDERR_TAIL,
                )));
            })
            .map_err(|error| ContainerError::Io {
                detail: format!("exec stderr drainer: {error}"),
            })?;
        let end = Instant::now() + deadline;
        let mut stdout: Option<Retained> = None;
        let mut stderr: Option<Retained> = None;
        let status = loop {
            if token.is_cancelled() {
                return Err(ContainerError::Cancelled);
            }
            if Instant::now() >= end {
                return Err(ContainerError::Io {
                    detail: format!("docker exec timed out after {}s", deadline.as_secs()),
                });
            }
            if lease.is_some() && stdout.is_some() && stderr.is_some() {
                // The supervisor stack has exited; close the lease so the
                // CLI can finish the session and report its exit code.
                drop(lease.take());
            }
            let exited = process.has_exited().map_err(|failure| ContainerError::Io {
                detail: failure.message,
            })?;
            if exited && stdout.is_some() && stderr.is_some() {
                process.terminate().map_err(|failure| ContainerError::Io {
                    detail: failure.message,
                })?;
                break process.wait().map_err(|failure| ContainerError::Io {
                    detail: failure.message,
                })?;
            }
            match rx.recv_timeout(POLL) {
                Ok(Drained::Stdout(result)) => {
                    stdout = Some(result.map_err(|error| ContainerError::Io {
                        detail: format!("exec stdout pipe: {error}"),
                    })?);
                }
                Ok(Drained::Stderr(result)) => {
                    stderr = Some(result.map_err(|error| ContainerError::Io {
                        detail: format!("exec stderr pipe: {error}"),
                    })?);
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    if !process.has_exited().map_err(|failure| ContainerError::Io {
                        detail: failure.message,
                    })? {
                        std::thread::park_timeout(POLL);
                    }
                    continue;
                }
            }
        };
        let stdout = stdout.unwrap_or(Retained {
            bytes: Vec::new(),
            dropped: 0,
        });
        let stderr = stderr.unwrap_or(Retained {
            bytes: Vec::new(),
            dropped: 0,
        });
        Ok(Captured {
            code: status.code(),
            stdout: stdout.bytes,
            stderr: stderr.bytes,
            stdout_dropped: stdout.dropped,
        })
    })
}

#[cfg(test)]
mod tests;
