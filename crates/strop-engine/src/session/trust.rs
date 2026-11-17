//! Project command trust uses the same private, lossless state boundary.
use super::{persistence, SessionError};
use crate::files::FileTarget;
use std::path::{Path, PathBuf};

fn path(base: Option<&Path>) -> Option<PathBuf> {
    base.map(|base| base.join("strop").join("trusted-projects"))
}

fn load(path: &Path) -> Result<Vec<FileTarget>, SessionError> {
    let bytes = match persistence::read(path) {
        Ok(bytes) => bytes,
        Err(SessionError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new())
        }
        Err(error) => return Err(error),
    };
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn is_trusted(base: Option<&Path>, root: &Path) -> Result<bool, SessionError> {
    let Some(path) = path(base) else {
        return Ok(false);
    };
    Ok(load(&path)?
        .iter()
        .any(|entry| matches!(entry, FileTarget::Local(path) if path == root)))
}

pub fn trust(base: Option<&Path>, root: &Path) -> Result<(), SessionError> {
    strop_core::path_serde::validate(root)?;
    save(base, FileTarget::Local(root.to_owned()))
}

fn save(base: Option<&Path>, target: FileTarget) -> Result<(), SessionError> {
    let path = path(base).ok_or_else(|| {
        SessionError::Invalid("state directory unavailable; trust was not saved".into())
    })?;
    let mut entries = load(&path)?;
    if !entries.contains(&target) {
        entries.push(target);
        persistence::write(&path, &entries)?;
    }
    Ok(())
}
