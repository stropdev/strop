//! The working tree as a whole (0096): what is staged, unstaged,
//! untracked or conflicted; file-level index and worktree mutations; the
//! branch list; the commit-message template. Everything here is libgit2
//! in-process — the commit and the branch switch themselves run the
//! `git` CLI (hooks, signing, config), see [`crate::exec`].

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::diff::{DiffLine, Hunk, LineOrigin};
use crate::repo::{hunks_from_buffers, split_lines_bytes, GitError, Repo};

/// Where a changed path sits, in the order the status buffer lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Section {
    Conflicts,
    Staged,
    Unstaged,
    Untracked,
}

impl Section {
    pub const ALL: [Section; 4] = [
        Section::Conflicts,
        Section::Staged,
        Section::Unstaged,
        Section::Untracked,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Conflicts => "Conflicts",
            Section::Staged => "Staged",
            Section::Unstaged => "Unstaged",
            Section::Untracked => "Untracked",
        }
    }
}

/// What happened to the path on its section's edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Change {
    Added,
    Modified,
    Deleted,
    TypeChanged,
    Conflicted,
    Untracked,
}

impl Change {
    /// The one-letter column git and fugitive print.
    pub fn letter(self) -> char {
        match self {
            Change::Added => 'A',
            Change::Modified => 'M',
            Change::Deleted => 'D',
            Change::TypeChanged => 'T',
            Change::Conflicted => 'U',
            Change::Untracked => '?',
        }
    }
}

/// One row of the status list. A path changed on both edges appears
/// twice, once per section.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct StatusEntry {
    pub section: Section,
    pub change: Change,
    /// Repo-relative native path; an untracked directory keeps git's
    /// trailing `/`.
    #[serde(with = "strop_core::path_serde")]
    pub rel: PathBuf,
}

impl StatusEntry {
    pub fn is_directory(&self) -> bool {
        self.rel.as_os_str().as_bytes().ends_with(b"/")
    }
}

/// The current branch's upstream and how far the two have diverged.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tracking {
    pub upstream: String,
    pub ahead: usize,
    pub behind: usize,
}

/// One status snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkingTree {
    /// Branch short name; a detached HEAD gives the abbreviated sha and
    /// an unborn one the branch it will create.
    pub head: Option<String>,
    pub tracking: Option<Tracking>,
    pub entries: Vec<StatusEntry>,
}

impl WorkingTree {
    pub fn section(&self, section: Section) -> impl Iterator<Item = &StatusEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.section == section)
    }
}

/// One file's change on one edge.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FileDelta {
    Hunks(Vec<Hunk>),
    /// Content with NUL bytes: no line diff to show.
    Binary,
}

/// The lines one change adds and removes: the fact on a status row
/// (0100 §7). A binary change has no line counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LineStat {
    pub added: usize,
    pub deleted: usize,
    pub binary: bool,
}

impl FileDelta {
    /// What the hunks add up to — the same lines the expanded entry shows.
    pub fn stat(&self) -> LineStat {
        let FileDelta::Hunks(hunks) = self else {
            return LineStat {
                binary: true,
                ..LineStat::default()
            };
        };
        let mut stat = LineStat::default();
        for line in hunks.iter().flat_map(|hunk| &hunk.lines) {
            match line.origin {
                LineOrigin::Addition => stat.added += 1,
                LineOrigin::Deletion => stat.deleted += 1,
                LineOrigin::Context => {}
            }
        }
        stat
    }
}

/// One row of the branch list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Branch {
    /// `main`, or `origin/main` for a remote-tracking branch.
    pub name: String,
    pub remote: bool,
    pub current: bool,
    /// What `git switch` is asked for: the name itself, or a remote
    /// branch's name without its remote, so git creates or reuses the
    /// local branch that tracks it.
    pub switch_to: String,
}

fn native(op: &str, error: git2::Error) -> GitError {
    GitError::Native(format!("{op}: {}", error.message()))
}

