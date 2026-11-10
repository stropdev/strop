//! Saving a whole file over the endpoint's SFTP session (plan 0090 §5).
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
use super::error::{Fault, RemoteFailureKind, RemoteStage};
use super::wire::{Attrs, Sftp};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncRead, AsyncWrite};

/// Mode for a file the save creates (no original to copy from).
const NEW_FILE_MODE: u32 = 0o644;
const S_IFMT: u32 = 0o170_000;
const S_IFLNK: u32 = 0o120_000;

/// What a load observed: the identity a later save compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// What a save expects to find at the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Expect {
    /// Save-as: refuse to replace an existing file.
    Absent,
    /// The file as it was loaded. A file deleted since is recreated
    /// (vim's `:w` does the same).
    Unchanged(Stamp),
    /// `:w!`: replace whatever is there.
    Any,
}

static TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Three round trips for a regular file: LSTAT with the temporary's
/// creation, then the writes, close and mode, then the rename with the
/// published attributes. A symlink is resolved first and saved through.
pub(crate) async fn save<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    session: &mut Sftp<W, R>,
    path: &Path,
    bytes: &[u8],
    expect: Expect,
) -> Result<Stamp, Fault> {
    let temporary = temporary_beside(path);
    let (existing, created) = session.inspect_and_create(path, &temporary).await?;
    let existing = optional(existing);
    let linked = matches!(&existing, Ok(Some(attrs)) if is_link(attrs));
    let admitted = existing.and_then(|existing| match linked {
        true => Ok(None),
        false => admit(path, expect, existing).map(Some),
    });
    let (destination, handle) = match (admitted, created) {
        (Ok(Some(destination)), Ok(handle)) => (destination, handle),
        (admitted, Ok(handle)) => {
            let discarded = session.discard(handle, &temporary).await;
            admitted?;
            discarded?;
            return save_through_link(session, path, bytes, expect).await;
        }
        (admitted, Err(fault)) => {
            if linked {
                return save_through_link(session, path, bytes, expect).await;
            }
            admitted?;
            return Err(fault);
        }
    };
    let staged = session
        .upload_into(handle, &temporary, bytes, destination.mode)
        .await;
    let staged = match staged {
        Ok(()) => publish(session, &temporary, path, destination.found.is_some()).await,
        Err(fault) => Err(fault),
    };
    settle(session, &temporary, staged).await
}

/// The slow path for a symlink: resolve it, then stage beside the
/// target so the rename stays on one filesystem.
async fn save_through_link<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    session: &mut Sftp<W, R>,
    path: &Path,
    bytes: &[u8],
    expect: Expect,
) -> Result<Stamp, Fault> {
    let target = session.expand_path(path).await?;
    let existing = optional(session.lstat(&target).await)?;
    let destination = admit(&target, expect, existing)?;
    let temporary = temporary_beside(&target);
    let staged = match session.upload(&temporary, bytes, destination.mode).await {
        Ok(()) => publish(session, &temporary, &target, destination.found.is_some()).await,
        Err(fault) => Err(fault),
    };
    settle(session, &temporary, staged).await
}

struct Destination {
    found: Option<Stamp>,
    mode: u32,
}

/// Whether `expect` allows replacing what is at `path`.
fn admit(path: &Path, expect: Expect, existing: Option<Attrs>) -> Result<Destination, Fault> {
    let found = match &existing {
        Some(attrs) => Some(stamp(attrs)?),
        None => None,
    };
    match (expect, found) {
        (Expect::Absent, Some(_)) => {
            return Err(Fault::new(
                RemoteStage::Inspect,
                RemoteFailureKind::Exists,
                format!("{} already exists", path.display()),
            ))
        }
        (Expect::Unchanged(expected), Some(found)) if found != expected => {
            return Err(Fault::new(
                RemoteStage::Inspect,
                RemoteFailureKind::Changed,
                format!("changed on disk since it was read (read {expected}, now {found})"),
            ))
        }
        _ => {}
    }
    let mode = existing
        .and_then(|attrs| attrs.permissions)
        .map_or(NEW_FILE_MODE, |mode| mode & 0o7777);
    Ok(Destination { found, mode })
}

fn is_link(attrs: &Attrs) -> bool {
    attrs
        .permissions
        .is_some_and(|mode| mode & S_IFMT == S_IFLNK)
}

/// Rename the staged file over `target` and return its attributes.
async fn publish<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    session: &mut Sftp<W, R>,
    temporary: &Path,
    target: &Path,
    replaces: bool,
) -> Result<Attrs, Fault> {
    if session.overwrites_on_rename() {
        return session.publish(temporary, target).await;
    }
    // SFTP v3 rename refuses an existing destination; without
    // posix-rename the replacement cannot be atomic.
    if replaces {
        session.remove(target).await?;
    }
    session.rename(temporary, target).await?;
    session.lstat(target).await
}

/// The new stamp, or the failure with the staged file removed.
async fn settle<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    session: &mut Sftp<W, R>,
    temporary: &Path,
    staged: Result<Attrs, Fault>,
) -> Result<Stamp, Fault> {
    match staged {
        Ok(attrs) => stamp(&attrs),
        Err(fault) => Err(match session.remove(temporary).await {
            Ok(()) => fault,
            Err(cleanup) => fault.with_cleanup(cleanup),
        }),
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

fn optional<T>(result: Result<T, Fault>) -> Result<Option<T>, Fault> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(fault) if fault.disposition().0 == RemoteFailureKind::NotFound => Ok(None),
        Err(fault) => Err(fault),
    }
}

pub(crate) fn stamp(attrs: &Attrs) -> Result<Stamp, Fault> {
    match (attrs.size, attrs.mtime) {
        (Some(size), Some(mtime)) => Ok(Stamp { size, mtime }),
        _ => Err(Fault::new(
            RemoteStage::Inspect,
            RemoteFailureKind::UnknownLength,
            "the server reported no size or modification time",
        )),
    }
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
