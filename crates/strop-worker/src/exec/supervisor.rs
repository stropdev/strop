//! The in-process supervisor: one owned target's lifecycle from spawn to
//! reap. See `exec/mod.rs` for the topology and the semantics contract this
//! ports from the Python supervisor.

use super::{ExecError, ExecSpec, StatusRecord, StdinMode};
use parking_lot::Mutex;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use strop_core::worker::CancelToken;

/// The last-chance drain on revoke: an already-settled exit recorded within
/// this window beats the cancellation that raced it (the Python supervisor's
/// 300 ms drain, unchanged).
const DRAIN: Duration = Duration::from_millis(300);
/// Wait-loop poll cadence; the Python loop polled at 200 ms.
const POLL: Duration = Duration::from_millis(20);

/// The supervised session's settled outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// The target's own terminal state, recorded before any teardown. A
    /// recorded status beats a concurrent cancellation or lease close.
    Recorded(StatusRecord),
    /// The lease was revoked: SIGTERM, the bounded grace, SIGKILL, then reap.
    /// A target that exited on the TERM is still revoked — the drain runs
    /// before the TERM, matching the Python ordering exactly.
    Revoked,
}

struct Shared {
    pid: u32,
    cancelled: bool,
    reaped: bool,
}

/// One running supervised target. Owns the child, the cancellation latch and
/// the teardown policy. Worker-stack only: [`Drop`] revokes the lease and may
/// wait out the grace, so never put this in editor state.
pub struct Running {
    child: Child,
    grace: Duration,
    nonce: [u8; 16],
    shared: Arc<Mutex<Shared>>,
    token: CancelToken,
    reaped: bool,
}

/// Launch one admitted spec under `token`'s lease. Returns once the target's
/// exec handshake resolved: a typed launch failure (chdir, not-found,
/// not-executable) never masquerades as an exit. A token cancelled while the
/// spawn was in flight revokes the fresh target before returning.
pub fn launch(spec: &ExecSpec, token: &CancelToken) -> Result<Running, ExecError> {
    use std::ffi::{CString, OsStr};
    use std::os::unix::ffi::OsStrExt;
    if token.is_cancelled() {
        return Err(ExecError::Cancelled);
    }
    // The chdir handshake: the child reports a chdir failure as `C` + errno
    // before exiting, so the parent distinguishes it from an exec failure.
    // libc pipe + FD_CLOEXEC: rustix's pipe_with is Linux/FreeBSD-only
    // (pipe2); macOS has fcntl only.
    let (handshake_read, handshake_write) = {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: `fds` is valid writable storage for two descriptors.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            let error = std::io::Error::last_os_error();
            return Err(ExecError::Supervisor {
                stage: "pipe".into(),
                diagnostics: error.to_string(),
            });
        }
        // SAFETY: both descriptors are live from the successful pipe call;
        // FD_CLOEXEC on a live descriptor cannot fail.
        unsafe {
            libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC);
        }
        use std::os::fd::FromRawFd;
        // SAFETY: the descriptors are owned by this scope from the
        // successful pipe call and are transferred exactly once.
        unsafe {
            (
                std::os::fd::OwnedFd::from_raw_fd(fds[0]),
                std::os::fd::OwnedFd::from_raw_fd(fds[1]),
            )
        }
    };
    // Admission already refused NUL bytes; the conversion cannot fail.
    let Some(cwd) = CString::new(spec.cwd()).ok() else {
        return Err(ExecError::Invalid {
            detail: "working directory cannot contain a NUL byte".into(),
        });
    };
    let program = &spec.argv()[0];
    let mut command = Command::new(OsStr::from_bytes(program));
    use std::os::unix::process::CommandExt;
    command.args(spec.argv()[1..].iter().map(|arg| OsStr::from_bytes(arg)));
    command
        .stdin(match spec.stdin_mode() {
            StdinMode::Finite => Stdio::null(),
            StdinMode::Relayed => Stdio::piped(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    {
        use std::os::fd::AsRawFd;
        let handshake = handshake_write.as_raw_fd();
        // SAFETY: runs in the forked child before exec; only async-signal-safe
        // calls (signal, pthread_sigmask, setsid, chdir, write) and no
        // allocation — the CString was built before the fork.
        unsafe {
            command.pre_exec(move || {
                for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGPIPE, libc::SIGTERM] {
                    libc::signal(signal, libc::SIG_DFL);
                }
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
                // Session leader: this PID is the process-group identity of the
                // target and every descendant, reserved until the supervisor
                // reaps the zombie.
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::chdir(cwd.as_ptr()) == -1 {
                    let error = std::io::Error::last_os_error();
                    let mut record = [0_u8; 5];
                    record[0] = b'C';
                    record[1..]
                        .copy_from_slice(&(error.raw_os_error().unwrap_or(0) as u32).to_le_bytes());
                    libc::write(handshake, record.as_ptr().cast(), record.len());
                    return Err(error);
                }
                Ok(())
            });
        }
    }
    let shared = Arc::new(Mutex::new(Shared {
        pid: 0,
        cancelled: false,
        reaped: false,
    }));
    let latch = shared.clone();
    token
        .register_cancel_resource(move || {
            latch.lock().cancelled = true;
            Ok(())
        })
        .map_err(|error| ExecError::Supervisor {
            stage: "cancel-registration".into(),
            diagnostics: error.message,
        })?;
    if token.is_cancelled() {
        token.clear_cancel_resource();
        return Err(ExecError::Cancelled);
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            token.clear_cancel_resource();
            // The parent still holds the handshake's write end; without
            // closing it the classify read would block instead of seeing the
            // dead child's EOF.
            drop(handshake_write);
            return Err(classify_launch(&handshake_read, error, program));
        }
    };
    drop(handshake_read);
    drop(handshake_write);
    shared.lock().pid = child.id();
    let running = Running {
        child,
        grace: spec.grace(),
        nonce: spec.nonce(),
        shared,
        token: token.clone(),
        reaped: false,
    };
    if running.shared.lock().cancelled || token.is_cancelled() {
        // Cancelled mid-spawn: revoke the fresh target before reporting.
        let _ = running.revoke();
        return Err(ExecError::Cancelled);
    }
    Ok(running)
}

