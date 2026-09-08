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
    if bytes.first() == Some(&b'[') {
        return Ok(serde_json::from_slice(&bytes)?);
    }
    // Legacy line-based stores only had unambiguous UTF-8 identities.
    let text = std::str::from_utf8(&bytes).map_err(|e| SessionError::Invalid(e.to_string()))?;
    text.lines()
        .map(|line| {
            if line.contains('\u{fffd}') {
                return Err(strop_core::path_serde::PathError::AmbiguousLegacy.into());
            }
            let root = PathBuf::from(line);
            strop_core::path_serde::validate(&root)?;
            Ok(FileTarget::Local(root))
        })
        .collect()
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

pub fn is_trusted_remote(
    base: Option<&Path>,
    endpoint: &strop_remote::RemoteEndpoint,
    root: &Path,
) -> Result<bool, SessionError> {
    let Some(path) = path(base) else {
        return Ok(false);
    };
    let file = strop_remote::RemoteFile::from_path(endpoint.clone(), root.to_owned())
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    let target = FileTarget::Remote(file.into());
    Ok(load(&path)?.contains(&target))
}
pub fn trust_remote(
    base: Option<&Path>,
    root: &strop_remote::RemoteFile,
) -> Result<(), SessionError> {
    save(base, FileTarget::Remote(root.clone().into()))
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