fn io(op: &str, path: &Path, error: std::io::Error) -> GitError {
    GitError::Native(format!("{op} {}: {error}", path.display()))
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|byte| *byte == 0)
}

impl Repo {
    /// The git directory (`.git`, or a linked worktree's own).
    pub fn git_dir(&self) -> &Path {
        self.inner.path()
    }

    /// Snapshot the working tree. Read-only: the index file is never
    /// refreshed here, so a status can not wake the file watcher.
    pub fn status(&self) -> Result<WorkingTree, GitError> {
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(false)
            .exclude_submodules(true);
        let statuses = self
            .inner
            .statuses(Some(&mut options))
            .map_err(|error| native("status", error))?;
        let mut entries = Vec::new();
        for status in statuses.iter() {
            let rel = PathBuf::from(std::ffi::OsStr::from_bytes(status.path_bytes()));
            let flags = status.status();
            let mut push = |section, change| {
                entries.push(StatusEntry {
                    section,
                    change,
                    rel: rel.clone(),
                })
            };
            if flags.is_conflicted() {
                push(Section::Conflicts, Change::Conflicted);
                continue;
            }
            if flags.is_index_new() {
                push(Section::Staged, Change::Added);
            } else if flags.is_index_modified() || flags.is_index_renamed() {
                push(Section::Staged, Change::Modified);
            } else if flags.is_index_deleted() {
                push(Section::Staged, Change::Deleted);
            } else if flags.is_index_typechange() {
                push(Section::Staged, Change::TypeChanged);
            }
            if flags.is_wt_new() {
                push(Section::Untracked, Change::Untracked);
            } else if flags.is_wt_modified() || flags.is_wt_renamed() {
                push(Section::Unstaged, Change::Modified);
            } else if flags.is_wt_deleted() {
                push(Section::Unstaged, Change::Deleted);
            } else if flags.is_wt_typechange() {
                push(Section::Unstaged, Change::TypeChanged);
            }
        }
        Ok(WorkingTree {
            head: self.head_name(),
            tracking: self.tracking(),
            entries,
        })
    }

    /// The branch HEAD names — also for an unborn HEAD, where
    /// `Repository::head` fails but the symbolic target is known.
    fn head_name(&self) -> Option<String> {
        if let Some(branch) = self.head_branch() {
            if self.inner.head_detached().unwrap_or(false) {
                return self.head_sha().map(|sha| sha.chars().take(7).collect());
            }
            return Some(branch);
        }
        let head = self.inner.find_reference("HEAD").ok()?;
        let target = head.symbolic_target()?;
        Some(
            target
                .strip_prefix("refs/heads/")
                .unwrap_or(target)
                .to_string(),
        )
    }

    fn tracking(&self) -> Option<Tracking> {
        let head = self.inner.head().ok()?;
        if !head.is_branch() {
            return None;
        }
        let local = git2::Branch::wrap(head);
        let upstream = local.upstream().ok()?;
        let name = upstream.name().ok()??.to_string();
        let (ahead, behind) = self
            .inner
            .graph_ahead_behind(local.get().target()?, upstream.get().target()?)
            .ok()?;
        Some(Tracking {
            upstream: name,
            ahead,
            behind,
        })
    }

