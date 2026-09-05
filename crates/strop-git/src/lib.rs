//! strop-git: the working surface (0001 pillar 3.1). libgit2 for the hot
//! paths — no process spawn per keystroke. HEAD vs the *live buffer*
//! (not the disk file), so gutter signs track unsaved edits.

pub mod memory;

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    Add,
    Change,
    Delete,
}

/// Where a diff line comes from — addition/deletion carry which side's
/// line number applies (0010 §1: typed origins, never `+`-sniffing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOrigin {
    Context,
    Addition,
    Deletion,
}

/// One line of a hunk: content without prefix, plus the 1-based line
/// number on each side that has one (absent side: `None`, never `0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub origin: LineOrigin,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
    /// The line's bytes WITHOUT its terminator (0018: byte-precise —
    /// a `\r` stays; non-UTF-8 stays bytes).
    pub text: Vec<u8>,
    /// Whether the source line ended in a newline — the missing-final-
    /// newline marker is data, not a patch-text nuance (0018).
    pub has_newline: bool,
}

impl DiffLine {
    /// Display form (lossy at the render edge only).
    pub fn text_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.text)
    }
    /// The line's bytes WITH its terminator, exactly as stored.
    pub fn bytes_with_terminator(&self) -> Vec<u8> {
        let mut b = self.text.clone();
        if self.has_newline {
            b.push(b'\n');
        }
        b
    }
}

/// One diff hunk between two versions of a file, in 1-based lines.
#[derive(Debug, Clone)]
pub struct Hunk {
    pub kind: HunkKind,
    /// First affected line in the new version (1-based). For pure
    /// deletions this is the line *after* which content vanished.
    pub new_start: usize,
    pub new_count: usize,
    pub old_start: usize,
    pub old_count: usize,
    pub lines: Vec<DiffLine>,
}

/// One file's diff at a commit (vs its parent): the delta view's data.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: PathBuf,
    pub hunks: Vec<Hunk>,
    pub added: usize,
    pub deleted: usize,
}

/// One changed line, for gutter signs. Hunk headers include context
/// lines, so signs track the +/- lines, not the header range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    /// Buffer line was added or changed.
    AddOrChange,
    /// Buffer line sits right below a deletion (the line number may be
    /// one past the buffer end for an EOF deletion — clamp on render).
    DeleteAfter,
}

impl Hunk {
    /// Signs this hunk produces, derived from its line origins.
    pub fn signs(&self) -> Vec<(usize, Sign)> {
        let mut out = Vec::new();
        let mut nl = self.new_start;
        for line in &self.lines {
            match line.origin {
                LineOrigin::Addition => {
                    out.push((nl, Sign::AddOrChange));
                    nl += 1;
                }
                LineOrigin::Deletion => out.push((nl, Sign::DeleteAfter)),
                LineOrigin::Context => nl += 1,
            }
        }
        out
    }

    /// The actual changed region (from add/del lines, not the header,
    /// which includes context): new-side `new_first`/`new_count`
    /// (1-based) and old-side `old_first`/`old_count`. For pure
    /// deletions `new_first` is the new line *following* the gap.
    pub fn changed_region(&self) -> (usize, usize, usize, usize) {
        let mut nl = self.new_start;
        let mut ol = self.old_start;
        let mut new_lines = Vec::new();
        let mut old_lines = Vec::new();
        for line in &self.lines {
            match line.origin {
                LineOrigin::Addition => {
                    new_lines.push(nl);
                    nl += 1;
                }
                LineOrigin::Deletion => {
                    old_lines.push(ol);
                    ol += 1;
                }
                LineOrigin::Context => {
                    nl += 1;
                    ol += 1;
                }
            }
        }
        let new_first = new_lines.first().copied().unwrap_or(nl);
        let old_first = old_lines.first().copied().unwrap_or(ol);
        (new_first, new_lines.len(), old_first, old_lines.len())
    }

    /// Buffer lines covered (signs render on these); `total_lines`
    /// clamps an EOF deletion onto the last line.
    pub fn covers(&self, line_1based: usize, total_lines: usize) -> bool {
        self.signs().iter().any(|&(l, kind)| match kind {
            Sign::AddOrChange => l == line_1based,
            Sign::DeleteAfter => l.min(total_lines) == line_1based,
        })
    }

