//! Worker-only child ownership. Cancellation revokes a private Unix process group
//! or a helper's lease; only its owner may revoke the capability and reap the PID.
mod capture;
use crate::worker::{CancelToken, Failure, FailureKind};
pub use capture::{
    capture, capture_with, stream_with, CaptureError, CapturePolicy, CommandOutput, StdinPolicy,
    StreamError, StreamOutput, StreamPolicy,
};
use parking_lot::Mutex;
use std::io;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};
use std::sync::Arc;

#[derive(Default)]
struct Group {
    pid: Option<u32>,
    cancelled: bool,
    #[cfg(unix)]
    lease: Option<std::os::unix::net::UnixStream>,
}
impl Group {
    fn signal(&self) -> Result<(), Failure> {
        #[cfg(unix)]
        if let Some(lease) = &self.lease {
            return match lease.shutdown(std::net::Shutdown::Write) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotConnected => Ok(()),
                Err(error) => Err(Failure::new(
                    FailureKind::Io,
                    format!("close process lease: {error}"),
                )),
            };
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            // SAFETY: this positive PID belongs to our unreaped child, launched
            // with process_group(0). The mutex serializes signalling and revocation;
            // negation targets only that private group. std cannot signal groups.
            if unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) } == -1 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(Failure::new(
                        FailureKind::Io,
                        format!("kill process group: {error}"),
                    ));
                }
            }
        }
        Ok(())
    }
    fn cancel(&mut self) -> Result<(), Failure> {
        self.cancelled = true;
        self.signal()
    }
}

/// Owns both the child and the cancellation capability. Never put this in editor
/// state: Drop may wait, so it belongs exclusively to a worker stack.
pub struct OwnedProcess {
    child: Child,
    group: Arc<Mutex<Group>>,
    token: CancelToken,
    reaped: bool,
}
impl OwnedProcess {
    pub fn spawn(command: &mut Command, token: &CancelToken) -> Result<Self, Failure> {
        #[cfg(not(unix))]
        return Err(Failure::new(
            FailureKind::Unavailable,
            "process-group supervision requires Unix",
        ));
        #[cfg(unix)]
        Self::spawn_unix(command, token, None)
    }

    /// The helper must own descendant cleanup and exit after lease EOF.
    /// Unlike an ordinary group child, it may establish a controlling session.
    #[cfg(unix)]
    pub fn spawn_leased(
        command: &mut Command,
        token: &CancelToken,
        lease: std::os::unix::net::UnixStream,
    ) -> Result<Self, Failure> {
        Self::spawn_unix(command, token, Some(lease))
    }

    #[cfg(unix)]
    fn spawn_unix(
        command: &mut Command,
        token: &CancelToken,
        lease: Option<std::os::unix::net::UnixStream>,
    ) -> Result<Self, Failure> {
        use std::os::unix::process::CommandExt;
        if lease.is_none() {
            command.process_group(0);
        }
        let group = Arc::new(Mutex::new(Group {
            lease,
            ..Group::default()
        }));
        let callback = group.clone();
        token.register_cancel_resource(move || callback.lock().cancel())?;
        if token.is_cancelled() {
            token.clear_cancel_resource();
            return Err(Failure::new(
                FailureKind::Unavailable,
                "process cancelled before spawn",
            ));
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                token.clear_cancel_resource();
                return Err(Failure::new(FailureKind::Spawn, error.to_string()));
            }
        };
        let process = Self {
            child,
            group,
            token: token.clone(),
            reaped: false,
        };
        {
            let mut group = process.group.lock();
            group.pid = Some(process.child.id());
            if group.cancelled || token.is_cancelled() {
                group.cancel()?;
            }
        }
        Ok(process)
    }
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }
    pub fn terminate(&mut self) -> Result<(), Failure> {
        self.group.lock().signal()
    }

    /// Observe without reaping: descendants may still hold pipes, and cancellation
    /// must retain its PID reservation until those descendants have been killed.
    pub fn has_exited(&self) -> Result<bool, Failure> {
        child_has_exited(&self.child)
    }
    /// Wait with cancellation intact, then revoke before reaping. A callback
    /// detached from CancelToken can never signal a reused PID.
    pub fn wait(&mut self) -> Result<ExitStatus, Failure> {
        // Retain the signalling capability throughout the wait. Even a caller
        // waiting before termination must remain cancellable.
        while !self.reaped && !self.has_exited()? {
            std::thread::park_timeout(std::time::Duration::from_millis(20));
        }
        self.group.lock().pid = None;
        loop {
            match self.child.wait() {
                Ok(status) => {
                    self.reaped = true;
                    self.token.clear_cancel_resource();
                    return Ok(status);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(Failure::new(FailureKind::Wait, error.to_string())),
            }
        }
    }
}
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.terminate();
            // A leased helper must finish its own cross-group cleanup; killing it
            // here would bypass that ownership protocol and orphan its descendants.
            #[cfg(unix)]
            if self.group.lock().lease.is_none() {
                let _ = self.child.kill();
            }
            #[cfg(not(unix))]
            let _ = self.child.kill();
            let _ = self.wait();
        }
        self.token.clear_cancel_resource();
    }
}

/// Worker-only observation that retains an owned child's PID reservation.
/// Call before `Child::wait`/`try_wait`; reaping ends that reservation.
pub fn child_has_exited(child: &Child) -> Result<bool, Failure> {
    #[cfg(not(unix))]
    return Err(Failure::new(
        FailureKind::Unavailable,
        "process supervision requires Unix",
    ));
    #[cfg(unix)]
    {
        // SAFETY: zero initializes siginfo_t; waitid writes it on success.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        loop {
            // SAFETY: child is owned and unreaped. WNOWAIT preserves its PID;
            // WNOHANG prevents blocking. The output pointer is valid.
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    child.id() as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result == 0 {
                // SAFETY: successful waitid initialized the platform layout.
                return Ok(unsafe { info.si_pid() } != 0);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(Failure::new(FailureKind::Wait, error.to_string()));
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::worker::{self, CancelReason, Outcome};
    use std::io::Read;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn leased_cancellation_allows_helper_cleanup_before_reaping() {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = worker::spawn_effect(
            "leased-cleanup-test",
            move |outcome| {
                done_tx.send(outcome).unwrap();
            },
            move |token| {
                let (lease, child_lease) = UnixStream::pair().unwrap();
                let mut command = Command::new("/bin/sh");
                command
                    .args(["-c", "printf READY; cat >/dev/null; printf CLEANED"])
                    .stdin(Stdio::from(OwnedFd::from(child_lease)))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                let mut process = OwnedProcess::spawn_leased(&mut command, &token, lease).unwrap();
                let mut stdout = process.take_stdout().unwrap();
                let mut ready = [0; 5];
                stdout.read_exact(&mut ready).unwrap();
                assert_eq!(&ready, b"READY");
                ready_tx.send(()).unwrap();
                let mut output = Vec::new();
                stdout.read_to_end(&mut output).unwrap();
                let status = process.wait().unwrap();
                Outcome::Success((output, status.success()))
            },
        );
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.cancel(CancelReason::Shutdown);
        let outcome = done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let Outcome::Success((output, successful)) = outcome else {
            panic!("cleanup outcome lost: {outcome:?}");
        };
        assert!(successful);
        assert_eq!(output, b"CLEANED");
    }
}