    /// The entry's change as hunks: index↔worktree for unstaged and
    /// untracked paths, HEAD↔index for staged ones. Bytes stay bytes.
    pub fn file_delta(&self, entry: &StatusEntry) -> Result<FileDelta, GitError> {
        let rel = entry.rel.as_path();
        let (old, new) = match entry.section {
            Section::Staged => (
                self.head_bytes(rel),
                self.index_bytes(rel).unwrap_or_default(),
            ),
            Section::Unstaged | Section::Untracked | Section::Conflicts => {
                let path = self.workdir.join(rel);
                let new = match std::fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_file() => {
                        std::fs::read(&path).map_err(|error| io("read", &path, error))?
                    }
                    // deleted, or not a regular file: the new side is empty
                    _ => Vec::new(),
                };
                (self.index_bytes(rel).or_else(|| self.head_bytes(rel)), new)
            }
        };
        if is_binary(&new) || old.as_deref().is_some_and(is_binary) {
            return Ok(FileDelta::Binary);
        }
        Ok(FileDelta::Hunks(hunks_from_buffers(
            old.as_deref(),
            &new,
            rel,
        )?))
    }

    /// `git add -- <rel>`: the worktree state of one path (a file, a
    /// deletion, or everything under an untracked directory) into the
    /// index. Adding a conflicted path marks it resolved.
    pub fn stage_path(&self, rel: &Path) -> Result<(), GitError> {
        let mut index = self.inner.index().map_err(|error| native("index", error))?;
        index.read(true).map_err(|error| native("index", error))?;
        let path = self.workdir.join(rel);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => index
                .add_all([rel], git2::IndexAddOption::DEFAULT, None)
                .map_err(|error| native("stage", error))?,
            Ok(_) => index
                .add_path(rel)
                .map_err(|error| native("stage", error))?,
            Err(_) => index
                .remove_path(rel)
                .map_err(|error| native("stage", error))?,
        }
        index.write().map_err(|error| native("index", error))
    }

    /// `git restore --staged -- <rel>`: the index entry back to HEAD's
    /// (removed when HEAD has none, or is unborn).
    pub fn unstage_path(&self, rel: &Path) -> Result<(), GitError> {
        let head = self
            .inner
            .head()
            .ok()
            .and_then(|head| head.peel(git2::ObjectType::Commit).ok());
        self.inner
            .reset_default(head.as_ref(), [rel])
            .map_err(|error| native("unstage", error))
    }

    /// Throw away the unstaged change to one path: a tracked file is
    /// restored from the index, an untracked path is deleted. Staged
    /// and conflicted entries refuse — one call never discards two
    /// layers of work.
    pub fn discard_path(&self, entry: &StatusEntry) -> Result<(), GitError> {
        let path = self.workdir.join(&entry.rel);
        match entry.section {
            Section::Untracked => {
                let removed = match std::fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&path),
                    Ok(_) => std::fs::remove_file(&path),
                    Err(error) => Err(error),
                };
                removed.map_err(|error| io("delete", &path, error))
            }
            Section::Unstaged => {
                let mut checkout = git2::build::CheckoutBuilder::new();
                checkout
                    .force()
                    .disable_pathspec_match(true)
                    .path(&entry.rel);
                self.inner
                    .checkout_index(None, Some(&mut checkout))
                    .map_err(|error| native("discard", error))
            }
            Section::Staged | Section::Conflicts => Err(GitError::Native(
                "discard works on unstaged changes; unstage first".into(),
            )),
        }
    }

    /// Revert one unstaged hunk in the working file: its new-side
    /// region is replaced by its old-side lines, byte-precise. The
    /// hunk must come from this file's current index↔worktree delta.
    pub fn discard_hunk(&self, rel: &Path, hunk: &Hunk) -> Result<(), GitError> {
        let path = self.workdir.join(rel);
        let bytes = std::fs::read(&path).map_err(|error| io("read", &path, error))?;
        let lines = split_lines_bytes(&bytes);
        let new_side: Vec<&DiffLine> = hunk
            .lines
            .iter()
            .filter(|line| line.origin != LineOrigin::Deletion)
            .collect();
        let lo = hunk.new_start.saturating_sub(1).min(lines.len());
        let hi = (lo + hunk.new_count).min(lines.len());
        let unchanged = lines[lo..hi].len() == new_side.len()
            && lines[lo..hi]
                .iter()
                .zip(&new_side)
                .all(|((text, _), line)| *text == line.text);
        if !unchanged {
            return Err(GitError::Native(
                "the file changed since this diff was taken".into(),
            ));
        }
        let mut out = Vec::with_capacity(bytes.len());
        let mut write = |text: &[u8], newline: bool| {
            out.extend_from_slice(text);
            if newline {
                out.push(b'\n');
            }
        };
        for (text, newline) in &lines[..lo] {
            write(text, *newline);
        }
        for line in hunk
            .lines
            .iter()
            .filter(|line| line.origin != LineOrigin::Addition)
        {
            write(&line.text, line.has_newline);
        }
        for (text, newline) in &lines[hi..] {
            write(text, *newline);
        }
        std::fs::write(&path, out).map_err(|error| io("write", &path, error))
    }

    /// Local branches (current first), then remote-tracking ones; each
    /// group most recently committed first, names breaking ties.
    pub fn branches(&self) -> Result<Vec<Branch>, GitError> {
        let remotes: Vec<String> = self.remotes().into_iter().map(|(name, _)| name).collect();
        let mut rows: Vec<(Branch, i64)> = Vec::new();
        let listed = self
            .inner
            .branches(None)
            .map_err(|error| native("branches", error))?;
        for item in listed {
            let (branch, kind) = item.map_err(|error| native("branches", error))?;
            let Ok(Some(name)) = branch.name() else {
                continue; // a non-UTF-8 ref name cannot be typed back to git
            };
            let remote = kind == git2::BranchType::Remote;
            if remote && name.ends_with("/HEAD") {
                continue;
            }
            let switch_to = if remote {
                remotes
                    .iter()
                    .filter_map(|remote| name.strip_prefix(remote.as_str())?.strip_prefix('/'))
                    .next()
                    .unwrap_or(name)
                    .to_string()
            } else {
                name.to_string()
            };
            let when = branch
                .get()
                .peel_to_commit()
                .map_or(0, |commit| commit.time().seconds());
            rows.push((
                Branch {
                    name: name.to_string(),
                    remote,
                    current: branch.is_head(),
                    switch_to,
                },
                when,
            ));
        }
        rows.sort_by(|(a, a_when), (b, b_when)| {
            (a.remote, !a.current, std::cmp::Reverse(*a_when), &a.name).cmp(&(
                b.remote,
                !b.current,
                std::cmp::Reverse(*b_when),
                &b.name,
            ))
        });
        Ok(rows.into_iter().map(|(branch, _)| branch).collect())
    }

    /// Write `<gitdir>/COMMIT_EDITMSG` for `tree` (a status the caller
    /// just took): an empty first line (HEAD's message when amending),
    /// then `#` lines naming the branch and what the commit will hold.
    /// Refuses when nothing is staged and this is not an amend.
    pub fn write_commit_template(
        &self,
        tree: &WorkingTree,
        amend: bool,
    ) -> Result<PathBuf, GitError> {
        let staged: Vec<&StatusEntry> = tree.section(Section::Staged).collect();
        if staged.is_empty() && !amend {
            return Err(GitError::NothingStaged);
        }
        let mut text = String::new();
        if amend {
            let head = self
                .inner
                .head()
                .and_then(|head| head.peel_to_commit())
                .map_err(|_| GitError::Native("nothing to amend: no commit yet".into()))?;
            text.push_str(head.message().unwrap_or_default().trim_end());
        }
        text.push_str("\n\n# Write the message above; lines starting with # are left out.\n");
        text.push_str("# :wq commits, :q! aborts.\n#\n");
        if let Some(head) = &tree.head {
            text.push_str(&format!("# On branch {head}\n"));
        }
        if amend {
            text.push_str("# Amending the last commit.\n");
        }
        if staged.is_empty() {
            text.push_str("# No staged changes: only the message changes.\n");
        } else {
            text.push_str("# Changes to be committed:\n");
            for entry in staged {
                text.push_str(&format!(
                    "#\t{} {}\n",
                    entry.change.letter(),
                    entry.rel.to_string_lossy()
                ));
            }
        }
        let unstaged = tree.section(Section::Unstaged).count();
        let untracked = tree.section(Section::Untracked).count();
        if unstaged + untracked > 0 {
            text.push_str(&format!(
                "#\n# Not in this commit: {unstaged} unstaged, {untracked} untracked.\n"
            ));
        }
        let path = self.git_dir().join("COMMIT_EDITMSG");
        std::fs::write(&path, text).map_err(|error| io("write", &path, error))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests;
