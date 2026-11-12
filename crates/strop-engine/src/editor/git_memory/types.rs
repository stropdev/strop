//! Request ownership and terminal results for native Git operations.
//!
//! Every key carries its [`RepoTarget`] (0036 RW8): the repository a
//! request runs against is the one it was launched with — local or
//! remote — and every completion validates against that target, so a
//! cached context for one machine can never satisfy a request owned by
//! another.
use std::path::PathBuf;

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{Completion, WorkerId};
use strop_git::log::LogRow;
use strop_git::memory::{BlameCard, BlameLine, CommitSummary};
use strop_git::{
    Branch, FileDelta, GitContext, Hunk, LineStat, RepoTarget, StatusEntry, WorkingTree,
};

use crate::files::FileTarget;

/// What a discovery request resolves from — the buffer's file target
/// (local path or remote file) or, for local scratch buffers, the cwd.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContextKey {
    pub from: FileTarget,
}

/// The identity of one gutter-diff request: the document, its text
/// revision, the file (local path or remote file identity), the
/// repository target it runs against, and the git view (the index
/// state the diff was computed against). Any change to any of these
/// makes a completion stale.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HunkKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    /// The buffer's file identity — a remote buffer names its remote
    /// file, never a local path spelling of one.
    pub file: FileTarget,
    pub repo: RepoTarget,
    pub git_view: WorkerId,
}

/// One hunk snapshot: both cached vectors plus whether the diff base
/// was absent entirely (the buffer is untracked — restore has nothing
/// to go back to).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HunkData {
    pub unstaged: super::HunkSet,
    pub staged: super::HunkSet,
    pub untracked: bool,
}

/// One serialized mutation's edge. `rel` is repo-relative; `kind`
/// names the edge for messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MutationKind {
    Stage,
    Unstage,
    Discard,
}

impl MutationKind {
    pub fn edge(self) -> &'static str {
        match self {
            Self::Stage => "worktree → index",
            Self::Unstage => "index → HEAD",
            Self::Discard => "index → worktree",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MutationKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub kind: MutationKind,
    #[serde(with = "strop_core::path_serde")]
    pub rel: PathBuf,
    pub repo: RepoTarget,
    pub git_view: WorkerId,
}

/// The mutation payload: one hunk (the in-file verbs and an expanded
/// status row) or whole status entries (a file row, a section
/// heading). Kept separate from the key — the key names the view, the
/// op is the work.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum MutationOp {
    Stage {
        hunk: std::sync::Arc<Hunk>,
    },
    Unstage {
        hunk: std::sync::Arc<Hunk>,
    },
    /// Revert one unstaged hunk in the working file.
    DiscardHunk {
        hunk: std::sync::Arc<Hunk>,
    },
    StageFiles {
        entries: Vec<StatusEntry>,
    },
    UnstageFiles {
        entries: Vec<StatusEntry>,
    },
    /// Restore tracked files from the index; delete untracked paths.
    DiscardFiles {
        entries: Vec<StatusEntry>,
    },
}

impl MutationOp {
    pub fn kind(&self) -> MutationKind {
        match self {
            Self::Stage { .. } | Self::StageFiles { .. } => MutationKind::Stage,
            Self::Unstage { .. } | Self::UnstageFiles { .. } => MutationKind::Unstage,
            Self::DiscardHunk { .. } | Self::DiscardFiles { .. } => MutationKind::Discard,
        }
    }

    /// Run the mutation against the repository. Entries apply in
    /// order; the first failure stops the batch and is reported.
    pub fn apply(&self, repo: &strop_git::Repo, rel: &std::path::Path) -> Result<(), String> {
        let each =
            |entries: &[StatusEntry],
             run: &dyn Fn(&StatusEntry) -> Result<(), strop_git::GitError>| {
                entries
                    .iter()
                    .try_for_each(run)
                    .map_err(|error| error.to_string())
            };
        match self {
            Self::Stage { hunk } => repo.stage_hunk(rel, hunk),
            Self::Unstage { hunk } => repo.unstage_hunk(rel, hunk),
            Self::DiscardHunk { hunk } => repo
                .discard_hunk(rel, hunk)
                .map_err(|error| error.to_string()),
            Self::StageFiles { entries } => each(entries, &|entry| repo.stage_path(&entry.rel)),
            Self::UnstageFiles { entries } => each(entries, &|entry| repo.unstage_path(&entry.rel)),
            Self::DiscardFiles { entries } => each(entries, &|entry| repo.discard_path(entry)),
        }
    }

    /// The echo-row line once the mutation landed: `hunk staged`,
    /// `staged src/lib.rs`, `discarded 3 files`.
    pub fn done(&self) -> String {
        let verb = match self.kind() {
            MutationKind::Stage => "staged",
            MutationKind::Unstage => "unstaged",
            MutationKind::Discard => "discarded",
        };
        match self {
            Self::Stage { .. } | Self::Unstage { .. } | Self::DiscardHunk { .. } => {
                format!("hunk {verb}")
            }
            Self::StageFiles { entries }
            | Self::UnstageFiles { entries }
            | Self::DiscardFiles { entries } => match entries.as_slice() {
                [entry] => format!("{verb} {}", entry.rel.to_string_lossy()),
                entries => format!("{verb} {} files", entries.len()),
            },
        }
    }
}

