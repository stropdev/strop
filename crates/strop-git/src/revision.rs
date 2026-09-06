//! Revision-addressed locations (0014 wave 4): permalinks, jumps
//! into history, blame parent-hops all speak this.

use std::path::{Path, PathBuf};

use crate::repo::Repo;

/// A revisioned source location (0014 wave 4): permalinks, jumps into
/// history, and blame's parent-hop all speak this — no more "permalink
/// from a historical view links HEAD's file".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub revision: GitRevision,
    /// Repo-relative path.
    pub path: PathBuf,
    /// 1-based line range, when the location is a selection.
    pub lines: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRevision {
    /// The checked-out branch head.
    Head,
    /// A specific commit (surfaces carry this).
    Commit(String),
    /// The staged content (the index) — 0018's four-state model makes
    /// it a first-class revision, not an implicit middle.
    Index,
    /// The on-disk worktree file.
    Worktree,
    /// The merge-base of two commits (review starts here).
    MergeBase(String, String),
}

impl GitRevision {
    /// Read a file's bytes AT this revision. Live (the editor's buffer)
    /// never crosses this seam — the editor owns that copy.
    pub fn read(&self, repo: &Repo, rel: &Path) -> Option<Vec<u8>> {
        match self {
            GitRevision::Head => repo.head_bytes(rel),
            GitRevision::Commit(sha) => repo.commit_bytes(sha, rel),
            GitRevision::Index => repo.index_bytes(rel),
            GitRevision::Worktree => std::fs::read(repo.workdir.join(rel)).ok(),
            GitRevision::MergeBase(a, b) => {
                let base = repo.merge_base(a, b)?;
                repo.commit_bytes(&base, rel)
            }
        }
    }
}

impl SourceLocation {
    /// The URL slug: a pinned commit sha or the branch's name.
    pub fn revision_slug(&self) -> String {
        match &self.revision {
            GitRevision::Head => "HEAD".into(),
            GitRevision::Commit(sha) => sha.clone(),
            GitRevision::Index => "index".into(),
            GitRevision::Worktree => "worktree".into(),
            GitRevision::MergeBase(a, b) => format!("{a}...{b}"),
        }
    }
}
