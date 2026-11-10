//! Whole-file read, save and listing over one SFTP session on the
//! endpoint's master (plan 0090 §5).
//!
//! A save uploads to a private temporary name beside the file, copies the
//! file's permission bits onto it and renames it over the original, so a
//! reader never sees a half-written file. It is refused when the file's
//! size or modification time moved since it was read — vim's "changed
//! since reading". There is no cross-process lock: like every other
//! editor, a writer racing between the check and the rename wins.
//! Ownership is not carried over (a root save of someone else's file
//! leaves it root-owned), and SFTP v3 mtimes are whole seconds, so a
//! same-size rewrite within the second of the read goes unnoticed.
use super::ExecError;
use crate::transport::error::{Fault, ReadFailureKind};
use crate::transport::session::kind_of;
use crate::transport::wire::DeploySftp;
use crate::RemoteEntryKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::runtime::Runtime;

/// Mode for a file the save creates (no original to copy from).
const NEW_FILE_MODE: u32 = 0o644;
const S_IFMT: u32 = 0o170_000;
const S_IFLNK: u32 = 0o120_000;

/// What a load observed: the identity a later save compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    /// Seconds since the epoch.
    pub mtime: u32,
}

impl std::fmt::Display for Stamp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} bytes, mtime {}", self.size, self.mtime)
    }
}

pub struct Loaded {
    pub bytes: Vec<u8>,
    pub stamp: Stamp,
}

/// What a save expects to find at the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// Save-as: refuse to replace an existing file.
    Absent,
    /// The file as it was loaded. A file deleted since is recreated
    /// (vim's `:w` does the same).
    Unchanged(Stamp),
    /// `:w!`: replace whatever is there.
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedEntry {
    pub name: PathBuf,
    pub kind: RemoteEntryKind,
    pub size: Option<u64>,
}

/// One SFTP session. Codec first: its pipes close before the runtime
/// that drives them, and the child (killed on drop) goes last.
pub struct RemoteFiles {
    session: DeploySftp<ChildStdin, ChildStdout>,
    runtime: Runtime,
    _child: Child,
}

static TEMPORARY: AtomicU64 = AtomicU64::new(0);

impl RemoteFiles {
    pub(super) fn start(command: Command) -> Result<Self, ExecError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(ExecError::Spawn)?;
        let mut command = tokio::process::Command::from(command);
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let (child, stdin, stdout) = {
            let _context = runtime.enter();
            let mut child = command.spawn().map_err(ExecError::Spawn)?;
            let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
                return Err(ExecError::Sftp("ssh pipes were not captured".into()));
            };
            (child, stdin, stdout)
        };
        let session = runtime
            .block_on(DeploySftp::connect(stdin, stdout))
            .map_err(sftp)?;
        Ok(Self {
            session,
            runtime,
            _child: child,
        })
    }

    /// The whole file, refused past `max` bytes.
    pub fn read(&mut self, path: &Path, max: u64) -> Result<Loaded, ExecError> {
        let (bytes, attrs) = self
            .runtime
            .block_on(self.session.read_whole(path, max))
            .map_err(sftp)?;
        let stamp = stamp(attrs.size, attrs.mtime)?;
        Ok(Loaded { bytes, stamp })
    }

    /// The symlink-following stamp, `None` when nothing is there.
    pub fn stat(&mut self, path: &Path) -> Result<Option<Stamp>, ExecError> {
        match self.runtime.block_on(self.session.stat_full(path)) {
            Ok(attrs) => stamp(attrs.size, attrs.mtime).map(Some),
            Err(fault) if fault.disposition().0 == ReadFailureKind::NotFound => Ok(None),
            Err(fault) => Err(sftp(fault)),
        }
    }

    /// Save `bytes` to `path` (through a symlink, to its target) under
    /// `expect`, returning the new stamp to compare the next save with.
    pub fn write(&mut self, path: &Path, bytes: &[u8], expect: Expect) -> Result<Stamp, ExecError> {
        let Self {
            session, runtime, ..
        } = self;
        runtime.block_on(save(session, path, bytes, expect))
    }

    /// Every entry of `directory` except `.` and `..`, unsorted.
    pub fn list(&mut self, directory: &Path) -> Result<Vec<ListedEntry>, ExecError> {
        let entries = self
            .runtime
            .block_on(self.session.list_entries(directory))
            .map_err(sftp)?;
        Ok(entries
            .into_iter()
            .map(|entry| ListedEntry {
                name: entry.name,
                kind: kind_of(entry.permissions),
                size: entry.size,
            })
            .collect())
    }
}

