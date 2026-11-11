use super::*;
use crate::tests::git;
use std::process::Command;

fn out(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Two committed files, `main` checked out.
fn fixture() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "t@t.t"]);
    git(root, &["config", "user.name", "t"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();
    std::fs::write(root.join("b.rs"), "one\ntwo\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "init"]);
    let repo = Repo::discover(root).unwrap();
    (dir, repo)
}

fn rows(tree: &WorkingTree) -> Vec<(Section, char, String)> {
    tree.entries
        .iter()
        .map(|entry| {
            (
                entry.section,
                entry.change.letter(),
                entry.rel.to_string_lossy().into_owned(),
            )
        })
        .collect()
}

#[test]
fn clean_tree_has_a_head_and_no_entries() {
    let (_dir, repo) = fixture();
    let tree = repo.status().unwrap();
    assert_eq!(tree.head.as_deref(), Some("main"));
    assert!(tree.entries.is_empty());
    assert_eq!(tree.tracking, None);
}

#[test]
fn every_state_lands_in_its_section() {
    let (dir, repo) = fixture();
    let root = dir.path();
    // a.rs: staged edit plus a further unstaged edit; b.rs deleted;
    // a new staged file; an untracked file and an untracked directory
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b2() {}\nfn c() {}\n").unwrap();
    git(root, &["add", "a.rs"]);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\nfn b2() {}\nfn c() {}\nfn d() {}\n",
    )
    .unwrap();
    std::fs::remove_file(root.join("b.rs")).unwrap();
    std::fs::write(root.join("new.rs"), "new\n").unwrap();
    git(root, &["add", "new.rs"]);
    std::fs::write(root.join("loose.txt"), "x\n").unwrap();
    std::fs::create_dir(root.join("dir")).unwrap();
    std::fs::write(root.join("dir/inner.txt"), "y\n").unwrap();
    let tree = repo.status().unwrap();
    assert_eq!(
        rows(&tree),
        vec![
            (Section::Staged, 'M', "a.rs".into()),
            (Section::Unstaged, 'M', "a.rs".into()),
            (Section::Unstaged, 'D', "b.rs".into()),
            (Section::Untracked, '?', "dir/".into()),
            (Section::Untracked, '?', "loose.txt".into()),
            (Section::Staged, 'A', "new.rs".into()),
        ]
    );
    assert!(tree.entries[3].is_directory());
}

#[test]
fn unborn_head_names_the_branch_it_will_create() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "trunk"]);
    std::fs::write(dir.path().join("f"), "x\n").unwrap();
    let repo = Repo::discover(dir.path()).unwrap();
    let tree = repo.status().unwrap();
    assert_eq!(tree.head.as_deref(), Some("trunk"));
    assert_eq!(rows(&tree), vec![(Section::Untracked, '?', "f".into())]);
    // staging and unstaging work without a HEAD to reset to
    repo.stage_path(Path::new("f")).unwrap();
    assert_eq!(repo.status().unwrap().entries[0].section, Section::Staged);
    repo.unstage_path(Path::new("f")).unwrap();
    assert_eq!(
        repo.status().unwrap().entries[0].section,
        Section::Untracked
    );
}

#[test]
fn conflicts_are_their_own_section_and_staging_resolves() {
    let (dir, repo) = fixture();
    let root = dir.path();
    git(root, &["checkout", "-qb", "side"]);
    std::fs::write(root.join("b.rs"), "one\nside\n").unwrap();
    git(root, &["commit", "-qam", "side"]);
    git(root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("b.rs"), "one\nmain\n").unwrap();
    git(root, &["commit", "-qam", "main"]);
    git(root, &["merge", "side"]);
    let tree = repo.status().unwrap();
    assert_eq!(rows(&tree), vec![(Section::Conflicts, 'U', "b.rs".into())]);
    assert!(repo.discard_path(&tree.entries[0]).is_err());
    std::fs::write(root.join("b.rs"), "one\nboth\n").unwrap();
    repo.stage_path(Path::new("b.rs")).unwrap();
    assert_eq!(
        rows(&repo.status().unwrap()),
        vec![(Section::Staged, 'M', "b.rs".into())]
    );
}

