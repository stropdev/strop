//! Project command trust uses the same private, lossless state boundary.
use super::{persistence, SessionError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct TrustedPath(#[serde(with = "strop_core::path_serde")] PathBuf);

fn path(base: Option<&Path>) -> Option<PathBuf> {
    base.map(|base| base.join("strop").join("trusted-projects"))
}

fn load(path: &Path) -> Result<Vec<TrustedPath>, SessionError> {
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
            Ok(TrustedPath(root))
        })
        .collect()
}

pub fn is_trusted(base: Option<&Path>, root: &Path) -> Result<bool, SessionError> {
    let Some(path) = path(base) else {
        return Ok(false);
    };
    Ok(load(&path)?.iter().any(|entry| entry.0 == root))
}

pub fn trust(base: Option<&Path>, root: &Path) -> Result<(), SessionError> {
    let path = path(base).ok_or_else(|| {
        SessionError::Invalid("state directory unavailable; trust was not saved".into())
    })?;
    strop_core::path_serde::validate(root)?;
    let mut entries = load(&path)?;
    if !entries.iter().any(|entry| entry.0 == root) {
        entries.push(TrustedPath(root.to_owned()));
        persistence::write(&path, &entries)?;
    }
    Ok(())
}
