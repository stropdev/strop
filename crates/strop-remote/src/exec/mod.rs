//! Remote work over plain `ssh` exec (plan 0090 §5): one multiplexed
//! OpenSSH connection per endpoint and quoted argv on the host; the
//! pooled SFTP session ([`crate::RemoteClient`]) is one more channel on
//! the same connection. Nothing of strop is installed on the host.
//!
//! The master authenticates once (`ssh -M -N -f`, `ControlPersist`) on a
//! socket in a private 0700 directory the caller owns; every exec,
//! terminal and SFTP session is a channel on it, so a later operation
//! costs a channel open, not a key exchange. Authentication, host keys
//! and the user's `~/.ssh/config` stay entirely with system OpenSSH under
//! the crate's one safety policy ([`crate::ssh`]): `BatchMode=yes` and
//! `StrictHostKeyChecking=yes` mean an unknown host key or a password
//! prompt is a typed refusal carrying ssh's own words, never a hang.
mod quote;

use crate::ssh::{multiplexed_argv, Mux};
use std::ffi::OsStr;
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use strop_core::process::OwnedProcess;
use strop_core::worker::CancelToken;
use strop_workspace::RemoteEndpoint;

/// How long an idle master outlives its last channel.
const PERSIST: &str = "10m";
/// `sun_path` is 108 bytes on Linux; OpenSSH appends a 17-byte
/// temporary suffix while it binds.
const MAX_SOCKET_PATH: usize = 90;
/// Bounded tail of ssh's own diagnostics retained for an error.
const DIAGNOSTIC_TAIL: u64 = 4096;

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("ssh control directory {}: {detail}", path.display())]
    ControlDir { path: PathBuf, detail: String },
    #[error("could not run ssh: {0}")]
    Spawn(#[source] io::Error),
    #[error("ssh to {endpoint} failed: {detail}")]
    Connect { endpoint: String, detail: String },
    #[error("connecting to {endpoint} was cancelled")]
    Cancelled { endpoint: String },
}

/// One endpoint's multiplexed connection: the names of its control
/// socket and log. Cloning never connects; the master is the background
/// `ssh` process those names lead to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecHost {
    endpoint: RemoteEndpoint,
    socket: PathBuf,
    log: PathBuf,
}

