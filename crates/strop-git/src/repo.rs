//! The repository: libgit2 in-process (0001 §3 — no shell-outs on
//! hot paths), hunk staging, content by revision.

use std::path::{Path, PathBuf};

use crate::diff::{DiffLine, FileDiff, Hunk, HunkKind, LineOrigin};

/// Why a repository operation failed — typed, not a string and not an
/// empty Vec standing in for "something went wrong" (R9).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GitError {
    /// libgit2 failed underneath (corrupt index, blob read, config…).
    Native(String),
    /// The path is not inside the repository workdir — the caller's
    /// buffer cannot take part in this repository's edges at all.
    OutsideWorkdir,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(message) => write!(f, "{message}"),
            Self::OutsideWorkdir => write!(f, "path is outside the repository workdir"),
        }
    }
}

/// The pure cached view of a repository (R6): no libgit2 handle, no
/// locks, no IO to read — render and command decisions consult this,
/// while every native read runs on a worker. Equality is meaningful:
/// an unchanged context (same HEAD, branch, remotes) means cached
/// diffs stay valid.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitContext {
    #[serde(with = "strop_core::path_serde")]
    pub workdir: PathBuf,
    pub head_sha: Option<String>,
    pub head_branch: Option<String>,
    /// (name, url) pairs; permalink selection is a pure fold over them.
    pub remotes: Vec<(String, String)>,
}

impl GitContext {
    /// Call-shape compatibility with `Repo::workdir` — readers that
    /// only need the repository root work against either.
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }
}

pub struct Repo {
    inner: git2::Repository,
    pub(crate) workdir: PathBuf,
}

impl Repo {
    /// Discover the repository containing `path` (buffer path or cwd).
    pub fn discover(from: &Path) -> Option<Self> {
        let inner = git2::Repository::discover(from).ok()?;
        let workdir = inner.workdir()?.to_path_buf();
        Some(Self { inner, workdir })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Remotes as (name, url) pairs — libgit2 config, no spawn.
    pub fn remotes(&self) -> Vec<(String, String)> {
        let Ok(remotes) = self.inner.remotes() else {
            return vec![];
        };
        remotes
            .iter()
            .flatten()
            .filter_map(|name| {
                self.inner
                    .find_remote(name)
                    .ok()
                    .and_then(|r| r.url().map(|u| (name.to_string(), u.to_string())))
            })
            .collect()
    }

    /// HEAD's full SHA (permalink base — branch always resolves to SHA).
    pub fn head_sha(&self) -> Option<String> {
        Some(
            self.inner
                .head()
                .ok()?
                .peel_to_commit()
                .ok()?
                .id()
                .to_string(),
        )
    }

    /// Current branch (short name; detached HEAD gives the sha prefix).
    pub fn head_branch(&self) -> Option<String> {
        self.inner
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(String::from))
    }

    /// The pure cached view (R6): snapshot HEAD, branch and remotes
    /// once — on a worker — and let render/decisions consult it with
    /// zero native work. An equal context means nothing changed.
    pub fn context(&self) -> GitContext {
        GitContext {
            workdir: self.workdir.clone(),
            head_sha: self.head_sha(),
            head_branch: self.head_branch(),
            remotes: self.remotes(),
        }
    }

