//! Private metadata only: successful endpoint/directory choices, never contents,
//! credentials or permission to reconnect. All functions run on an I/O worker.
use super::{persistence, SessionError};
use std::path::Path;
use strop_remote::RemoteFile;

pub(crate) const RETAINED: usize = 32;

pub(crate) fn load(base: &Path) -> Result<Vec<RemoteFile>, SessionError> {
    let path = base.join("strop/remote-destinations.json");
    let bytes = match persistence::read(&path) {
        Ok(bytes) => bytes,
        Err(SessionError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new())
        }
        Err(error) => return Err(error),
    };
    let entries: Vec<RemoteFile> = serde_json::from_slice(&bytes)?;
    if entries.len() > RETAINED {
        return Err(SessionError::Invalid(
            "remote destination history exceeds its retention bound".into(),
        ));
    }
    Ok(entries)
}

pub(crate) fn remember(base: &Path, directory: RemoteFile) -> Result<(), SessionError> {
    let mut entries = load(base)?;
    entries.retain(|entry| entry.endpoint() != directory.endpoint());
    entries.insert(0, directory);
    entries.truncate(RETAINED);
    persistence::write(&base.join("strop/remote-destinations.json"), &entries)
}
