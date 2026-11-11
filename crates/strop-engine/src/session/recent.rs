//! A project's frecent files, stored beside its session under the same
//! project key and the same private, atomic publication (0093 §3.4).
//! Names and counts only — never contents. A separate file because its
//! lifetime differs: a session is replaced by each capture, this record
//! accumulates, and reading it at startup must not parse undo history.
use super::{persistence, SessionError};
use crate::editor::RecentFiles;
use std::path::{Path, PathBuf};

pub(super) fn path(base_dir: Option<&Path>, cwd: &Path) -> Option<PathBuf> {
    Some(
        base_dir?
            .join("strop")
            .join("recent")
            .join(format!("{}.json", super::project_key(cwd))),
    )
}

/// Missing state is an empty record, not an error.
pub fn load(base_dir: Option<&Path>, cwd: &Path) -> Result<RecentFiles, SessionError> {
    let Some(path) = path(base_dir, cwd) else {
        return Ok(RecentFiles::default());
    };
    let bytes = match persistence::read(&path) {
        Ok(bytes) => bytes,
        Err(SessionError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecentFiles::default())
        }
        Err(error) => return Err(error),
    };
    let recent: RecentFiles = serde_json::from_slice(&bytes)?;
    recent.validate().map_err(SessionError::Invalid)?;
    Ok(recent)
}

pub(super) fn store(path: &Path, recent: &RecentFiles) -> Result<(), SessionError> {
    persistence::write(path, recent)
}