    /// Repo-relative path for a buffer path (diff keys are relative).
    fn rel_path(&self, path: &Path) -> Option<PathBuf> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workdir.join(path)
        };
        abs.strip_prefix(&self.workdir)
            .ok()
            .map(|p| p.to_path_buf())
    }

    /// HEAD's content for `path`, if tracked.
    /// HEAD's blob bytes for a repo-relative path (typed, not lossy).
    pub fn head_bytes(&self, rel: &Path) -> Option<Vec<u8>> {
        let commit = self.inner.head().ok()?.peel_to_commit().ok()?;
        let tree = commit.tree().ok()?;
        let entry = tree.get_path(rel).ok()?;
        let blob = self.inner.find_blob(entry.id()).ok()?;
        Some(blob.content().to_vec())
    }

    /// A commit's blob bytes for a repo-relative path.
    pub fn commit_bytes(&self, sha: &str, rel: &Path) -> Option<Vec<u8>> {
        let oid = self.inner.revparse_single(sha).ok()?.id();
        let commit = self.inner.find_commit(oid).ok()?;
        let tree = commit.tree().ok()?;
        let entry = tree.get_path(rel).ok()?;
        let blob = self.inner.find_blob(entry.id()).ok()?;
        Some(blob.content().to_vec())
    }

    /// The index's blob bytes for a repo-relative path (reloads — never
    /// a stale snapshot).
    pub fn index_bytes(&self, rel: &Path) -> Option<Vec<u8>> {
        let mut index = self.inner.index().ok()?;
        index.read(true).ok()?;
        let entry = index.get_path(rel, 0)?;
        let blob = self.inner.find_blob(entry.id).ok()?;
        Some(blob.content().to_vec())
    }

    /// Merge-base oid of two revisions.
    pub fn merge_base(&self, a: &str, b: &str) -> Option<String> {
        let a = self.inner.revparse_single(a).ok()?.id();
        let b = self.inner.revparse_single(b).ok()?.id();
        let base = self.inner.merge_base(a, b).ok()?;
        Some(base.to_string())
    }

    pub fn head_content(&self, path: &Path) -> Option<String> {
        self.head_content_res(path).ok().flatten()
    }

    /// HEAD's content for `path`, distinguishing "not in HEAD's tree"
    /// (Ok(None) — untracked or unborn) from a native failure.
    fn head_content_res(&self, path: &Path) -> Result<Option<String>, GitError> {
        let rel = self.rel_path(path).ok_or(GitError::OutsideWorkdir)?;
        let head = match self.inner.head() {
            Ok(reference) => reference
                .peel_to_tree()
                .map_err(|e| GitError::Native(format!("read HEAD: {e}")))?,
            // an unborn branch has no commits: HEAD knows nothing —
            // the file is new, not failed
            Err(error) if error.code() == git2::ErrorCode::UnbornBranch => {
                return Ok(None);
            }
            Err(error) => return Err(GitError::Native(format!("read HEAD: {error}"))),
        };
        match head.get_path(&rel) {
            // not in HEAD's tree: untracked — the file is new
            Err(_) => Ok(None),
            Ok(entry) => self.blob_utf8(entry.id(), "HEAD").map(Some),
        }
    }

    /// The index's content for `path` (the staged version), if any.
    pub fn index_content(&self, path: &Path) -> Option<String> {
        self.index_content_res(path).ok().flatten()
    }

    /// The index's content distinguishing "nothing staged" (Ok(None))
    /// from a native failure. Reloads — never a stale snapshot.
    fn index_content_res(&self, path: &Path) -> Result<Option<String>, GitError> {
        let rel = self.rel_path(path).ok_or(GitError::OutsideWorkdir)?;
        let mut index = self
            .inner
            .index()
            .map_err(|e| GitError::Native(format!("open index: {e}")))?;
        index
            .read(true)
            .map_err(|e| GitError::Native(format!("reload index: {e}")))?;
        match index.get_path(&rel, 0) {
            Some(entry) => self.blob_utf8(entry.id, "index").map(Some),
            None => Ok(None),
        }
    }

    /// One blob's content as UTF-8; the caller names the edge in the
    /// error so failures read "index blob: …", never anonymous.
    fn blob_utf8(&self, id: git2::Oid, edge: &str) -> Result<String, GitError> {
        let blob = self
            .inner
            .find_blob(id)
            .map_err(|e| GitError::Native(format!("{edge} blob: {e}")))?;
        String::from_utf8(blob.content().to_vec())
            .map_err(|_| GitError::Native(format!("{edge} blob is not UTF-8")))
    }

    /// True when neither the index nor HEAD knows `path` — the buffer
    /// is untracked, so hunk undo has nothing to restore from.
    pub fn is_untracked(&self, path: &Path) -> Result<bool, GitError> {
        Ok(self.index_content_res(path)?.is_none() && self.head_content_res(path)?.is_none())
    }

    /// Hunks between HEAD and the index — the STAGED set (0014 wave 4:
    /// the four states are HEAD → index → worktree → live document, and
    /// every command names its edge). Nothing staged is an honest
    /// empty set; an unborn HEAD diffs the index against empty.
    pub fn staged_hunks(&self, path: &Path) -> Result<Vec<Hunk>, GitError> {
        let rel = self.rel_path(path).ok_or(GitError::OutsideWorkdir)?;
        let Some(index) = self.index_content_res(path)? else {
            return Ok(vec![]);
        };
        let head = self.head_content_res(path)?.unwrap_or_default();
        self.diff_strings(&head, &index, &rel)
    }

    /// Hunks between the index and `content` — the UNSTAGED set (what
    /// the gutter shows while you edit). When nothing is staged this
    /// equals HEAD↔content, matching pre-0.5 behavior. An untracked
    /// file reports one all-add hunk against empty.
    pub fn unstaged_hunks(&self, path: &Path, content: &str) -> Result<Vec<Hunk>, GitError> {
        let rel = self.rel_path(path).ok_or(GitError::OutsideWorkdir)?;
        let base = match self.index_content_res(path)? {
            Some(index) => Some(index),
            None => self.head_content_res(path)?,
        };
        match base {
            Some(base) => self.diff_strings(&base, content, &rel),
            None => self.hunks(path, content),
        }
    }

    /// Unstage one hunk, STRUCTURED (0018): the staged hunk's new side
    /// is what's in the index; swap that region for the old side.
    pub fn unstage_hunk(&self, rel: &Path, hunk: &Hunk) -> Result<(), String> {
        let old_side: Vec<&DiffLine> = hunk
            .lines
            .iter()
            .filter(|l| l.origin != LineOrigin::Addition)
            .collect();
        self.index_region_edit(rel, hunk.new_start, hunk.new_count, &old_side)
    }

    /// Hunks between HEAD and `content` for `path`. Untracked files
    /// report a single all-Add hunk — a useful empty-history case, not
    /// a failure. Native failures are typed, never empty.
    pub fn hunks(&self, path: &Path, content: &str) -> Result<Vec<Hunk>, GitError> {
        let rel = self.rel_path(path).ok_or(GitError::OutsideWorkdir)?;
        match self.head_content_res(path)? {
            Some(old) => self.diff_strings(&old, content, &rel),
            None => Ok(all_add_hunk(content)),
        }
    }

    fn diff_strings(&self, old: &str, new: &str, rel: &Path) -> Result<Vec<Hunk>, GitError> {
        let mut opts = git2::DiffOptions::new();
        opts.context_lines(3);
        let patch = git2::Patch::from_buffers(
            old.as_bytes(),
            Some(rel),
            new.as_bytes(),
            Some(rel),
            Some(&mut opts),
        )
        .map_err(|e| GitError::Native(format!("diff {rel:?}: {e}")))?;
        Ok(hunks_from_patch(&patch))
    }

    /// One file's diff at `sha` vs its first parent, as structured
    /// hunks. The delta view's data (0010 §1) — libgit2, no shell-out,
    /// no re-parsing our own text.
    pub fn commit_file_diff(&self, sha: &str, path: &Path) -> Result<FileDiff, String> {
        let commit = self
            .inner
            .find_commit(git2::Oid::from_str(sha).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let new_tree = commit.tree().map_err(|e| e.to_string())?;
        let old_tree = match commit.parent(0) {
            Ok(parent) => Some(parent.tree().map_err(|e| e.to_string())?),
            // root commit: diff against no tree at all
            Err(_) => None,
        };
        let mut opts = git2::DiffOptions::new();
        opts.context_lines(3)
            .pathspec(path)
            .include_unmodified(false);
        let diff = self
            .inner
            .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))
            .map_err(|e| e.to_string())?;
        let mut file = None;
        for (d, _delta) in diff.deltas().enumerate() {
            let Some(patch) = git2::Patch::from_diff(&diff, d).map_err(|e| e.to_string())? else {
                continue; // binary or unrenderable: nothing to show
            };
            let hunks = hunks_from_patch(&patch);
            let added = hunks
                .iter()
                .flat_map(|h| &h.lines)
                .filter(|l| l.origin == LineOrigin::Addition)
                .count();
            let deleted = hunks
                .iter()
                .flat_map(|h| &h.lines)
                .filter(|l| l.origin == LineOrigin::Deletion)
                .count();
            file = Some(FileDiff {
                path: path.to_path_buf(),
                hunks,
                added,
                deleted,
            });
        }
        file.ok_or_else(|| "no diff for path".to_string())
    }

    /// Stage one hunk, STRUCTURED (0018): read the index blob, swap the
    /// hunk's old-side region for its new-side lines, write the blob
    /// back into the index. No patch serialization — path quoting,
    /// CRLF, and missing-final-newline can't go wrong because nothing
    /// is serialized. `rel` is repo-relative.
    pub fn stage_hunk(&self, rel: &Path, hunk: &Hunk) -> Result<(), String> {
        let new_side: Vec<&DiffLine> = hunk
            .lines
            .iter()
            .filter(|l| l.origin != LineOrigin::Deletion)
            .collect();
        self.index_region_edit(rel, hunk.old_start, hunk.old_count, &new_side)
    }

    /// Replace 1-based line region [start, start+count) of `rel`'s
    /// INDEX blob with the given lines, byte-precise. With an empty
    /// index entry (untracked file) the region is the whole file.
    fn index_region_edit(
        &self,
        rel: &Path,
        start: usize,
        count: usize,
        new_lines: &[&DiffLine],
    ) -> Result<(), String> {
        let mut index = self.inner.index().map_err(|e| e.to_string())?;
        index.read(true).map_err(|e| e.to_string())?; // never a stale in-memory index
        let entry = index.get_path(rel, 0);
        let (old_bytes, mode) = match entry {
            Some(e) => {
                let blob = self
                    .inner
                    .find_blob(e.id)
                    .map_err(|e| format!("index blob: {e}"))?;
                (blob.content().to_vec(), e.mode)
            }
            None => (Vec::new(), 0o100644), // untracked: stage from empty
        };
        let lines = split_lines_bytes(&old_bytes);
        let lo = start.saturating_sub(1).min(lines.len());
        let hi = (lo + count).min(lines.len());
        let mut out: Vec<u8> = Vec::with_capacity(old_bytes.len() + 64);
        for (text, nl) in &lines[..lo] {
            out.extend_from_slice(text);
            if *nl {
                out.push(b'\n');
            }
        }
        for l in new_lines {
            out.extend_from_slice(&l.bytes_with_terminator());
        }
        for (text, nl) in &lines[hi..] {
            out.extend_from_slice(text);
            if *nl {
                out.push(b'\n');
            }
        }
        let oid = self.inner.blob(&out).map_err(|e| e.to_string())?;
        index
            .add(&git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode,
                uid: 0,
                gid: 0,
                file_size: 0,
                id: oid,
                flags: 0,
                flags_extended: 0,
                path: rel.to_string_lossy().replace('\\', "/").into_bytes(),
            })
            .map_err(|e| e.to_string())?;
        index.write().map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// The untracked-file hunk: everything added, against nothing. Empty
