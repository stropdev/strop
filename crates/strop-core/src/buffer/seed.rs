//! Deterministic buffer seeds (R11 forensic replay): whole text, revision,
//! undo history and the disk baseline — a replayed buffer keeps its exact
//! live identity without reading the filesystem. `disk_stamp` is seed DATA
//! for overwrite protection, never a request to stat anything.
use serde::{Deserialize, Serialize};

use super::Buffer;
use crate::history::History;
use crate::id::BufferRevision;

/// A pure, serializable image of a buffer at the startup seed boundary.
/// The buffer's diagnostic trace identity is deliberately absent: it names
/// a process-local incarnation, not document semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferSeed {
    pub text: String,
    pub revision: BufferRevision,
    pub history: History,
    #[serde(with = "crate::path_serde::option")]
    pub path: Option<std::path::PathBuf>,
    pub name: Option<String>,
    pub dirty: bool,
    pub readonly: bool,
    pub disk_stamp: Option<std::time::SystemTime>,
    #[serde(with = "crate::path_serde::option")]
    pub file_identity: Option<std::path::PathBuf>,
}

impl Buffer {
    pub fn seed(&self) -> BufferSeed {
        BufferSeed {
            text: self.rope.to_string(),
            revision: self.revision(),
            history: self.history.clone(),
            path: self.path.clone(),
            name: self.name.clone(),
            dirty: self.dirty,
            readonly: self.readonly,
            disk_stamp: self.disk_stamp,
            file_identity: self.file_identity.clone(),
        }
    }
}

impl BufferSeed {
    /// Rebuild the buffer. Restored history is validated against the
    /// seeded text; a seed that disagrees is rejected, not coerced.
    pub fn into_buffer(self) -> Result<Buffer, crate::history::HistoryError> {
        let mut buffer = Buffer::from_text(&self.text);
        buffer.restore_history(self.history)?;
        buffer.epoch = self.revision.get();
        buffer.path = self.path;
        buffer.name = self.name;
        buffer.dirty = self.dirty;
        buffer.readonly = self.readonly;
        buffer.disk_stamp = self.disk_stamp;
        buffer.file_identity = self.file_identity;
        Ok(buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_round_trips_content_history_and_disk_baseline() {
        let mut buffer = Buffer::from_text("line\nline two\n");
        buffer.begin_undo_group();
        buffer
            .edit()
            .replace(crate::Range::charwise(5, 8), "XY")
            .unwrap();
        buffer.commit_undo_group();
        buffer.dirty = true;
        buffer.disk_stamp = Some(std::time::UNIX_EPOCH);
        let seed = buffer.seed();
        let mut restored = seed.into_buffer().unwrap();
        assert_eq!(restored.text().to_string(), "line\nXYe two\n");
        // The undo that survived the seed still runs on the rebuilt buffer.
        restored.undo().unwrap();
        assert_eq!(restored.text().to_string(), "line\nline two\n");
    }

    #[test]
    fn history_that_disagrees_with_text_is_rejected() {
        let mut buffer = Buffer::from_text("abc\n");
        buffer.begin_undo_group();
        buffer
            .edit()
            .replace(crate::Range::charwise(0, 1), "z")
            .unwrap();
        buffer.commit_undo_group();
        let mut seed = buffer.seed();
        seed.text = "different bytes\n".into();
        assert!(seed.into_buffer().is_err());
    }
}