    /// The `@@ -a,b +c,d @@` header row as the diff surface shows it.
    pub fn header(&self) -> String {
        format!(
            "@@ -{},{} +{},{} @@",
            self.old_start, self.old_count, self.new_start, self.new_count
        )
    }

    /// Assemble a hunk from its header numbers and typed lines; the
    /// kind comes from the actual origins — header counts include
    /// context lines, which would mislabel small-file hunks.
    pub fn build(
        old_start: usize,
        old_count: usize,
        new_start: usize,
        new_count: usize,
        lines: Vec<DiffLine>,
    ) -> Self {
        let has_add = lines.iter().any(|l| l.origin == LineOrigin::Addition);
        let has_del = lines.iter().any(|l| l.origin == LineOrigin::Deletion);
        let kind = match (has_add, has_del) {
            (true, false) => HunkKind::Add,
            (false, true) => HunkKind::Delete,
            _ => HunkKind::Change,
        };
        Hunk {
            kind,
            new_start,
            new_count,
            old_start,
            old_count,
            lines,
        }
    }
}

pub struct Repo {
    inner: git2::Repository,
    workdir: PathBuf,
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
        let rel = self.rel_path(path)?;
        let head = self.inner.head().ok()?.peel_to_tree().ok()?;
        let entry = head.get_path(&rel).ok()?;
        let blob = self.inner.find_blob(entry.id()).ok()?;
        String::from_utf8(blob.content().to_vec()).ok()
    }

    /// The index's content for `path` (the staged version), if any.
    pub fn index_content(&self, path: &Path) -> Option<String> {
        let rel = self.rel_path(path)?;
        // the shell write path (git apply --cached) owns the on-disk
        // index — reload before reading or we serve a cached snapshot
        let mut index = self.inner.index().ok()?;
        index.read(true).ok()?;
        let entry = index.get_path(&rel, 0)?;
        let blob = self.inner.find_blob(entry.id).ok()?;
        String::from_utf8(blob.content().to_vec()).ok()
    }

    /// Hunks between HEAD and the index — the STAGED set (0014 wave 4:
    /// the four states are HEAD → index → worktree → live document, and
    /// every command names its edge).
    pub fn staged_hunks(&self, path: &Path) -> Vec<Hunk> {
        let Some(rel) = self.rel_path(path) else {
            return vec![];
        };
        let (Some(head), Some(index)) = (self.head_content(path), self.index_content(path)) else {
            return vec![];
        };
        self.diff_strings(&head, &index, &rel)
    }

    /// Hunks between the index and `content` — the UNSTAGED set (what
    /// the gutter shows while you edit). When nothing is staged this
    /// equals HEAD↔content, matching pre-0.5 behavior.
    pub fn unstaged_hunks(&self, path: &Path, content: &str) -> Vec<Hunk> {
        let Some(rel) = self.rel_path(path) else {
            return vec![];
        };
        let base = self.index_content(path).or_else(|| self.head_content(path));
        match base {
            None => self.hunks(path, content), // untracked: all-add
            Some(base) => self.diff_strings(&base, content, &rel),
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
    /// report a single all-Add hunk.
    pub fn hunks(&self, path: &Path, content: &str) -> Vec<Hunk> {
        let Some(rel) = self.rel_path(path) else {
            return vec![];
        };
        let old = self.head_content(path);
        match old {
            None => {
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
            Some(old) => self.diff_strings(&old, content, &rel),
        }
    }

    fn diff_strings(&self, old: &str, new: &str, rel: &Path) -> Vec<Hunk> {
        let mut opts = git2::DiffOptions::new();
        opts.context_lines(3);
        let Ok(patch) = git2::Patch::from_buffers(
            old.as_bytes(),
            Some(rel),
            new.as_bytes(),
            Some(rel),
            Some(&mut opts),
        ) else {
            return vec![];
        };
        hunks_from_patch(&patch)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    pub(crate) fn git(root: &std::path::Path, args: &[&str]) {
        Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
    }

    pub(crate) fn fixture() -> (tempfile::TempDir, Repo, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "t@t.t"]);
        git(root, &["config", "user.name", "t"]);
        std::fs::write(root.join("f.rs"), "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "init"]);
        let repo = Repo::discover(root).unwrap();
        let file = root.join("f.rs");
        (dir, repo, file)
    }

    #[test]
    fn clean_buffer_has_no_hunks() {
        let (_d, repo, path) = fixture();
        let content = repo.head_content(&path).unwrap();
        assert!(repo.hunks(&path, &content).is_empty());
    }

    #[test]
    fn change_and_add_and_delete() {
        let (_d, repo, path) = fixture();
        let edited = "fn a() {}\nfn b2() {}\nfn c() {}\nfn d() {}\n";
        let hunks = repo.hunks(&path, edited);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, HunkKind::Change);
        assert!(hunks[0].covers(2, 4));
        assert!(hunks[0].covers(4, 4));
        assert!(!hunks[0].covers(1, 4));
        assert!(hunks[0]
            .lines
            .iter()
            .any(|l| l.origin == LineOrigin::Addition && l.text.starts_with(b"fn d")));
    }

    /// The typed structure carries both sides' 1-based numbers: the
    /// renderer never guesses them from text (0010 §1).
    #[test]
    fn line_numbers_track_both_sides() {
        let (_d, repo, path) = fixture();
        let edited = "fn a() {}\nfn b2() {}\nfn c() {}\nfn d() {}\n";
        let hunks = repo.hunks(&path, edited);
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        let ctx = h
            .lines
            .iter()
            .find(|l| l.origin == LineOrigin::Context)
            .unwrap();
        assert_eq!(
            (ctx.old_lineno, ctx.new_lineno),
            (Some(1), Some(1)),
            "context lines carry both numbers, 1-based"
        );
        let add = h
            .lines
            .iter()
            .find(|l| l.origin == LineOrigin::Addition && l.text.starts_with(b"fn d"))
            .unwrap();
        assert_eq!((add.old_lineno, add.new_lineno), (None, Some(4)));
        let del = h
            .lines
            .iter()
            .find(|l| l.origin == LineOrigin::Deletion)
            .unwrap();
        assert_eq!((del.old_lineno, del.new_lineno), (Some(2), None));
    }

    #[test]
    fn pure_delete_marks_following_line() {
        let (_d, repo, path) = fixture();
        let edited = "fn a() {}\nfn c() {}\n";
        let hunks = repo.hunks(&path, edited);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, HunkKind::Delete);
        assert!(hunks[0].covers(2, 4)); // sign on the line after the gap
    }

    #[test]
    fn stage_hunk_applies_to_index() {
        let (_d, repo, path) = fixture();
        let edited = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n";
        let hunks = repo.hunks(&path, edited);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, HunkKind::Add);
        let root = repo.workdir.clone();
        repo.stage_hunk(Path::new("f.rs"), &hunks[0]).unwrap();
        let out = Command::new("git")
            .args([
                "-C",
                &root.display().to_string(),
                "diff",
                "--cached",
                "--stat",
            ])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&out.stdout);
        assert!(stat.contains("f.rs"), "{stat}");
    }

    /// Structured staging is byte-precise (0018): stage a hunk, and
    /// the index holds exactly the post-edit bytes — including a
    /// missing final newline, which the old patch path could not
    /// represent.
    #[test]
    fn stage_hunk_is_byte_precise() {
        let (_d, repo, path) = fixture();
        let edited = "fn a() {}\nfn b2() {}\nfn c() {}\n";
        let hunks = repo.hunks(&path, edited);
        assert_eq!(hunks.len(), 1);
        repo.stage_hunk(Path::new("f.rs"), &hunks[0]).unwrap();
        // the index now holds the edited text; HEAD is untouched
        assert_eq!(
            repo.index_content(&path).as_deref(),
            Some("fn a() {}\nfn b2() {}\nfn c() {}\n")
        );
        assert_eq!(
            repo.head_content(&path).as_deref(),
            Some("fn a() {}\nfn b() {}\nfn c() {}\n")
        );
        // and unstaging the same hunk restores the index to HEAD
        let staged = repo.staged_hunks(&path);
        assert_eq!(staged.len(), 1);
        repo.unstage_hunk(Path::new("f.rs"), &staged[0]).unwrap();
        assert_eq!(
            repo.index_content(&path).as_deref(),
            Some("fn a() {}\nfn b() {}\nfn c() {}\n")
        );
    }

    #[test]
    fn stage_hunk_preserves_a_missing_final_newline() {
        let (_d, repo, path) = fixture();
        // the worktree file drops its trailing newline
        let edited = "fn a() {}\nfn b() {}\nfn c() {}";
        let hunks = repo.hunks(&path, edited);
        repo.stage_hunk(Path::new("f.rs"), &hunks[0]).unwrap();
        assert_eq!(repo.index_content(&path).as_deref(), Some(edited));
        let staged = repo.staged_hunks(&path);
        repo.unstage_hunk(Path::new("f.rs"), &staged[0]).unwrap();
        assert_eq!(
            repo.index_content(&path).as_deref(),
            Some("fn a() {}\nfn b() {}\nfn c() {}\n"),
            "unstage restores the newline-terminated HEAD text"
        );
    }

    /// The commit delta view's data: structured hunks at a SHA, via
    /// libgit2 — the `git show` shell-out replacement.
    #[test]
    fn commit_file_diff_is_structured() {
        let (_d, repo, path) = fixture();
        let root = repo.workdir.clone();
        std::fs::write(root.join("f.rs"), "fn a() {}\nfn b2() {}\nfn c() {}\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "change b"]);
        let sha = String::from_utf8_lossy(
            &Command::new("git")
                .args(["-C", &root.display().to_string(), "rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string();
        let diff = repo.commit_file_diff(&sha, Path::new("f.rs")).unwrap();
        assert_eq!(diff.added, 1);
        assert_eq!(diff.deleted, 1);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].kind, HunkKind::Change);
        assert!(diff.hunks[0].lines.iter().any(|l| l.text == b"fn b2() {}"));
        let _ = path;
    }

    /// Root commits diff against the empty tree: the init commit shows
    /// as one all-addition hunk, not an error.
    #[test]
    fn commit_file_diff_root_commit() {
        let (_d, repo, _path) = fixture();
        let root = repo.workdir.clone();
        let sha = String::from_utf8_lossy(
            &Command::new("git")
                .args(["-C", &root.display().to_string(), "rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string();
        let diff = repo.commit_file_diff(&sha, Path::new("f.rs")).unwrap();
        assert_eq!(diff.added, 3);
        assert_eq!(diff.deleted, 0);
        assert!(diff
            .hunks
            .iter()
            .all(|h| h.lines.iter().all(|l| l.old_lineno.is_none())));
    }
}

#[cfg(test)]
mod head_tests {
    use super::tests::fixture;
    use super::*;
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
        let staged = repo.unstaged_hunks(&path, &std::fs::read_to_string(&path).unwrap());
        assert_eq!(staged.len(), 1);
        let hunk = staged.into_iter().next().unwrap();
        repo.stage_hunk(Path::new("f.rs"), &hunk).unwrap();
        // index now differs from HEAD
        let idx = repo.index_content(&path).unwrap();
        assert!(idx.contains("STAGED"));
        let head = repo.head_content(&path).unwrap();
        assert!(!head.contains("STAGED"));
        // staged set: HEAD↔index has the hunk; unstaged (index↔same content) is empty
        assert_eq!(repo.staged_hunks(&path).len(), 1);
        let wt = std::fs::read_to_string(&path).unwrap();
        assert!(repo.unstaged_hunks(&path, &wt).is_empty());
        // a further live-only edit shows in the unstaged set only
        let live = "fn a() {}\nfn STAGED() {}\nfn c() {}\nfn live()\n";
        let unstaged = repo.unstaged_hunks(&path, live);
        assert_eq!(unstaged.len(), 1);
        assert!(unstaged[0]
            .lines
            .iter()
            .any(|l| l.text.starts_with(b"fn live")));
        assert_eq!(repo.staged_hunks(&path).len(), 1, "staged untouched");
        // unstage reverses the edge
        let staged = repo.staged_hunks(&path);
        repo.unstage_hunk(Path::new("f.rs"), &staged[0]).unwrap();
        assert!(repo.staged_hunks(&path).is_empty());
        assert!(!repo.index_content(&path).unwrap().contains("STAGED"));
    }
}
