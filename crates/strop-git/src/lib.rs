//! strop-git: the working surface (0001 pillar 3.1). libgit2 for the hot
//! paths — no process spawn per keystroke. HEAD vs the *live buffer*
//! (not the disk file), so gutter signs track unsaved edits.

pub mod memory;

mod diff;
mod repo;
mod revision;

pub use diff::{DiffLine, FileDiff, Hunk, HunkKind, LineOrigin, Sign};
pub use repo::{GitContext, GitError, Repo};
pub use revision::{GitRevision, SourceLocation};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
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
        assert!(repo.hunks(&path, &content).unwrap().is_empty());
    }

    #[test]
    fn change_and_add_and_delete() {
        let (_d, repo, path) = fixture();
        let edited = "fn a() {}\nfn b2() {}\nfn c() {}\nfn d() {}\n";
        let hunks = repo.hunks(&path, edited).unwrap();
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
        let hunks = repo.hunks(&path, edited).unwrap();
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
        let hunks = repo.hunks(&path, edited).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, HunkKind::Delete);
        assert!(hunks[0].covers(2, 4)); // sign on the line after the gap
    }

    #[test]
    fn stage_hunk_applies_to_index() {
        let (_d, repo, path) = fixture();
        let edited = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n";
        let hunks = repo.hunks(&path, edited).unwrap();
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
        let hunks = repo.hunks(&path, edited).unwrap();
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
        let staged = repo.staged_hunks(&path).unwrap();
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
        let hunks = repo.hunks(&path, edited).unwrap();
        repo.stage_hunk(Path::new("f.rs"), &hunks[0]).unwrap();
        assert_eq!(repo.index_content(&path).as_deref(), Some(edited));
        let staged = repo.staged_hunks(&path).unwrap();
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

    /// R9: a path outside the workdir is a typed refusal, not an
    /// empty Vec masquerading as "no hunks".
    #[test]
    fn outside_workdir_is_typed_not_empty() {
        let (_d, repo, _path) = fixture();
        let outside = std::env::temp_dir().join("strop-outside-f.rs");
        assert!(matches!(
            repo.hunks(&outside, "x\n"),
            Err(GitError::OutsideWorkdir)
        ));
        assert!(matches!(
            repo.unstaged_hunks(&outside, "x\n"),
            Err(GitError::OutsideWorkdir)
        ));
        assert!(matches!(
            repo.staged_hunks(&outside),
            Err(GitError::OutsideWorkdir)
        ));
    }

    /// An untracked file's unstaged set is one all-add hunk against
    /// empty — a useful case, distinct from failure; is_untracked
    /// says so without touching content.
    #[test]
    fn untracked_file_is_all_add_not_failure() {
        let (d, repo, _path) = fixture();
        let untracked = d.path().join("new.rs");
        std::fs::write(&untracked, "fn n() {}\n").unwrap();
        assert!(repo.is_untracked(&untracked).unwrap());
        let hunks = repo.unstaged_hunks(&untracked, "fn n() {}\n").unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].kind, HunkKind::Add);
        assert_eq!(hunks[0].old_count, 0);
        // an empty untracked buffer is an honest empty set
        assert!(repo.unstaged_hunks(&untracked, "").unwrap().is_empty());
        // the committed file is tracked: empty diff, not all-add
        assert!(!repo.is_untracked(&_path).unwrap());
    }

    /// An unborn HEAD (fresh init, no commits) still diffs: staging a
    /// new file yields an all-add staged set against empty.
    #[test]
    fn unborn_head_stages_all_add() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        let repo = Repo::discover(root).unwrap();
        std::fs::write(root.join("f.rs"), "fn a() {}\n").unwrap();
        git(root, &["add", "f.rs"]);
        let staged = repo.staged_hunks(&root.join("f.rs")).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].kind, HunkKind::Add);
        // context snapshot: head_sha absent until the first commit
        let ctx = repo.context();
        assert_eq!(ctx.head_sha, None);
        assert_eq!(ctx.workdir, root.to_path_buf());
    }

    /// The pure context round-trips through serde (replay tapes carry
    /// it) and equality tracks the repository state it captured.
    #[test]
    fn git_context_serde_and_equality() {
        let (d, repo, _path) = fixture();
        let ctx = repo.context();
        let wire = serde_json::to_string(&ctx).unwrap();
        assert_eq!(serde_json::from_str::<GitContext>(&wire).unwrap(), ctx);
        assert!(ctx.head_sha.is_some());
        // a new commit changes HEAD: the context is no longer equal —
        // cached diffs built against it are stale
        std::fs::write(d.path().join("f.rs"), "fn a() {}\nfn z() {}\n").unwrap();
        git(d.path(), &["commit", "-qam", "z"]);
        let repo2 = Repo::discover(d.path()).unwrap();
        assert_ne!(repo2.context(), ctx);
    }
}
