//! Owned PTY launch (0058 WK12): the same supervised-lease semantics as
//! [`super::launch`] — byte-exact argv, readiness handshake, revoke-
//! before-reap group teardown — but the target runs on a freshly
//! allocated pseudo-terminal it owns as controlling terminal.
//!
//! The child leads a new session (`setsid`) and takes the PTY slave as
//! its controlling terminal (`TIOCSCTTY`) with the admitted geometry;
//! stdin, stdout and stderr are all the slave, exactly what a terminal
//! consumer expects. The worker keeps the master: reads are the
//! child's merged output, writes are its input, `TIOCSWINSZ` resizes
//! it. There is no half-close on a PTY; revocation is the supervised
//! TERM/grace/KILL group teardown, after which the master reports EIO
//! and output ends truthfully.

use super::{ExecError, ExecSpec, Running};
use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd};
use strop_core::worker::CancelToken;

/// One running PTY target: the supervised child plus its master. The
/// master's open file description is shared by the read/write halves,
/// so a full input buffer applies backpressure to writers while reads
/// keep draining (full-duplex; neither direction stalls the other).
pub struct RunningPty {
    running: Running,
    master: File,
}

/// The admitted terminal geometry in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub columns: u16,
    pub rows: u16,
}

impl RunningPty {
    /// The master side of the child's terminal. Clone per consumer
    /// (reader, writer, resizer); all clones share one description.
    pub fn master(&self) -> &File {
        &self.master
    }

    /// The supervised target: revocation, settlement and reaping are
    /// the unchanged supervisor contract.
    pub fn running(&mut self) -> &mut Running {
        &mut self.running
    }

    pub fn into_running(self) -> Running {
        self.running
    }
}

impl std::fmt::Debug for RunningPty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningPty").finish_non_exhaustive()
    }
}

/// Apply a new geometry: `TIOCSWINSZ` on the master, which SIGWINCHes
/// the foreground process group. Honest errors — a dead terminal
/// reports EIO and the caller classifies it, never a guessed success.
pub fn resize(master: &File, size: PtySize) -> Result<(), ExecError> {
    let winsize = libc::winsize {
        ws_row: size.rows,
        ws_col: size.columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `master` is a live PTY master and `winsize` valid storage.
    if unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ as _, &winsize) } != 0 {
        return Err(ExecError::Supervisor {
            stage: "resize".into(),
            diagnostics: std::io::Error::last_os_error().to_string(),
        });
    }
    Ok(())
}

/// Launch one admitted spec on a fresh PTY under `token`'s lease.
/// Readiness matches [`super::launch`]: the chdir/exec handshake
/// resolves before return, so a launch failure is typed and never
/// masquerades as an exit. A token cancelled mid-spawn revokes the
/// fresh target.
pub fn launch_pty(
    spec: &ExecSpec,
    size: PtySize,
    token: &CancelToken,
) -> Result<RunningPty, ExecError> {
    use std::ffi::{CString, OsStr};
    use std::os::unix::ffi::OsStrExt;
    use std::process::{Command, Stdio};
    if token.is_cancelled() {
        return Err(ExecError::Cancelled);
    }
    let winsize = rustix::termios::Winsize {
        ws_row: size.rows,
        ws_col: size.columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pair =
        rustix_openpty::openpty(None, Some(&winsize)).map_err(|error| ExecError::Supervisor {
            stage: "openpty".into(),
            diagnostics: error.to_string(),
        })?;
    let master = File::from(pair.controller);
    let slave = File::from(pair.user);
    // The chdir/exec handshake, identical to the pipe supervisor's: the
    // child reports a chdir failure as `C` + errno so the parent tells
    // a typed launch failure from an exit.
    let (handshake_read, handshake_write) = {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: `fds` is valid writable storage for two descriptors.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(ExecError::Supervisor {
                stage: "pipe".into(),
                diagnostics: std::io::Error::last_os_error().to_string(),
            });
        }
        // SAFETY: both descriptors are live from the successful pipe
        // call; FD_CLOEXEC on a live descriptor cannot fail.
        unsafe {
            libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC);
        }
        use std::os::fd::FromRawFd;
        // SAFETY: the descriptors are owned by this scope and are
        // transferred exactly once.
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
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
    for (name, value) in spec.env() {
        command.env(OsStr::from_bytes(name), OsStr::from_bytes(value));
    }
    command
        .stdin(Stdio::from(slave.try_clone().map_err(|error| {
            ExecError::Supervisor {
                stage: "slave".into(),
                diagnostics: error.to_string(),
            }
        })?))
        .stdout(Stdio::from(slave.try_clone().map_err(|error| {
            ExecError::Supervisor {
                stage: "slave".into(),
                diagnostics: error.to_string(),
            }
        })?))
        .stderr(Stdio::from(slave));
    {
        // SAFETY: runs in the forked child before exec; only
        // async-signal-safe calls (signal, pthread_sigmask, setsid,
        // ioctl, chdir, write) and no allocation — the CString was
        // built before the fork.
        let handshake = handshake_write.as_raw_fd();
        unsafe {
            command.pre_exec(move || {
                for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGPIPE, libc::SIGTERM] {
                    libc::signal(signal, libc::SIG_DFL);
                }
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
                // New session; stdin (the slave) becomes its controlling
                // terminal, so job control and SIGHUP semantics are the
                // real ones a terminal consumer expects.
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
    }
    let running =
        super::spawn_with_handshake(command, spec, token, handshake_read, handshake_write)?;
    Ok(RunningPty { running, master })
}