/// content is an honest empty set.
fn all_add_hunk(content: &str) -> Vec<Hunk> {
    let count = content.lines().count();
    if count == 0 {
        return vec![];
    }
    vec![Hunk {
        kind: HunkKind::Add,
        new_start: 1,
        new_count: count,
        old_start: 0,
        old_count: 0,
        lines: split_lines_bytes(content.as_bytes())
            .into_iter()
            .enumerate()
            .map(|(i, (text, has_newline))| DiffLine {
                origin: LineOrigin::Addition,
                old_lineno: None,
                new_lineno: Some(i + 1),
                text,
                has_newline,
            })
            .collect(),
    }]
}

/// Byte-precise line split: (content-without-terminator, had-newline)
/// pairs. Unlike str::lines, the final unterminated line keeps its
/// identity — staging round-trips a missing trailing newline (0018).
fn split_lines_bytes(bytes: &[u8]) -> Vec<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            out.push((bytes[start..i].to_vec(), true));
            start = i + 1;
        }
    }
    if start < bytes.len() {
        out.push((bytes[start..].to_vec(), false));
    }
    out
}

/// Typed hunks from a libgit2 patch — the one place line origins and
/// both sides' 1-based numbers are read off the wire.
fn hunks_from_patch(patch: &git2::Patch) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    for h in 0..patch.num_hunks() {
        let Ok((header, line_count)) = patch.hunk(h) else {
            continue;
        };
        let mut lines = Vec::with_capacity(line_count);
        for l in 0..line_count {
            let Ok(line) = patch.line_in_hunk(h, l) else {
                continue;
            };
            // the "\ No newline at end of file" marker arrives as a
            // Context-origin line (libgit2 quirk) — it's patch
            // metadata, not content; has_newline carries its truth
            let raw = line.content();
            if raw.starts_with(b"\\ No newline") || raw.starts_with(b"\n\\ No newline") {
                continue;
            }
            let origin = match line.origin() {
                '+' => LineOrigin::Addition,
                '-' => LineOrigin::Deletion,
                _ => LineOrigin::Context,
            };
            // libgit2 numbers are 1-based; the absent side is None.
            let old_lineno = line.old_lineno().map(|n| n as usize);
            let new_lineno = line.new_lineno().map(|n| n as usize);
            let content = line.content();
            let (text, has_newline) = match content.last() {
                Some(b'\n') => (&content[..content.len() - 1], true),
                _ => (content, false),
            };
            lines.push(DiffLine {
                origin,
                old_lineno,
                new_lineno,
                text: text.to_vec(),
                has_newline,
            });
        }
        hunks.push(Hunk::build(
            header.old_start() as usize,
            header.old_lines() as usize,
            header.new_start() as usize,
            header.new_lines() as usize,
            lines,
        ));
    }
    hunks
}