/// A queued mutation waiting for the running one to settle — index
/// writes are strictly FIFO, never concurrent.
#[derive(Debug, Clone)]
pub struct GitMutation {
    pub key: MutationKey,
    pub op: MutationOp,
}

/// One log surface's request: the surface document, the revision its
/// buffer was created at, and the repository the log runs against.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub repo: RepoTarget,
}

/// A log request in flight: its ticket, and the revision the surface is
/// at — the one the next page must find it at. Each page the request
/// publishes moves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRequest {
    pub ticket: strop_core::worker::Ticket<LogKey>,
    pub expected: BufferRevision,
    /// A page has replaced the `loading log…` text.
    pub published: bool,
}

/// The end of a history read: the rows after the last page, and how
/// long git took — a slow read on a local repository asks for a
/// private commit-graph (0101 §2.1).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogDone {
    pub rows: Vec<LogRow>,
    pub took: std::time::Duration,
}

/// A private commit-graph build: the repository and the `HEAD` it was
/// started for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GraphKey {
    pub repo: RepoTarget,
    pub head: Option<String>,
}

/// What a build found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GraphBuilt {
    /// Both steps installed.
    Built,
    /// The repository keeps a commit-graph of its own: git's to refresh.
    Gits,
}

/// Frozen document, file and repository identity for a blame request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlameKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub file: FileTarget,
    pub repo: RepoTarget,
}

/// The single-line card: a blame origin plus the 1-based line.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CardKey {
    pub origin: BlameKey,
    pub line: usize,
}

/// What a dive is fetching: a commit's changed-file list, or one
/// file's delta at a commit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DiveTarget {
    HunkPreview {
        origin: super::HunkOrigin,
        index: usize,
    },
    CommitFiles {
        sha: String,
    },
    FileDelta {
        sha: String,
        #[serde(with = "strop_core::path_serde")]
        path: PathBuf,
    },
}

/// A dive request: the surface document asking, the repository the
/// fetch runs against, and what it asked for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiveKey {
    pub document: DocumentId,
    pub repo: RepoTarget,
    pub target: DiveTarget,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum DiveData {
    /// A commit's file list, with what the commit says about itself.
    Files(super::PreparedFiles, Box<CommitSummary>),
    Delta(super::PreparedDiff),
}

/// One status buffer's snapshot request (the ticket's request id
/// tells a refresh apart from the one before it).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatusKey {
    pub document: DocumentId,
    pub repo: RepoTarget,
}

/// A status snapshot: the tree and the diffs of its expanded files.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StatusData {
    pub tree: WorkingTree,
    pub deltas: Vec<(StatusEntry, FileDelta)>,
    /// `stats[n]` is the line count of `tree.entries[n]`: none for a
    /// conflict or an untracked directory (0100 §7).
    pub stats: Vec<Option<LineStat>>,
}

/// A commit-message template being written for `repo`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DraftKey {
    pub repo: RepoTarget,
    pub amend: bool,
}

/// The written template's path (`<gitdir>/COMMIT_EDITMSG`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommitDraft {
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
}

/// What one `git` CLI run does (0096 §2.2: hooks, signing and the
/// working-tree checks are git's own).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Porcelain {
    Commit {
        amend: bool,
        #[serde(with = "strop_core::path_serde")]
        message: PathBuf,
    },
    Switch {
        branch: String,
    },
    Create {
        branch: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PorcelainKey {
    pub repo: RepoTarget,
    pub command: Porcelain,
}

/// A finished CLI run: the line git printed and the repository's
/// context afterwards (a commit and a switch both move HEAD).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PorcelainDone {
    pub line: String,
    pub context: Option<GitContext>,
}

/// The branch list for the picker opened from `document`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BranchKey {
    pub document: DocumentId,
    pub repo: RepoTarget,
}

/// Results from git workers; the event loop (or headless drain)
/// routes each to its handler. Every variant owns its ticket — there
/// is no unowned error path (R9).
#[derive(serde::Serialize, serde::Deserialize)]
pub enum GitJob {
    Context(Completion<ContextKey, Option<GitContext>>),
    Hunks(Completion<HunkKey, HunkData>),
    Mutation(Completion<MutationKey, ()>),
    /// A page of a history read still running; rows are appended.
    LogPage(strop_core::worker::Ticket<LogKey>, Vec<LogRow>),
    Log(Completion<LogKey, LogDone>),
    Graph(Completion<GraphKey, GraphBuilt>),
    Gutter(Completion<BlameKey, Vec<BlameLine>>),
    Card(Completion<CardKey, Box<BlameCard>>),
    Dive(Completion<DiveKey, DiveData>),
    Status(Completion<StatusKey, StatusData>),
    CommitDraft(Completion<DraftKey, CommitDraft>),
    Porcelain(Completion<PorcelainKey, PorcelainDone>),
    Branches(Completion<BranchKey, Vec<Branch>>),
}
