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
    pub text: String,
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

    /// The hunk as a unified-diff patch fragment (`git apply` input).
    /// The prefixed form is derived here — the one place it exists.
    pub fn to_patch(&self, rel: &Path) -> String {
        let mut patch = format!("--- a/{}\n+++ b/{}\n", rel.display(), rel.display());
        patch.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            self.old_start, self.old_count, self.new_start, self.new_count
        ));
        for line in &self.lines {
            let prefix = match line.origin {
                LineOrigin::Addition => '+',
                LineOrigin::Deletion => '-',
                LineOrigin::Context => ' ',
            };
            patch.push(prefix);
            patch.push_str(&line.text);
            patch.push('\n');
        }
        patch
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
    pub fn head_content(&self, path: &Path) -> Option<String> {
        let rel = self.rel_path(path)?;
        let head = self.inner.head().ok()?.peel_to_tree().ok()?;
        let entry = head.get_path(&rel).ok()?;
        let blob = self.inner.find_blob(entry.id()).ok()?;
        String::from_utf8(blob.content().to_vec()).ok()
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
                    lines: content
                        .lines()
                        .enumerate()
                        .map(|(i, l)| DiffLine {
                            origin: LineOrigin::Addition,
                            old_lineno: None,
                            new_lineno: Some(i + 1),
                            text: l.to_string(),
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

    /// Stage one hunk. Prototype path: synthesize a single-hunk patch and
    /// `git apply --cached` it (shell git is the write path per 0001 §3;
    /// libgit2 owns the read hot paths). `rel` is repo-relative.
    pub fn stage_hunk(&self, rel: &Path, hunk: &Hunk) -> Result<(), String> {
        let patch = hunk.to_patch(rel);
        let mut child = std::process::Command::new("git")
            .args([
                "-C",
                &self.workdir.display().to_string(),
                "apply",
                "--cached",
                "--unidiff-zero",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn git: {e}"))?;
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("piped")
            .write_all(patch.as_bytes())
            .map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
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
            let origin = match line.origin() {
                '+' => LineOrigin::Addition,
                '-' => LineOrigin::Deletion,
                _ => LineOrigin::Context,
            };
            // libgit2 numbers are 1-based; the absent side is None.
            let old_lineno = line.old_lineno().map(|n| n as usize);
            let new_lineno = line.new_lineno().map(|n| n as usize);
            let text = String::from_utf8_lossy(line.content())
                .trim_end_matches('\n')
                .to_string();
            lines.push(DiffLine {
                origin,
                old_lineno,
                new_lineno,
                text,
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
mod tests {
    use super::*;
    use std::process::Command;

    fn git(root: &std::path::Path, args: &[&str]) {
        Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
    }

    fn fixture() -> (tempfile::TempDir, Repo, PathBuf) {
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
            .any(|l| l.origin == LineOrigin::Addition && l.text.starts_with("fn d")));
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
            .find(|l| l.origin == LineOrigin::Addition && l.text.starts_with("fn d"))
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

    /// `to_patch` is real `git apply` input: the prefixed form exists
    /// only here, derived from typed origins.
    #[test]
    fn to_patch_is_applyable() {
        let (_d, repo, path) = fixture();
        let edited = "fn a() {}\nfn b2() {}\nfn c() {}\n";
        let hunks = repo.hunks(&path, edited);
        assert_eq!(hunks.len(), 1);
        let patch = hunks[0].to_patch(Path::new("f.rs"));
        assert!(
            patch.starts_with("--- a/f.rs\n+++ b/f.rs\n@@ -1,3 +1,3 @@\n"),
            "{patch}"
        );
        assert!(patch.contains("-fn b() {}\n+fn b2() {}\n"), "{patch}");
        assert!(patch.ends_with(" fn c() {}\n"), "{patch}");
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
        assert!(diff.hunks[0].lines.iter().any(|l| l.text == "fn b2() {}"));
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
}
