//! Worker-only child ownership. Cancellation signals a private Unix process
//! group promptly; only its owner may revoke the capability and reap the PID.
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
}
impl Group {
    fn signal(&self) -> Result<(), Failure> {
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
        {
            use std::os::unix::process::CommandExt;
            let group = Arc::new(Mutex::new(Group::default()));
            let callback = group.clone();
            token.register_cancel_resource(move || callback.lock().cancel())?;
            if token.is_cancelled() {
                token.clear_cancel_resource();
                return Err(Failure::new(
                    FailureKind::Unavailable,
                    "process cancelled before spawn",
                ));
            }
            let child = match command.process_group(0).spawn() {
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
                // A callback may already have run before publication.
                if group.cancelled || token.is_cancelled() {
                    group.cancel()?;
                }
            }
            Ok(process)
        }
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
        #[cfg(not(unix))]
        return Err(Failure::new(
            FailureKind::Unavailable,
            "process supervision requires Unix",
        ));
        #[cfg(unix)]
        {
            // SAFETY: zero initializes siginfo_t; successful waitid writes it.
            // WNOWAIT retains ownership of the child PID, WNOHANG never blocks.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            loop {
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        self.child.id() as libc::id_t,
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                };
                if result == 0 {
                    // SAFETY: waitid successfully initialized the platform layout.
                    return Ok(unsafe { info.si_pid() } != 0);
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(Failure::new(FailureKind::Wait, error.to_string()));
                }
            }
        }
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
            // Direct-child fallback if process-group signalling failed.
            let _ = self.child.kill();
            let _ = self.wait();
        }
        self.token.clear_cancel_resource();
    }
}