fn with_origin(root: &Path) -> tempfile::TempDir {
    let origin = tempfile::tempdir().unwrap();
    git(origin.path(), &["init", "-q", "--bare"]);
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            &origin.path().display().to_string(),
        ],
    );
    origin
}

#[test]
fn tracking_counts_ahead_and_behind() {
    let (dir, repo) = fixture();
    let root = dir.path();
    let _origin = with_origin(root);
    git(root, &["push", "-qu", "origin", "main"]);
    std::fs::write(root.join("b.rs"), "one\ntwo\nthree\n").unwrap();
    git(root, &["commit", "-qam", "ahead"]);
    assert_eq!(
        repo.status().unwrap().tracking,
        Some(Tracking {
            upstream: "origin/main".into(),
            ahead: 1,
            behind: 0
        })
    );
}

#[test]
fn stage_and_unstage_paths_match_what_git_sees() {
    let (dir, repo) = fixture();
    let root = dir.path();
    std::fs::write(root.join("a.rs"), "changed\n").unwrap();
    std::fs::remove_file(root.join("b.rs")).unwrap();
    std::fs::create_dir(root.join("dir")).unwrap();
    std::fs::write(root.join("dir/inner.txt"), "y\n").unwrap();
    for path in ["a.rs", "b.rs", "dir/"] {
        repo.stage_path(Path::new(path)).unwrap();
    }
    assert_eq!(
        out(root, &["status", "--porcelain"]),
        "M  a.rs\nD  b.rs\nA  dir/inner.txt\n"
    );
    for path in ["a.rs", "b.rs", "dir/inner.txt"] {
        repo.unstage_path(Path::new(path)).unwrap();
    }
    assert_eq!(
        out(root, &["status", "--porcelain"]),
        " M a.rs\n D b.rs\n?? dir/\n"
    );
}

#[test]
fn discard_restores_tracked_files_and_deletes_untracked_ones() {
    let (dir, repo) = fixture();
    let root = dir.path();
    std::fs::write(root.join("a.rs"), "changed\n").unwrap();
    std::fs::remove_file(root.join("b.rs")).unwrap();
    std::fs::write(root.join("loose.txt"), "x\n").unwrap();
    std::fs::create_dir(root.join("dir")).unwrap();
    std::fs::write(root.join("dir/inner.txt"), "y\n").unwrap();
    for entry in repo.status().unwrap().entries {
        repo.discard_path(&entry).unwrap();
    }
    assert_eq!(out(root, &["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read_to_string(root.join("a.rs")).unwrap(),
        "fn a() {}\nfn b() {}\nfn c() {}\n"
    );
    assert!(root.join("b.rs").exists());
    assert!(!root.join("dir").exists());
}

/// A staged change survives discarding the unstaged one on top of it:
/// the file returns to the index, not to HEAD.
#[test]
fn discard_returns_to_the_index_not_to_head() {
    let (dir, repo) = fixture();
    let root = dir.path();
    std::fs::write(root.join("b.rs"), "one\nstaged\n").unwrap();
    git(root, &["add", "b.rs"]);
    std::fs::write(root.join("b.rs"), "one\nstaged\nloose\n").unwrap();
    let tree = repo.status().unwrap();
    assert!(
        repo.discard_path(&tree.entries[0]).is_err(),
        "staged refuses"
    );
    repo.discard_path(&tree.entries[1]).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("b.rs")).unwrap(),
        "one\nstaged\n"
    );
    assert_eq!(out(root, &["status", "--porcelain"]), "M  b.rs\n");
}

fn hunks(repo: &Repo, entry: &StatusEntry) -> Vec<Hunk> {
    match repo.file_delta(entry).unwrap() {
        FileDelta::Hunks(hunks) => hunks,
        FileDelta::Binary => panic!("text expected"),
    }
}

