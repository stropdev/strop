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
use strop_git::memory::{BlameCard, BlameLine, LogRow};
use strop_git::{FileDiff, GitContext, Hunk, RepoTarget};

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
    pub unstaged: Vec<Hunk>,
    pub staged: Vec<Hunk>,
    pub untracked: bool,
}

/// One serialized index mutation (stage/unstage). `rel` is
/// repo-relative; `kind` names the edge for messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MutationKind {
    Stage,
    Unstage,
}

impl MutationKind {
    pub fn edge(self) -> &'static str {
        match self {
            Self::Stage => "worktree → index",
            Self::Unstage => "index → HEAD",
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

/// The mutation payload: the hunk being applied to the index. Kept
/// separate from the key — the key names the view, the op is the work.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum MutationOp {
    Stage { hunk: Hunk },
    Unstage { hunk: Hunk },
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
    Files(Vec<strop_git::memory::ChangedFile>),
    Delta(FileDiff),
}

/// Results from git workers; the event loop (or headless drain)
/// routes each to its handler. Every variant owns its ticket — there
/// is no unowned error path (R9).
#[derive(serde::Serialize, serde::Deserialize)]
pub enum GitJob {
    Context(Completion<ContextKey, Option<GitContext>>),
    Hunks(Completion<HunkKey, HunkData>),
    Mutation(Completion<MutationKey, ()>),
    Log(Completion<LogKey, Vec<LogRow>>),
    Gutter(Completion<BlameKey, Vec<BlameLine>>),
    Card(Completion<CardKey, Box<BlameCard>>),
    Dive(Completion<DiveKey, DiveData>),
}
