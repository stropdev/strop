//! Owned file writes: preparing and accepting are pure; execute belongs on a worker.
use super::Buffer;
use crate::id::BufferRevision;
use ropey::Rope;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct SaveRequest {
    text: Rope,
    origin: Option<PathBuf>,
    target: PathBuf,
    revision: BufferRevision,
    baseline: Option<SystemTime>,
    new_name: bool,
    force: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SaveReceipt {
    #[serde(with = "crate::path_serde::option")]
    origin: Option<PathBuf>,
    #[serde(with = "crate::path_serde")]
    target: PathBuf,
    #[serde(with = "crate::path_serde")]
    canonical: PathBuf,
    revision: BufferRevision,
    stamp: Option<SystemTime>,
}

impl Buffer {
    pub fn prepare_save(&self, target: Option<PathBuf>, force: bool) -> io::Result<SaveRequest> {
        if self.readonly && !force {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "readonly buffer — :w! to force",
            ));
        }
        let new_name = target.is_some();
        let target = target.or_else(|| self.path.clone()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no file name — :w {path} to name it",
            )
        })?;
        Ok(SaveRequest {
            text: self.snapshot(),
            origin: self.path.clone(),
            target,
            revision: self.revision(),
            baseline: self.disk_stamp,
            new_name,
            force,
        })
    }

    /// A write of an older snapshot may update the on-disk baseline, never clear
    /// dirty text or rename an edited document. Caller also checks request identity.
    pub fn accept_save(&mut self, receipt: SaveReceipt) -> bool {
        if self.path != receipt.origin {
            return false;
        }
        let current = self.revision() == receipt.revision;
        if current {
            self.path = Some(receipt.target);
            self.disk_stamp = receipt.stamp;
            self.file_identity = Some(receipt.canonical);
            self.dirty = false;
        } else if self.path.as_ref() == Some(&receipt.target) {
            self.disk_stamp = receipt.stamp;
        }
        current
    }
}

impl SaveRequest {
    /// Blocking filesystem work; no editor borrow crosses this boundary.
    pub fn execute(self) -> io::Result<SaveReceipt> {
        let target = if self.new_name {
            self.target.clone()
        } else {
            fs::canonicalize(&self.target).unwrap_or_else(|_| self.target.clone())
        };
        let current = match fs::metadata(&target) {
            Ok(metadata) => Some(metadata.modified()?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if !self.force && self.new_name && current.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file exists — :w! to overwrite",
            ));
        }
        if !self.force && !self.new_name && current != self.baseline {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file changed on disk — :w! to force",
            ));
        }
        write_atomic(&target, &self.text, !self.new_name || self.force)?;
        let stamp = Some(fs::metadata(&target)?.modified()?);
        let canonical = fs::canonicalize(&target)?;
        Ok(SaveReceipt {
            origin: self.origin,
            target: self.target,
            canonical,
            revision: self.revision,
            stamp,
        })
    }
}

fn write_atomic(target: &Path, contents: &Rope, overwrite: bool) -> io::Result<()> {
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    match fs::metadata(target) {
        Ok(metadata) => temporary
            .as_file()
            .set_permissions(metadata.permissions())?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    for chunk in contents.chunks() {
        temporary.write_all(chunk.as_bytes())?;
    }
    temporary.as_file().sync_all()?;
    let result = if overwrite {
        temporary.persist(target)
    } else {
        temporary.persist_noclobber(target)
    };
    result.map_err(|error| error.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
