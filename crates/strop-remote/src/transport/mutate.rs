//! File management on a host over the endpoint's SFTP session: create a
//! file or directory, rename, copy a regular file, remove a file or an
//! empty directory. None of these ever replaces an existing name: a file
//! is created with `O_EXCL`, a directory with MKDIR (which refuses an
//! existing name), a rename with SFTP v3 RENAME (OpenSSH refuses an
//! existing destination) and a copy is an exclusive create. There is no
//! Trash on a host: removal is permanent. Each call is one operation;
//! nothing spans calls atomically.
use super::error::{Fault, RemoteFailureKind};
use super::save::stamp;
use super::wire::{Attrs, Sftp};
use crate::RemoteEntryKind;
use std::path::Path;
use tokio::io::{AsyncRead, AsyncWrite};

/// Permission bits for a created file or directory; the server's umask
/// still applies.
const NEW_FILE_MODE: u32 = 0o644;
const NEW_DIRECTORY_MODE: u32 = 0o755;

/// What an LSTAT of one name observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteAttrs {
    pub kind: RemoteEntryKind,
    pub size: Option<u64>,
    /// Seconds since the epoch.
    pub mtime: Option<u32>,
    /// The full mode word, kind bits included.
    pub permissions: Option<u32>,
}

impl RemoteAttrs {
    fn from_attrs(attrs: &Attrs) -> Self {
        Self {
            kind: super::session::kind_of(attrs.permissions),
            size: attrs.size,
            mtime: attrs.mtime,
            permissions: attrs.permissions,
        }
    }
}

/// One file-management request. Every variant leaves an existing name
/// untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    /// A new regular file holding `bytes`.
    CreateFile {
        path: std::path::PathBuf,
        bytes: Vec<u8>,
    },
    CreateDirectory {
        path: std::path::PathBuf,
    },
    Rename {
        from: std::path::PathBuf,
        to: std::path::PathBuf,
    },
    /// A new regular file with `from`'s bytes and permission bits.
    Copy {
        from: std::path::PathBuf,
        to: std::path::PathBuf,
        max: u64,
    },
    /// A file, a symlink (not its target) or an empty directory.
    Remove {
        path: std::path::PathBuf,
        directory: bool,
    },
}

/// The LSTAT of `path`, `None` when nothing is there.
pub(crate) async fn observe<W, R>(
    session: &mut Sftp<W, R>,
    path: &Path,
) -> Result<Option<RemoteAttrs>, Fault>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    match session.lstat(path).await {
        Ok(attrs) => Ok(Some(RemoteAttrs::from_attrs(&attrs))),
        Err(fault) if fault.disposition().0 == RemoteFailureKind::NotFound => Ok(None),
        Err(fault) => Err(fault),
    }
}

/// Run one mutation; the result is what the touched name holds after it
/// (`None` after a removal).
pub(crate) async fn mutate<W, R>(
    session: &mut Sftp<W, R>,
    mutation: &Mutation,
) -> Result<Option<RemoteAttrs>, Fault>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    match mutation {
        Mutation::CreateFile { path, bytes } => {
            create_exclusive(session, path, bytes, NEW_FILE_MODE).await?;
            observe(session, path).await
        }
        Mutation::CreateDirectory { path } => {
            session.mkdir(path, NEW_DIRECTORY_MODE).await?;
            observe(session, path).await
        }
        Mutation::Rename { from, to } => {
            session.rename(from, to).await?;
            observe(session, to).await
        }
        Mutation::Copy { from, to, max } => {
            let (bytes, attrs) = session.read_whole(from, *max).await?;
            stamp(&attrs)?;
            let mode = attrs
                .permissions
                .map_or(NEW_FILE_MODE, |mode| mode & 0o7777);
            create_exclusive(session, to, &bytes, mode).await?;
            observe(session, to).await
        }
        Mutation::Remove { path, directory } => {
            if *directory {
                session.rmdir(path).await?;
            } else {
                session.remove(path).await?;
            }
            Ok(None)
        }
    }
}

/// Create `path` exclusively and fill it; a failed fill removes the
/// partial file it created.
async fn create_exclusive<W, R>(
    session: &mut Sftp<W, R>,
    path: &Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), Fault>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    let handle = session.open_exclusive(path).await?;
    match session.upload_into(handle, path, bytes, mode).await {
        Ok(()) => Ok(()),
        Err(fault) => Err(match session.remove(path).await {
            Ok(()) => fault,
            Err(cleanup) => fault.with_cleanup(cleanup),
        }),
    }
}