async fn save(
    session: &mut DeploySftp<ChildStdin, ChildStdout>,
    path: &Path,
    bytes: &[u8],
    expect: Expect,
) -> Result<Stamp, ExecError> {
    let mut target = path.to_path_buf();
    let mut existing = optional(session.lstat(path).await)?;
    if existing
        .as_ref()
        .and_then(|attrs| attrs.permissions)
        .is_some_and(|mode| mode & S_IFMT == S_IFLNK)
    {
        target = session.canonical(path).await.map_err(sftp)?;
        existing = optional(session.lstat(&target).await)?;
    }
    let found = match &existing {
        Some(attrs) => Some(stamp(attrs.size, attrs.mtime)?),
        None => None,
    };
    match (expect, found) {
        (Expect::Absent, Some(_)) => return Err(ExecError::Exists { path: target }),
        (Expect::Unchanged(expected), Some(found)) if found != expected => {
            return Err(ExecError::ChangedOnDisk {
                path: target,
                expected,
                found: Some(found),
            })
        }
        _ => {}
    }
    let mode = existing
        .and_then(|attrs| attrs.permissions)
        .map_or(NEW_FILE_MODE, |mode| mode & 0o7777);
    let temporary = temporary_beside(&target);
    let staged = async {
        session.upload(&temporary, bytes, mode).await?;
        if session.overwrites_on_rename() {
            return session.publish(&temporary, &target).await;
        }
        // SFTP v3 rename refuses an existing destination; without
        // posix-rename the replacement cannot be atomic.
        if found.is_some() {
            session.remove(&target).await?;
        }
        session.rename(&temporary, &target).await?;
        session.lstat(&target).await
    }
    .await;
    match staged {
        Ok(attrs) => stamp(attrs.size, attrs.mtime),
        Err(fault) => Err(sftp(match session.remove(&temporary).await {
            Ok(()) => fault,
            Err(cleanup) => fault.with_cleanup(cleanup),
        })),
    }
}

/// `.<name>.strop-<pid>-<n>~` in the same directory, so the rename never
/// crosses a filesystem.
fn temporary_beside(target: &Path) -> PathBuf {
    let mut name = std::ffi::OsString::from(".");
    name.push(target.file_name().unwrap_or_default());
    name.push(format!(
        ".strop-{}-{}~",
        std::process::id(),
        TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    target.with_file_name(name)
}

fn optional<T>(result: Result<T, Fault>) -> Result<Option<T>, ExecError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(fault) if fault.disposition().0 == ReadFailureKind::NotFound => Ok(None),
        Err(fault) => Err(sftp(fault)),
    }
}

fn stamp(size: Option<u64>, mtime: Option<u32>) -> Result<Stamp, ExecError> {
    match (size, mtime) {
        (Some(size), Some(mtime)) => Ok(Stamp { size, mtime }),
        _ => Err(ExecError::Sftp(
            "the server reported no size or modification time".into(),
        )),
    }
}

fn sftp(fault: Fault) -> ExecError {
    ExecError::Sftp(fault.detail().to_string())
}

#[cfg(test)]
mod tests {
    use super::temporary_beside;
    use std::path::Path;

    #[test]
    fn temporaries_stay_beside_the_target_and_are_unique() {
        let one = temporary_beside(Path::new("/srv/app/main.rs"));
        let two = temporary_beside(Path::new("/srv/app/main.rs"));
        assert_eq!(one.parent(), Some(Path::new("/srv/app")));
        assert!(one
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(".main.rs.strop-"));
        assert_ne!(one, two);
    }
}
