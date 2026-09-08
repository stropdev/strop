//! Tokio adapter for the same unreaped-process-group invariant as
//! strop-core::OwnedProcess. The child is private: no caller can reap it
//! before this owner signals the group. Drop is the panic backstop.
use std::io;
use std::time::Duration;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};

pub(super) struct ServerProcess {
    child: Child,
}
impl ServerProcess {
    pub(super) fn new(child: Child) -> Self {
        Self { child }
    }
    pub(super) fn id(&self) -> Option<u32> {
        self.child.id()
    }
    pub(super) fn take_io(&mut self) -> io::Result<(ChildStdout, ChildStdin, ChildStderr)> {
        match (
            self.child.stdout.take(),
            self.child.stdin.take(),
            self.child.stderr.take(),
        ) {
            (Some(stdout), Some(stdin), Some(stderr)) => Ok((stdout, stdin, stderr)),
            _ => Err(io::Error::other(
                "language server is missing a configured stdio pipe",
            )),
        }
    }
    fn signal_group(&mut self) -> io::Result<()> {
        let Some(pid) = self.child.id() else {
            return Ok(());
        };
        #[cfg(unix)]
        {
            let pid = libc::pid_t::try_from(pid)
                .map_err(|_| io::Error::other("child PID is outside the platform range"))?;
            // SAFETY: this owner retains the unreaped child; spawn set a private
            // process group with its PID. No safe Rust API signals a Unix group.
            if unsafe { libc::kill(-pid, libc::SIGKILL) } == -1 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error);
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            self.child.start_kill()?;
        }
        Ok(())
    }
    pub(super) async fn finish(
        &mut self,
        drain: Option<tokio::task::JoinHandle<()>>,
        grace: Duration,
    ) -> io::Result<()> {
        // Stderr EOF can acknowledge remote supervisor exit without reaping
        // ssh. Keep its PID reserved until local ProxyCommand children die.
        if let Some(mut drain) = drain {
            match tokio::time::timeout(grace, &mut drain).await {
                Ok(Ok(())) | Err(_) => {}
                Ok(Err(error)) => strop_trace::record_with(
                    strop_trace::EventKind::Error,
                    || serde_json::json!({"source":"lsp_stderr_task","message":error.to_string()}),
                ),
            }
        }
        self.signal_group()?;
        tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error))??;
        Ok(())
    }
}
impl Drop for ServerProcess {
    fn drop(&mut self) {
        if let Err(error) = self.signal_group() {
            strop_trace::record_with(
                strop_trace::EventKind::Error,
                || serde_json::json!({"source":"lsp_process_cleanup","message":error.to_string()}),
            );
        }
    }
}