/// Turn a failed spawn into the typed launch failure: the chdir handshake
/// record wins over the exec errno, then classic errno meanings apply.
fn classify_launch(
    handshake: &std::os::fd::OwnedFd,
    error: std::io::Error,
    program: &[u8],
) -> ExecError {
    let mut record = [0_u8; 5];
    // The child is dead at this point, so its write end is closed: this read
    // returns the record or EOF immediately.
    if rustix::io::read(handshake, &mut record).unwrap_or(0) == record.len() && record[0] == b'C' {
        let errno = u32::from_le_bytes(record[1..].try_into().expect("5-byte handshake record"));
        return ExecError::Launch {
            diagnostics: format!("chdir failed (errno {errno})"),
        };
    }
    let program = String::from_utf8_lossy(program);
    let diagnostics = match error.raw_os_error() {
        Some(libc::ENOENT) => format!("not-found {program}"),
        Some(libc::EACCES) => format!("not-executable {program}"),
        _ => format!("os {error}"),
    };
    ExecError::Launch { diagnostics }
}

impl Running {
    /// The session nonce marking this target's status records. Not a secret.
    pub fn nonce(&self) -> [u8; 16] {
        self.nonce
    }

    /// The target's PID, which is also its process-group identity.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// The relayed-stdin writer, once.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    /// Half-close the target's stdin: EOF, not revocation. The target keeps
    /// running; its shutdown response may still be in flight.
    pub fn close_stdin(&mut self) {
        drop(self.child.stdin.take());
    }

