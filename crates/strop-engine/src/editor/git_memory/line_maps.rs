//! Lines of the live buffer in HEAD (line-geometry kernel RFC, #290, #291):
//! a permalink and a line history name HEAD lines, so buffer lines map
//! through the two diff edges of the buffer's hunk snapshot — the unstaged
//! regions (index → buffer), then the staged ones (HEAD → index) — with the
//! proved `strop_kernel::lines::old_span`. A range that holds an uncommitted
//! change is refused with the reason; nothing guesses a line.

use super::{HunkData, HunkSet};
use strop_core::id::{BufferRevision, DocumentId};
use strop_kernel::lines;

/// Both diff edges of one buffer revision.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LineMaps {
    /// Index → buffer.
    unstaged: HunkSet,
    /// HEAD → index.
    staged: HunkSet,
    /// No HEAD version: nothing maps.
    untracked: bool,
}

/// Two maps are the same when they hold the same published hunk sets.
impl PartialEq for LineMaps {
    fn eq(&self, other: &Self) -> bool {
        self.unstaged.identity() == other.unstaged.identity()
            && self.staged.identity() == other.staged.identity()
            && self.untracked == other.untracked
    }
}

impl Eq for LineMaps {}

impl LineMaps {
    pub(crate) fn of(data: &HunkData) -> Self {
        Self {
            unstaged: data.unstaged.clone(),
            staged: data.staged.clone(),
            untracked: data.untracked,
        }
    }

    /// The 1-based HEAD lines of 0-based buffer lines `first..=last`, or
    /// why they have none.
    pub(crate) fn head_lines(&self, first: usize, last: usize) -> Result<(usize, usize), String> {
        if self.untracked {
            return Err("file is not in HEAD".into());
        }
        let refused = || {
            if first == last {
                format!("line {} is not in HEAD (uncommitted change)", first + 1)
            } else {
                format!(
                    "lines {}–{} are not in HEAD (uncommitted changes in the selection)",
                    first + 1,
                    last + 1
                )
            }
        };
        let (Some(unstaged), Some(staged)) = (self.unstaged.line_map(), self.staged.line_map())
        else {
            return Err("this diff maps no lines".into());
        };
        let (a, b) = lines::old_span(first, last, unstaged).ok_or_else(refused)?;
        let (a, b) = lines::old_span(a, b, staged).ok_or_else(refused)?;
        Ok((a + 1, b + 1))
    }
}

/// What asked for HEAD lines while the hunk snapshot was still in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeadRequest {
    Permalink(crate::editor::permalink::PermalinkIntent),
    LineHistory,
}

/// A request held until the hunk snapshot of exactly this buffer revision
/// lands; an edit in between drops it with a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeldLines {
    pub(crate) request: HeadRequest,
    pub(crate) document: DocumentId,
    pub(crate) revision: BufferRevision,
    /// 0-based buffer lines.
    pub(crate) rows: (usize, usize),
}

/// The current buffer's HEAD lines: ready, refused, or the snapshot for
/// this revision is still in flight.
pub(crate) enum HeadLines {
    Ready((usize, usize)),
    Refused(String),
    Pending,
}
