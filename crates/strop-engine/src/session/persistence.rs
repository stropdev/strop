//! Private exclusive staging; updates replace the directory entry atomically.
use super::{io, SessionError};
use serde::Serialize;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

/// This is retention, not redaction. Oversize snapshots fail visibly.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn read(path: &Path) -> Result<Vec<u8>, SessionError> {
    let file = File::open(path).map_err(|e| io(path, e))?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io(path, e))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(SessionError::Invalid(
            "state exceeds the 16 MiB capture limit".into(),
        ));
    }
    Ok(bytes)
}

pub(super) fn write(path: &Path, value: &impl Serialize) -> Result<(), SessionError> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(SessionError::Invalid(
            "state exceeds the 16 MiB capture limit".into(),
        ));
    }
    publish(path, |file| {
        file.write_all(&bytes).and_then(|()| file.sync_all())
    })
}

/// The store callback is the fault-injection seam; privacy and publication
/// ordering are identical for successful writes and real write/sync failures.
pub(super) fn publish(
    path: &Path,
    store: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> Result<(), SessionError> {
    let parent = path
        .parent()
        .ok_or_else(|| SessionError::Invalid("state path has no parent".into()))?;
    let mut directory = fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(parent).map_err(|e| io(parent, e))?;
    // NamedTempFile creates with 0600 before any sensitive bytes are written.
    let mut staged = tempfile::Builder::new()
        .prefix(".strop-state-")
        .tempfile_in(parent)
        .map_err(|e| io(parent, e))?;
    if let Err(error) = store(staged.as_file_mut()) {
        let original = io(path, error);
        return match staged.close() {
            Ok(()) => Err(original),
            Err(error) => Err(SessionError::Cleanup {
                original: Box::new(original),
                cleanup: Box::new(io(parent, error)),
            }),
        };
    }
    if let Err(error) = staged.persist(path) {
        let original = io(path, error.error);
        return match error.file.close() {
            Ok(()) => Err(original),
            Err(error) => Err(SessionError::Cleanup {
                original: Box::new(original),
                cleanup: Box::new(io(parent, error)),
            }),
        };
    }
    #[cfg(unix)]
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|e| io(parent, e))?;
    Ok(())
}