#[test]
fn file_delta_reads_each_edge() {
    let (dir, repo) = fixture();
    let root = dir.path();
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b2() {}\nfn c() {}\n").unwrap();
    git(root, &["add", "a.rs"]);
    std::fs::write(
        root.join("a.rs"),
        "fn a() {}\nfn b2() {}\nfn c() {}\nfn d() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("loose.txt"), "x\ny\n").unwrap();
    std::fs::write(root.join("blob.bin"), b"a\0b").unwrap();
    let tree = repo.status().unwrap();
    let entry = |section, rel: &str| {
        tree.entries
            .iter()
            .find(|entry| entry.section == section && entry.rel == Path::new(rel))
            .unwrap()
    };
    let staged = hunks(&repo, entry(Section::Staged, "a.rs"));
    assert!(staged[0]
        .lines
        .iter()
        .any(|line| line.text == b"fn b2() {}"));
    assert!(!staged[0].lines.iter().any(|line| line.text == b"fn d() {}"));
    let unstaged = hunks(&repo, entry(Section::Unstaged, "a.rs"));
    assert_eq!(unstaged.len(), 1);
    assert!(unstaged[0]
        .lines
        .iter()
        .any(|line| line.origin == LineOrigin::Addition && line.text == b"fn d() {}"));
    let untracked = hunks(&repo, entry(Section::Untracked, "loose.txt"));
    assert_eq!(untracked[0].new_count, 2);
    assert_eq!(
        repo.file_delta(entry(Section::Untracked, "blob.bin"))
            .unwrap(),
        FileDelta::Binary
    );
}

/// Two separated edits: discarding the first hunk leaves the second,
/// byte for byte, including a missing final newline.
#[test]
fn discard_hunk_reverts_only_that_hunk() {
    let (dir, repo) = fixture();
    let root = dir.path();
    let base: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    std::fs::write(root.join("a.rs"), &base).unwrap();
    git(root, &["commit", "-qam", "twenty"]);
    let edited = base
        .replace("line 2\n", "line two\n")
        .replace("line 20\n", "line twenty");
    std::fs::write(root.join("a.rs"), &edited).unwrap();
    let tree = repo.status().unwrap();
    let delta = hunks(&repo, &tree.entries[0]);
    assert_eq!(delta.len(), 2);
    repo.discard_hunk(Path::new("a.rs"), &delta[0]).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("a.rs")).unwrap(),
        base.replace("line 20\n", "line twenty")
    );
    // the stale first hunk no longer matches the file: refused
    assert!(repo.discard_hunk(Path::new("a.rs"), &delta[0]).is_err());
}

#[test]
fn branches_list_current_first_then_remotes() {
    let (dir, repo) = fixture();
    let root = dir.path();
    let _origin = with_origin(root);
    git(root, &["branch", "feature/x"]);
    git(root, &["push", "-q", "origin", "main", "feature/x"]);
    git(root, &["branch", "-D", "feature/x"]);
    git(root, &["branch", "topic"]);
    let rows: Vec<(String, bool, bool, String)> = repo
        .branches()
        .unwrap()
        .into_iter()
        .map(|branch| (branch.name, branch.remote, branch.current, branch.switch_to))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("main".into(), false, true, "main".into()),
            ("topic".into(), false, false, "topic".into()),
            ("origin/feature/x".into(), true, false, "feature/x".into()),
            ("origin/main".into(), true, false, "main".into()),
        ]
    );
}

#[test]
fn commit_template_lists_what_is_staged() {
    let (dir, repo) = fixture();
    let root = dir.path();
    assert_eq!(
        repo.write_commit_template(&repo.status().unwrap(), false),
        Err(GitError::NothingStaged)
    );
    std::fs::write(root.join("a.rs"), "changed\n").unwrap();
    std::fs::write(root.join("loose.txt"), "x\n").unwrap();
    git(root, &["add", "a.rs"]);
    let path = repo
        .write_commit_template(&repo.status().unwrap(), false)
        .unwrap();
    assert_eq!(path, repo.git_dir().join("COMMIT_EDITMSG"));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("\n\n# "), "{text}");
    assert!(text.contains("# On branch main\n"));
    assert!(text.contains("#\tM a.rs\n"));
    assert!(text.contains("# Not in this commit: 0 unstaged, 1 untracked.\n"));
    // amend starts from the last message and needs nothing staged
    git(root, &["reset", "-q"]);
    let path = repo
        .write_commit_template(&repo.status().unwrap(), true)
        .unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.starts_with("init\n\n# "), "{text}");
    assert!(text.contains("# Amending the last commit.\n"));
}
