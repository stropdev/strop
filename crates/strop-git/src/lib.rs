//! strop-git: the working surface (0001 pillar 3.1). libgit2 for the hot
//! paths — no process spawn per keystroke. HEAD vs the *live buffer*
//! (not the disk file), so gutter signs track unsaved edits.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    Add,
    Change,
    Delete,
}

/// One diff hunk between HEAD and the buffer, in 1-based buffer lines.
#[derive(Debug, Clone)]
pub struct Hunk {
    pub kind: HunkKind,
    /// First affected line in the buffer (1-based). For pure deletions
    /// this is the line *after* which content vanished.
    pub new_start: usize,
    pub new_count: usize,
    pub old_start: usize,
    pub old_count: usize,
    /// Diff lines with their origin prefix (+ - space), for preview.
    pub lines: Vec<String>,
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
    /// Signs this hunk produces, derived from its diff lines.
    pub fn signs(&self) -> Vec<(usize, Sign)> {
        let mut out = Vec::new();
        let mut nl = self.new_start;
        for line in &self.lines {
            match line.as_bytes().first() {
                Some(b'+') => {
                    out.push((nl, Sign::AddOrChange));
                    nl += 1;
                }
                Some(b'-') => out.push((nl, Sign::DeleteAfter)),
                _ => nl += 1,
            }
        }
        out
    }

    /// The actual changed region (from +/- lines, not the header, which
    /// includes context): buffer-side `new_first`/`new_count` (1-based)
    /// and HEAD-side `old_first`/`old_count`. For pure deletions
    /// `new_first` is the buffer line *following* the gap.
    pub fn changed_region(&self) -> (usize, usize, usize, usize) {
        let mut nl = self.new_start;
        let mut ol = self.old_start;
        let mut new_lines = Vec::new();
        let mut old_lines = Vec::new();
        for line in &self.lines {
            match line.as_bytes().first() {
                Some(b'+') => {
                    new_lines.push(nl);
                    nl += 1;
                }
                Some(b'-') => {
                    old_lines.push(ol);
                    ol += 1;
                }
                _ => {
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
                    lines: content.lines().map(|l| format!("+{l}")).collect(),
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
        let mut hunks = Vec::new();
        for h in 0..patch.num_hunks() {
            let Ok((header, line_count)) = patch.hunk(h) else {
                continue;
            };
            let mut lines = Vec::with_capacity(line_count);
            for l in 0..line_count {
                if let Ok(line) = patch.line_in_hunk(h, l) {
                    let prefix = match line.origin() {
                        '+' | '-' => line.origin(),
                        _ => ' ',
                    };
                    let text = String::from_utf8_lossy(line.content())
                        .trim_end_matches('\n')
                        .to_string();
                    lines.push(format!("{prefix}{text}"));
                }
            }
            // kind from the actual +/- lines: header counts include
            // context lines, which would mislabel small-file hunks
            let has_plus = lines.iter().any(|l| l.starts_with('+'));
            let has_minus = lines.iter().any(|l| l.starts_with('-'));
            let kind = match (has_plus, has_minus) {
                (true, false) => HunkKind::Add,
                (false, true) => HunkKind::Delete,
                _ => HunkKind::Change,
            };
            hunks.push(Hunk {
                kind,
                new_start: header.new_start() as usize,
                new_count: header.new_lines() as usize,
                old_start: header.old_start() as usize,
                old_count: header.old_lines() as usize,
                lines,
            });
        }
        hunks
    }

    /// Stage one hunk. Prototype path: synthesize a single-hunk patch and
    /// `git apply --cached` it (shell git is the write path per 0001 §3;
    /// libgit2 owns the read hot paths). `rel` is repo-relative.
    pub fn stage_hunk(&self, rel: &Path, hunk: &Hunk) -> Result<(), String> {
        let mut patch = format!("--- a/{}\n+++ b/{}\n", rel.display(), rel.display());
        patch.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
        ));
        for line in &hunk.lines {
            patch.push_str(line);
            patch.push('\n');
        }
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
            .any(|l| l.starts_with("+fn d2") || l.starts_with("+fn d()")));
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
