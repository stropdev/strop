//! The document's source identity (0021 §4): what a document IS drives
//! readonly-ness and save behavior — not conventions on three fields.
//! Git-memory surfaces carry their typed payload here; the editor's
//! surface bookkeeping reads one place.

use super::JumpRecord;
use crate::editor::git_memory::{CommitFiles, HunkOrigin};
use strop_git::memory::LogRow;
use strop_git::Hunk;

/// What backs a document. Derived facts (readonly, save refusal) come
/// from this, not from fields callers must keep in sync.
#[derive(Debug, Clone)]
pub enum DocumentSource {
    /// A file on disk (`Buffer.path` is Some).
    File,
    /// The [No Name] scratch buffer.
    Scratch,
    /// An in-memory SSH document; write authority is explicit and never local.
    Remote(Box<super::RemoteDocument>),
    /// A remote directory's real, read-only listing buffer.
    RemoteDirectory(Box<super::RemoteDirectory>),
    /// A file inside a container: readonly (DC1b refuses writes); the
    /// path names the container's filesystem — never a local path.
    Container {
        container: strop_workspace::ContainerId,
        path: std::path::PathBuf,
    },
    /// A git-memory surface: job-owned content, readonly.
    Surface(Box<GitSurface>),
    /// `:!cmd` output / help: named virtual content, readonly. The
    /// return point is where `:q` hands the view back (0051 §7 R07) —
    /// temporary surfaces restore their origin, not the MRU's line 1.
    Output { return_to: Option<JumpRecord> },
}

#[derive(Debug, Clone)]
pub struct GitSurface {
    pub context: strop_git::GitContext,
    pub content: Surface,
}

#[derive(Debug, Clone)]
pub enum Surface {
    CommitLog {
        rows: Vec<LogRow>,
        /// Sha to land the cursor on once rows arrive (the blame dive
        /// opens the browser *at* a commit, 0011 §3).
        focus: Option<String>,
        return_to: Option<JumpRecord>,
    },
    ChangedFiles {
        sha: String,
        files: crate::editor::git_memory::PreparedFiles,
        return_to: Option<JumpRecord>,
    },
    /// A diff as a readonly buffer (0010 §2): the file's delta at a
    /// commit, or the `Space g p` hunk preview. The buffer's rows mirror
    /// the rendered layout — a stats row, then per hunk a `@@` header
    /// row and unprefixed content rows — so motions, `/` and yank see
    /// exactly what's on screen. `origin` names the working buffer a
    /// hunk preview belongs to, so `Space g u`/`g s` act on the file;
    /// `commit` carries the commit's other files when this delta came
    /// from the dive chain (the sidebar + `]f`/`[f`, 0011 §4).
    Diff {
        /// Worker-prepared immutable content and its display/source projection.
        hunks: crate::editor::git_memory::PreparedDiff,
        origin: Option<HunkOrigin>,
        commit: Option<CommitFiles>,
        /// tuicr-style: Tab moves focus between the file sidebar and
        /// the diff content (j/k step files when the sidebar has focus).
        sidebar_focus: bool,
        return_to: Option<JumpRecord>,
    },
}

/// One canonical row projection shared by rendering and source navigation.
pub enum DiffRow<'a> {
    Stats,
    HunkHeader(&'a Hunk),
    Line(&'a strop_git::DiffLine),
}

impl Surface {
    pub fn diff_row(&self, row: usize) -> Option<DiffRow<'_>> {
        let Surface::Diff { hunks, .. } = self else {
            return None;
        };
        hunks.row(row)
    }

    pub(crate) fn set_return_point(&mut self, ret: JumpRecord) {
        *self.return_slot() = Some(ret);
    }

    pub(crate) fn return_point(&self) -> Option<&JumpRecord> {
        match self {
            Surface::CommitLog { return_to, .. }
            | Surface::ChangedFiles { return_to, .. }
            | Surface::Diff { return_to, .. } => return_to.as_ref(),
        }
    }

    pub(crate) fn return_slot(&mut self) -> &mut Option<JumpRecord> {
        match self {
            Surface::CommitLog { return_to, .. }
            | Surface::ChangedFiles { return_to, .. }
            | Surface::Diff { return_to, .. } => return_to,
        }
    }
}