#[cfg(test)]
mod head_tests {
    use super::*;
    use crate::tests::fixture;
    use std::process::Command;

    #[test]
    fn head_content_probe() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("f.rs"), "fn a() {}\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        let repo = Repo::discover(root).unwrap();
        eprintln!("workdir: {:?}", repo.workdir());
        let abs = root.join("f.rs");
        eprintln!("abs: {:?} rel: {:?}", abs, repo.rel_path(&abs));
        eprintln!("head: {:?}", repo.head_content(&abs));
        assert!(repo.head_content(&abs).is_some());
    }

    /// 0014 wave 4: the four states are real and separately diffable.
    #[test]
    fn four_state_edges() {
        let (_d, repo, path) = fixture();
        // worktree edit, stage it, then edit again (live-only)
        std::fs::write(&path, "fn a() {}\nfn STAGED() {}\nfn c() {}\n").unwrap();
        let staged = repo
            .unstaged_hunks(&path, &std::fs::read_to_string(&path).unwrap())
            .unwrap();
        assert_eq!(staged.len(), 1);
        let hunk = staged.into_iter().next().unwrap();
        repo.stage_hunk(Path::new("f.rs"), &hunk).unwrap();
        // index now differs from HEAD
        let idx = repo.index_content(&path).unwrap();
        assert!(idx.contains("STAGED"));
        let head = repo.head_content(&path).unwrap();
        assert!(!head.contains("STAGED"));
        // staged set: HEAD↔index has the hunk; unstaged (index↔same content) is empty
        assert_eq!(repo.staged_hunks(&path).unwrap().len(), 1);
        let wt = std::fs::read_to_string(&path).unwrap();
        assert!(repo.unstaged_hunks(&path, &wt).unwrap().is_empty());
        // a further live-only edit shows in the unstaged set only
        let live = "fn a() {}\nfn STAGED() {}\nfn c() {}\nfn live()\n";
        let unstaged = repo.unstaged_hunks(&path, live).unwrap();
        assert_eq!(unstaged.len(), 1);
        assert!(unstaged[0]
            .lines
            .iter()
            .any(|l| l.text.starts_with(b"fn live")));
        assert_eq!(
            repo.staged_hunks(&path).unwrap().len(),
            1,
            "staged untouched"
        );
        // unstage reverses the edge
        let staged = repo.staged_hunks(&path).unwrap();
        repo.unstage_hunk(Path::new("f.rs"), &staged[0]).unwrap();
        assert!(repo.staged_hunks(&path).unwrap().is_empty());
        assert!(!repo.index_content(&path).unwrap().contains("STAGED"));
    }
}
