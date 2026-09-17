//! The recovery record contract (0056 AR04 §5): a checkpoint carries the
//! workspace/namespace, source-or-scratch identity, resource binding epoch,
//! captured revision, last source observation and the draft's actual bytes —
//! never a hash, undo pointer, native handle, write permit or command.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use strop_core::id::{BufferRevision, DocumentId};

pub const FORMAT_VERSION: u32 = 1;

/// Identity of a recoverable draft: where its bytes belong. A label or
/// identity is data for comparison and display — never a live handle or
/// write authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftOrigin {
    /// An ordinary local file: the display path and the canonical
    /// identity observed at capture.
    LocalFile {
        #[serde(with = "strop_core::path_serde")]
        path: PathBuf,
        #[serde(with = "strop_core::path_serde::option")]
        canonical: Option<PathBuf>,
    },
    /// A remote file the user explicitly consented to persist: a display
    /// label only. Restoring creates a local checked draft; reconnecting
    /// needs fresh authority through the existing remote paths.
    RemoteFile { label: String },
    /// A scratch buffer: stable per-session identity, no filesystem target.
    Scratch { id: u64 },
}

impl DraftOrigin {
    /// The original location as the user knows it.
    pub fn label(&self) -> String {
        match self {
            Self::LocalFile { path, .. } => path.display().to_string(),
            Self::RemoteFile { label } => label.clone(),
            Self::Scratch { id } => format!("[scratch #{id}]"),
        }
    }
}

/// What the source looked like when the checkpoint was taken — the basis
/// for the current/conflict/missing comparison at recovery time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceObservation {
    /// The source file's modification time at capture (unix milliseconds).
    pub modified_ms: u64,
    /// The source file's length at capture.
    pub len: u64,
}

/// Whether the record's bytes are in the checkpoint. A cohort that cannot
/// fit a draft reports that state in the record instead of silently
/// truncating the draft or the cohort (0056 AR04 §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Snapshot {
    /// The text bytes follow the header, `text_len` long.
    Captured { text_len: u64 },
    /// The draft alone exceeds what the bounded checkpoint can hold; it is
    /// not durable and the recovery surface says so.
    OverBound { bytes: u64 },
}

/// One draft's durable record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftRecord {
    pub origin: DraftOrigin,
    /// The document's content clock at capture (its applied revision then).
    pub revision: BufferRevision,
    /// Session-local document identity: live rename/newer-edit matching.
    /// Meaningless after a restart and never trusted as a resource handle.
    pub document: DocumentId,
    /// Local files only: the source state observed at capture.
    pub source: Option<SourceObservation>,
    pub snapshot: Snapshot,
}

/// The workspace/namespace and resource binding epoch the cohort was
/// captured under (0042's registry identity + incarnation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBinding {
    pub filesystem: strop_workspace::Filesystem,
    #[serde(with = "strop_core::path_serde::option")]
    pub root: Option<PathBuf>,
    /// The registry incarnation (binding epoch) at capture.
    pub incarnation: u64,
}

/// A coherent multi-document checkpoint: all of it publishes atomically or
/// none of it does, so recovery never mixes half of a newer group with
/// half of an older checkpoint (0056 AR04 §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CohortHeader {
    pub version: u32,
    pub cohort: u64,
    /// Capture time (unix milliseconds).
    pub captured_ms: u64,
    pub workspace: WorkspaceBinding,
    pub records: Vec<DraftRecord>,
}

impl CohortHeader {
    pub fn validate(&self) -> Result<(), crate::session::SessionError> {
        if self.version != FORMAT_VERSION {
            return Err(invalid(format!(
                "unsupported recovery format version {}",
                self.version
            )));
        }
        for record in &self.records {
            if let DraftOrigin::LocalFile { path, canonical } = &record.origin {
                strop_core::path_serde::validate(path)?;
                if let Some(canonical) = canonical {
                    strop_core::path_serde::validate(canonical)?;
                }
            }
        }
        Ok(())
    }
}

pub fn invalid(reason: String) -> crate::session::SessionError {
    crate::session::SessionError::Invalid(format!("recovery checkpoint: {reason}"))
}

/// Unix milliseconds, saturating at the epoch for pre-1970 clocks.
pub fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}
