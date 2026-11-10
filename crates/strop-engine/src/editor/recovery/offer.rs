//! Drafts an earlier session left behind, and the offer to recover one
//! when its file opens (vim's swap-file `ATTENTION` prompt is the
//! baseline, 0090 §3): recover / open read-only / delete draft / edit
//! anyway. The prior cohort is read once at startup and carried into every
//! publication of this session until the user recovers or deletes each
//! draft, so a new session's first checkpoint never destroys a crashed
//! session's bytes.

use std::path::Path;

use super::record::{DraftOrigin, DraftRecord, Snapshot};
use super::StoredCohort;
use strop_core::id::DocumentId;

/// One draft inherited from an earlier session.
pub(crate) struct Inherited {
    pub(crate) record: DraftRecord,
    /// The draft text; `None` when the checkpoint could not hold it.
    pub(crate) text: Option<ropey::Rope>,
    /// When the earlier session captured it (unix milliseconds).
    pub(crate) captured_ms: u64,
    /// Kept by the user's choice (edit anyway / read-only): not offered
    /// again this session; `:recover` still lists it.
    pub(crate) declined: bool,
}

impl Inherited {
    /// Every record of a loaded cohort, text adopted once (never on the
    /// input path again: publications clone the rope).
    pub(crate) fn adopt(stored: StoredCohort) -> Vec<Self> {
        let captured_ms = stored.header.captured_ms;
        stored
            .header
            .records
            .into_iter()
            .zip(stored.texts)
            .map(|(record, text)| Self {
                text: text
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .map(|text| ropey::Rope::from_str(&text)),
                record,
                captured_ms,
                declined: false,
            })
            .collect()
    }

    /// Whether this local-file draft belongs to the file at `path`
    /// (display path or canonical identity).
    pub(crate) fn is_for(&self, path: &Path, canonical: Option<&Path>) -> bool {
        let DraftOrigin::LocalFile {
            path: origin,
            canonical: identity,
        } = &self.record.origin
        else {
            return false;
        };
        origin == path || (identity.is_some() && identity.as_deref() == canonical)
    }

    pub(crate) fn same_draft(&self, record: &DraftRecord) -> bool {
        self.record.origin == record.origin && self.record.revision == record.revision
    }
}

/// The modal offer: which document, which inherited draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Offer {
    pub(crate) document: DocumentId,
    pub(crate) draft: usize,
    /// The opened file differs from what the draft's session observed.
    pub(crate) source_changed: bool,
}

/// The choices, keyed as in vim's prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Choice {
    Recover,
    ReadOnly,
    Delete,
    EditAnyway,
}

impl Choice {
    pub(crate) fn from_key(key: strop_core::frontend_input::Key) -> Option<Self> {
        use strop_core::frontend_input::Key;
        match key {
            Key::Char('r' | 'R') => Some(Self::Recover),
            Key::Char('o' | 'O') => Some(Self::ReadOnly),
            Key::Char('d' | 'D') => Some(Self::Delete),
            // Esc keeps the draft: the safe answer to an unread question.
            Key::Char('e' | 'E') | Key::Esc => Some(Self::EditAnyway),
            _ => None,
        }
    }
}

/// What the frontend paints on the offer card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryOffer {
    pub path: String,
    pub revision: u64,
    pub bytes: u64,
    pub captured_ms: u64,
    /// The file changed on disk after the draft was checkpointed.
    pub source_changed: bool,
}

pub(crate) fn captured_len(record: &DraftRecord) -> u64 {
    match record.snapshot {
        Snapshot::Captured { text_len } => text_len,
        Snapshot::OverBound { bytes } => bytes,
    }
}

/// A test-support draft of `text` for `path`, captured with no source
/// observation (so the offer flags the file as changed since).
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn fixture_inherited(path: &Path, text: &str, document: DocumentId) -> Inherited {
    Inherited {
        record: DraftRecord {
            origin: DraftOrigin::LocalFile {
                path: path.to_owned(),
                canonical: None,
            },
            revision: strop_core::id::BufferRevision::new(7),
            document,
            source: None,
            snapshot: Snapshot::Captured {
                text_len: text.len() as u64,
            },
        },
        text: Some(ropey::Rope::from_str(text)),
        captured_ms: 0,
        declined: false,
    }
}
