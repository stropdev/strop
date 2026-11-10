//! Owned PTY launch and supervision for one terminal child.
//!
//! The child leads a new session (`setsid`) and takes the PTY slave as
//! its controlling terminal (`TIOCSCTTY`) at the requested geometry;
//! stdin, stdout and stderr are all the slave. The caller keeps the
//! master: reads are the child's merged output, writes are its input,
//! `TIOCSWINSZ` resizes it. Its PID is the process-group identity of
//! every descendant, reserved by the unreaped zombie until cleanup.
//!
//! - **Typed launch failures.** A chdir/exec handshake resolves before
//!   [`spawn`] returns: a missing directory, a missing program or a
//!   non-executable file is a [`PtyError::Launch`], never a fake exit.
//! - **Environment overlay.** The launch's variables are set on top of
//!   this process's inherited environment.
//! - **Revoke-before-reap.** Cancelling the token latches; a 300 ms
//!   last-chance drain lets an already-recorded exit win, otherwise the
//!   group gets SIGTERM, the bounded grace, SIGKILL, and only then is the
//!   child reaped. A recorded exit still SIGKILLs leftover descendants
//!   before the reap. Descendants that call `setsid()` themselves escape
//!   a process-group kill.
mod supervisor;
#[cfg(test)]
mod tests;

pub(crate) use supervisor::{Running, Settlement, StatusRecord};

use crate::launch::Launch;
use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Duration;
use strop_core::worker::CancelToken;

/// TERM→KILL grace when a session is revoked.
pub(crate) const DEFAULT_GRACE: Duration = Duration::from_millis(2_000);

/// Why a PTY child could not be spawned, supervised or settled. No
/// variant guesses success.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PtyError {
    #[error("terminal launch refused: {0}")]
    Invalid(&'static str),
    #[error("the terminal child could not start: {diagnostics}")]
    Launch { diagnostics: String },
    #[error("terminal supervision failed at {stage}: {diagnostics}")]
    Supervisor {
        stage: &'static str,
        diagnostics: String,
    },
    #[error("the terminal was cancelled before its child launched")]
    Cancelled,
}

impl PtyError {
    fn os(stage: &'static str, error: impl std::fmt::Display) -> Self {
        Self::Supervisor {
            stage,
            diagnostics: error.to_string(),
        }
    }
}

/// Terminal geometry in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PtySize {
    pub columns: u16,
    pub rows: u16,
}

/// One running PTY child plus its master. The master's open file
/// description is shared by every clone, so a full input buffer applies
/// backpressure to writers while reads keep draining.
pub(crate) struct RunningPty {
    running: Running,
    master: File,
}

impl RunningPty {
    /// Clone per consumer (reader, writer, resizer); all clones share
    /// one description.
    pub(crate) fn master(&self) -> &File {
        &self.master
    }

    pub(crate) fn into_running(self) -> Running {
        self.running
    }
}

/// `TIOCSWINSZ` on the master, which SIGWINCHes the foreground process
/// group. A dead terminal reports its errno, never a guessed success.
pub(crate) fn resize(master: &File, size: PtySize) -> Result<(), PtyError> {
    let winsize = libc::winsize {
        ws_row: size.rows,
        ws_col: size.columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `master` is a live PTY master and `winsize` valid storage
    // for the duration of the call.
    if unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ as _, &winsize) } != 0 {
        return Err(PtyError::os("resize", std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Spawn `launch` on a fresh PTY of `size` under `token`'s cancellation.
/// Returns once the exec handshake resolved; a token cancelled mid-spawn
/// revokes the fresh child.
pub(crate) fn spawn(
    launch: &Launch,
    size: PtySize,
    grace: Duration,
    token: &CancelToken,
) -> Result<RunningPty, PtyError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    if token.is_cancelled() {
        return Err(PtyError::Cancelled);
    }
    if launch.program.is_empty() {
        return Err(PtyError::Invalid("empty program"));
    }
    let winsize = rustix::termios::Winsize {
        ws_row: size.rows,
        ws_col: size.columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pair = rustix_openpty::openpty(None, Some(&winsize))
        .map_err(|error| PtyError::os("openpty", error))?;
    let master = File::from(pair.controller);
    let slave = File::from(pair.user);
    // The child reports a chdir failure as `C` + errno before exiting, so
    // the parent tells it apart from an exec failure.
    let (handshake_read, handshake_write) =
        rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
            .map_err(|error| PtyError::os("pipe", std::io::Error::from(error)))?;
    // Built before the fork: the pre-exec hook must not allocate.
    let cwd = CString::new(launch.directory.as_os_str().as_bytes())
        .map_err(|_| PtyError::Invalid("NUL in the working directory"))?;
    let mut command = Command::new(&launch.program);
    command.args(&launch.arguments);
    for (name, value) in &launch.environment {
        command.env(name, value);
    }
    let stdio = |slave: &File| {
        slave
            .try_clone()
            .map(Stdio::from)
            .map_err(|error| PtyError::os("slave", error))
    };
    command
        .stdin(stdio(&slave)?)
        .stdout(stdio(&slave)?)
        .stderr(Stdio::from(slave));
    let handshake = handshake_write.as_raw_fd();
    // SAFETY: the hook runs in the forked child before exec and makes only
    // async-signal-safe calls (signal, sigemptyset, pthread_sigmask,
    // setsid, ioctl, chdir, write) on storage it owns; nothing allocates —
    // the CString was built before the fork.
    unsafe {
        command.pre_exec(move || {
            for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGPIPE, libc::SIGTERM] {
                libc::signal(signal, libc::SIG_DFL);
            }
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
            // New session; stdin (the slave) becomes its controlling
            // terminal, so job control and SIGHUP are the real ones.
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
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
    let running = supervisor::spawn_with_handshake(
        command,
        launch,
        grace,
        token,
        handshake_read,
        handshake_write,
    )?;
    Ok(RunningPty { running, master })
}

/// Turn a failed spawn into the typed launch failure: the chdir record
/// wins over the exec errno, then the classic errno meanings apply.
fn classify_launch(handshake: &OwnedFd, error: std::io::Error, program: &[u8]) -> PtyError {
    let mut record = [0_u8; 5];
    // The child is dead, so its write end is closed: this read returns
    // the record or EOF immediately.
    if rustix::io::read(handshake, &mut record).unwrap_or(0) == record.len() && record[0] == b'C' {
        let errno = u32::from_le_bytes([record[1], record[2], record[3], record[4]]);
        return PtyError::Launch {
            diagnostics: format!("chdir failed (errno {errno})"),
        };
    }
    let program = String::from_utf8_lossy(program);
    let diagnostics = match error.raw_os_error() {
        Some(libc::ENOENT) => format!("not-found {program}"),
        Some(libc::EACCES) => format!("not-executable {program}"),
        _ => format!("os {error}"),
    };
    PtyError::Launch { diagnostics }
}