    /// Wait for the target to settle, then revoke-before-reap: the group is
    /// SIGKILLed for descendant cleanup while the unreaped zombie still
    /// reserves the PGID, and only then is the target reaped. Cancellation
    /// enters the revocation path; an already-recorded exit beats it.
    pub fn wait(self) -> Result<Settlement, ExecError> {
        loop {
            if let Some(record) = self.poll_record()? {
                return self.settle(record);
            }
            if self.shared.lock().cancelled || self.token.is_cancelled() {
                return self.revoke();
            }
            std::thread::park_timeout(POLL);
        }
    }

    /// Revoke the lease explicitly: last-chance drain, then TERM, the bounded
    /// grace, KILL, and only then reap.
    pub fn revoke(mut self) -> Result<Settlement, ExecError> {
        let deadline = Instant::now() + DRAIN;
        loop {
            if let Some(record) = self.poll_record()? {
                return self.settle(record);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            std::thread::park_timeout(remaining.min(POLL));
        }
        self.signal_group(libc::SIGTERM)?;
        sleep_grace(self.grace);
        self.signal_group(libc::SIGKILL)?;
        self.reap()?;
        Ok(Settlement::Revoked)
    }

    /// The target's terminal state without reaping it, when it settled. The
    /// zombie keeps the PID — and PGID — reservation alive.
    pub(crate) fn poll_record(&self) -> Result<Option<StatusRecord>, ExecError> {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        loop {
            // SAFETY: zero initializes siginfo_t; waitid writes it on success.
            // WNOWAIT preserves the PID reservation; WNOHANG never blocks.
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(ExecError::Supervisor {
                    stage: "wait".into(),
                    diagnostics: error.to_string(),
                });
            }
        }
        // SAFETY: successful waitid initialized the platform layout.
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        let status = unsafe { info.si_status() } as u32;
        match info.si_code {
            libc::CLD_EXITED => Ok(Some(StatusRecord::Exited(status))),
            libc::CLD_KILLED | libc::CLD_DUMPED => Ok(Some(StatusRecord::Signaled(status))),
            other => Err(ExecError::Supervisor {
                stage: "wait".into(),
                diagnostics: format!("unexpected wait status {other}"),
            }),
        }
    }

    /// Recorded exit: descendants are killed before the target is reaped, so
    /// a finite command cannot leave its children behind.
    fn settle(mut self, record: StatusRecord) -> Result<Settlement, ExecError> {
        self.signal_group(libc::SIGKILL)?;
        self.reap()?;
        Ok(Settlement::Recorded(record))
    }

    fn signal_group(&self, signal: libc::c_int) -> Result<(), ExecError> {
        let shared = self.shared.lock();
        if shared.reaped {
            return Ok(());
        }
        // SAFETY: this positive PID belongs to our unreaped child, launched as
        // a session leader; negation targets only that private group. ESRCH
        // means the group is already empty, which is the goal.
        if unsafe { libc::kill(-(shared.pid as libc::pid_t), signal) } == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(ExecError::Supervisor {
                    stage: "kill-group".into(),
                    diagnostics: error.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Revoke the PID reservation, then reap.
    fn reap(&mut self) -> Result<(), ExecError> {
        self.shared.lock().reaped = true;
        loop {
            match self.child.wait() {
                Ok(_) => {
                    self.reaped = true;
                    self.token.clear_cancel_resource();
                    return Ok(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(ExecError::Supervisor {
                        stage: "reap".into(),
                        diagnostics: error.to_string(),
                    });
                }
            }
        }
    }
}

impl Drop for Running {
    /// An abandoned lease still tears the group down. There is no drain:
    /// nobody consumes a record here. Revoke-before-reap still holds.
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        let _ = self.signal_group(libc::SIGTERM);
        sleep_grace(self.grace);
        let _ = self.signal_group(libc::SIGKILL);
        let _ = self.reap();
    }
}

/// The bounded TERM→KILL grace, slept in slices like the Python supervisor.
fn sleep_grace(grace: Duration) {
    let slice = Duration::from_millis(50);
    let mut remaining = grace;
    while !remaining.is_zero() {
        let pause = remaining.min(slice);
        std::thread::sleep(pause);
        remaining -= pause;
    }
}
