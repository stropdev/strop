//! Process-group supervision of one PTY child, from spawn to the final
//! group cleanup and reap.
use super::{classify_launch, PtyError};
use crate::launch::Launch;
use parking_lot::Mutex;
use std::os::fd::OwnedFd;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};
use strop_core::worker::CancelToken;

/// A recorded exit within this last-chance drain beats a racing cancel.
const DRAIN: Duration = Duration::from_millis(300);
/// Wait-loop poll cadence.
const POLL: Duration = Duration::from_millis(20);

/// The child's observed exit state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusRecord {
    /// Exited with this code; non-zero codes are data, not failures.
    Exited(u32),
    /// Died on this signal.
    Signaled(u32),
}

/// The supervised child's settled outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settlement {
    /// The child's own terminal state, recorded before any teardown.
    Recorded(StatusRecord),
    /// Revoked: SIGTERM, the bounded grace, SIGKILL, then reap. A child
    /// that exits on the TERM is still revoked; the drain precedes it.
    Revoked,
}

struct Shared {
    pid: u32,
    cancelled: bool,
    reaped: bool,
}

/// One running supervised child. [`Drop`] revokes and may wait out the
/// grace, so it lives on a helper thread, never in editor state.
pub(crate) struct Running {
    child: Child,
    grace: Duration,
    shared: Arc<Mutex<Shared>>,
    token: CancelToken,
    reaped: bool,
}

/// The spawn tail of [`super::spawn`]: cancel latch, spawn, typed launch
/// classification. The caller built `command` (stdio, session setup) and
/// hands over both handshake pipe ends.
pub(super) fn spawn_with_handshake(
    mut command: Command,
    launch: &Launch,
    grace: Duration,
    token: &CancelToken,
    handshake_read: OwnedFd,
    handshake_write: OwnedFd,
) -> Result<Running, PtyError> {
    use std::os::unix::ffi::OsStrExt;
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
        .map_err(|error| PtyError::os("cancel-registration", error.message))?;
    if token.is_cancelled() {
        token.clear_cancel_resource();
        return Err(PtyError::Cancelled);
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            token.clear_cancel_resource();
            // Without closing the parent's write end the classifying read
            // would block instead of seeing the dead child's EOF.
            drop(handshake_write);
            return Err(classify_launch(
                &handshake_read,
                error,
                launch.program.as_bytes(),
            ));
        }
    };
    drop(handshake_read);
    drop(handshake_write);
    shared.lock().pid = child.id();
    let running = Running {
        child,
        grace,
        shared,
        token: token.clone(),
        reaped: false,
    };
    if running.shared.lock().cancelled || token.is_cancelled() {
        // Cancelled mid-spawn: revoke the fresh child before reporting.
        let _ = running.revoke();
        return Err(PtyError::Cancelled);
    }
    Ok(running)
}

impl Running {
    /// The child's PID, which is also its process-group identity.
    #[cfg(test)]
    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    /// Wait for the child to settle, then kill the group and reap.
    /// Cancellation enters the revocation path; an already-recorded exit
    /// beats it.
    pub(crate) fn wait(self) -> Result<Settlement, PtyError> {
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

    /// Last-chance drain, then TERM, the bounded grace, KILL, and only
    /// then reap.
    pub(crate) fn revoke(mut self) -> Result<Settlement, PtyError> {
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

    /// The child's terminal state without reaping it: the zombie keeps
    /// the PID — and PGID — reservation alive.
    pub(crate) fn poll_record(&self) -> Result<Option<StatusRecord>, PtyError> {
        // SAFETY: an all-zero siginfo_t is a valid value; waitid
        // overwrites it on success.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        loop {
            // SAFETY: `info` is valid writable storage. WNOWAIT keeps the
            // PID reservation; WNOHANG never blocks.
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
                return Err(PtyError::os("wait", error));
            }
        }
        // SAFETY: a successful waitid initialized the platform layout.
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        // SAFETY: as above; si_pid is non-zero, so the status is set.
        let status = unsafe { info.si_status() } as u32;
        match info.si_code {
            libc::CLD_EXITED => Ok(Some(StatusRecord::Exited(status))),
            libc::CLD_KILLED | libc::CLD_DUMPED => Ok(Some(StatusRecord::Signaled(status))),
            other => Err(PtyError::os(
                "wait",
                format!("unexpected wait status {other}"),
            )),
        }
    }

    /// Recorded exit: descendants die before the child is reaped.
    fn settle(mut self, record: StatusRecord) -> Result<Settlement, PtyError> {
        self.signal_group(libc::SIGKILL)?;
        self.reap()?;
        Ok(Settlement::Recorded(record))
    }

    fn signal_group(&self, signal: libc::c_int) -> Result<(), PtyError> {
        let shared = self.shared.lock();
        if shared.reaped {
            return Ok(());
        }
        debug_assert_ne!(shared.pid, 0);
        // SAFETY: this positive PID belongs to our unreaped child, launched
        // as a session leader; negation targets only that private group.
        // ESRCH means the group is already empty.
        if unsafe { libc::kill(-(shared.pid as libc::pid_t), signal) } == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            return Err(PtyError::os("kill-group", error));
        }
        Ok(())
    }

    /// Give up the PID reservation, then reap.
    fn reap(&mut self) -> Result<(), PtyError> {
        self.shared.lock().reaped = true;
        loop {
            match self.child.wait() {
                Ok(_) => {
                    self.reaped = true;
                    self.token.clear_cancel_resource();
                    return Ok(());
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(PtyError::os("reap", error)),
            }
        }
    }
}

impl Drop for Running {
    /// An abandoned child still has its group torn down, revoke before
    /// reap; nobody consumes a record here, so there is no drain.
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

/// Sleep the bounded TERM→KILL grace in slices.
fn sleep_grace(grace: Duration) {
    let slice = Duration::from_millis(50);
    let mut remaining = grace;
    while !remaining.is_zero() {
        let pause = remaining.min(slice);
        std::thread::sleep(pause);
        remaining -= pause;
    }
}