impl ExecHost {
    /// Reuse the endpoint's live master or authenticate a new one. The
    /// call returns once authentication finished (ssh forks only after
    /// it), so every failure — unknown host key, refused key, network —
    /// is reported here with ssh's own diagnostics. Cancelling `token`
    /// kills an authentication still in flight.
    pub fn connect(
        endpoint: &RemoteEndpoint,
        control_dir: &Path,
        token: &CancelToken,
    ) -> Result<Self, ExecError> {
        ensure_private_dir(control_dir)?;
        let name = socket_name(endpoint);
        let socket = control_dir.join(&name);
        if socket.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(ExecError::ControlDir {
                path: control_dir.to_path_buf(),
                detail: format!("too long for a socket path (limit {MAX_SOCKET_PATH} bytes)"),
            });
        }
        let host = Self {
            endpoint: endpoint.clone(),
            log: control_dir.join(format!("{name}.log")),
            socket,
        };
        if host.is_connected() {
            return Ok(host);
        }
        // A socket left by a master that died would make the new master
        // refuse to bind and silently run unmultiplexed.
        if let Err(error) = std::fs::remove_file(&host.socket) {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(ExecError::ControlDir {
                    path: host.socket.clone(),
                    detail: format!("stale control socket: {error}"),
                });
            }
        }
        std::fs::File::create(&host.log).map_err(|error| ExecError::ControlDir {
            path: host.log.clone(),
            detail: error.to_string(),
        })?;
        let master = Mux::Master {
            socket: &host.socket,
            persist: PERSIST,
            log: &host.log,
        };
        let mut command = Command::new("ssh");
        command
            .args(multiplexed_argv(endpoint, &master))
            .arg("--")
            .arg(endpoint.host())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process = OwnedProcess::spawn(&mut command, token)
            .map_err(|failure| ExecError::Spawn(io::Error::other(failure.message)))?;
        let status = process.wait();
        if token.is_cancelled() {
            return Err(ExecError::Cancelled {
                endpoint: endpoint.to_string(),
            });
        }
        match status {
            Ok(status) if status.success() => Ok(host),
            Ok(status) => Err(host.failure(&status.to_string())),
            Err(failure) => Err(host.failure(&failure.message)),
        }
    }

    pub fn endpoint(&self) -> &RemoteEndpoint {
        &self.endpoint
    }

    /// Whether the master answers on its socket (a local check; no
    /// network round trip).
    pub fn is_connected(&self) -> bool {
        self.socket.exists()
            && self
                .client(false)
                .args(["-O", "check", "--"])
                .arg(self.endpoint.host())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
    }

    /// `argv` on the host in `cwd` (the login directory when `None`),
    /// with all three pipes for the caller to own. Exit status 255 is
    /// ssh's own failure; [`Self::diagnostics`] then says why.
    pub fn command(&self, argv: &[&OsStr], cwd: Option<&Path>) -> Command {
        let mut command = self.channel(false, argv, cwd);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// Spawn [`Self::command`].
    pub fn exec(&self, argv: &[&OsStr], cwd: Option<&Path>) -> Result<Child, ExecError> {
        self.command(argv, cwd).spawn().map_err(ExecError::Spawn)
    }

    /// `argv` on a remote PTY (`ssh -tt`). Stdio is the caller's: run it
    /// on a local PTY so window-size changes reach the remote one.
    pub fn terminal(&self, argv: &[&OsStr], cwd: Option<&Path>) -> Command {
        self.channel(true, argv, cwd)
    }

    /// The SFTP subsystem as one channel on the master, all three pipes
    /// for the caller.
    pub(crate) fn sftp_command(&self) -> Command {
        let mut command = self.client(false);
        command
            .arg("-s")
            .arg("--")
            .arg(self.endpoint.host())
            .arg("sftp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// Stop the master (`ssh -O exit`); channels in flight end with it.
    pub fn disconnect(&self) -> Result<(), ExecError> {
        let status = self
            .client(false)
            .args(["-O", "exit", "--"])
            .arg(self.endpoint.host())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(ExecError::Spawn)?;
        if status.success() {
            Ok(())
        } else {
            Err(self.failure(&status.to_string()))
        }
    }

    /// The tail of ssh's own diagnostics for this endpoint.
    pub fn diagnostics(&self) -> Option<String> {
        let mut file = std::fs::File::open(&self.log).ok()?;
        let length = file.metadata().ok()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(DIAGNOSTIC_TAIL)))
            .ok()?;
        let mut tail = String::new();
        file.read_to_string(&mut tail).ok()?;
        let tail = tail.trim();
        (!tail.is_empty()).then(|| tail.to_string())
    }

    pub(crate) fn failure(&self, fallback: &str) -> ExecError {
        ExecError::Connect {
            endpoint: self.endpoint.to_string(),
            detail: self.diagnostics().unwrap_or_else(|| fallback.to_string()),
        }
    }

    fn client(&self, tty: bool) -> Command {
        let mut command = Command::new("ssh");
        command.args(multiplexed_argv(
            &self.endpoint,
            &Mux::Client {
                socket: &self.socket,
                log: &self.log,
                tty,
            },
        ));
        command
    }

    fn channel(&self, tty: bool, argv: &[&OsStr], cwd: Option<&Path>) -> Command {
        let words: Vec<&[u8]> = argv.iter().map(|word| word.as_bytes()).collect();
        let line = quote::remote_line(&words, cwd.map(|cwd| cwd.as_os_str().as_bytes()));
        let mut command = self.client(tty);
        command
            .arg("--")
            .arg(self.endpoint.host())
            .arg(OsStr::from_bytes(&line));
        command
    }
}

/// A stable short socket name per endpoint (FNV-1a over its URI): the
/// same endpoint finds the same master across editor restarts.
fn socket_name(endpoint: &RemoteEndpoint) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in endpoint.to_string().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Create `path` 0700 if missing; refuse one that is a symlink, not a
/// directory, someone else's, or open to group/other — a control socket
/// there would hand the authenticated connection to another user.
fn ensure_private_dir(path: &Path) -> Result<(), ExecError> {
    let refuse = |detail: String| ExecError::ControlDir {
        path: path.to_path_buf(),
        detail,
    };
    match std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
    {
        Ok(()) => {}
        Err(error) => return Err(refuse(error.to_string())),
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| refuse(error.to_string()))?;
    if !metadata.is_dir() {
        return Err(refuse("not a directory".into()));
    }
    if metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(refuse("owned by another user".into()));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| refuse(format!("cannot make it private: {error}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_names_are_stable_short_and_distinct() {
        let one = RemoteEndpoint::parse("ssh://dev@box:2222").unwrap();
        let other = RemoteEndpoint::parse("ssh://dev@box").unwrap();
        assert_eq!(socket_name(&one), socket_name(&one.clone()));
        assert_ne!(socket_name(&one), socket_name(&other));
        assert_eq!(socket_name(&one).len(), 16);
    }

    #[test]
    fn control_dir_is_created_private_and_tightened() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("state/ssh");
        ensure_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn control_dir_symlink_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(matches!(
            ensure_private_dir(&link),
            Err(ExecError::ControlDir { .. })
        ));
    }
}
