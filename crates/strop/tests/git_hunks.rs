//! Git hunks on reachable rows (#240), on the real binary: the shipped
//! `strop headless` stages from the only row of an emptied file and the
//! last row after an end-of-file deletion; `git` then reads the index.
use std::path::Path;
use std::process::Command;

const NOTE: &str = "one\ntwo\nthree\n";

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// `note` committed as [`NOTE`], then rewritten as `worktree` on disk.
fn repository(worktree: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("note"), NOTE).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "note"]);
    git(
        &root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.org",
            "commit",
            "-qm",
            "base",
        ],
    );
    std::fs::write(root.join("note"), worktree).unwrap();
    dir
}

fn run(root: &Path, script: &str) -> String {
    let outside = root.parent().unwrap();
    let steps = outside.join("steps");
    std::fs::write(&steps, script).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_strop"))
        .current_dir(root)
        .env("HOME", outside.join("home"))
        .env("XDG_CONFIG_HOME", outside.join("config"))
        .env("XDG_STATE_HOME", outside.join("state"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("STROP_LOG")
        .arg("headless")
        .arg(&steps)
        .arg("note")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn last_state(output: &str) -> serde_json::Value {
    let state = output
        .lines()
        .filter_map(|line| line.strip_prefix("─── state "))
        .next_back()
        .unwrap();
    serde_json::from_str(state).unwrap()
}

#[test]
fn emptied_file_stages_from_its_only_row() {
    let dir = repository("");
    let root = dir.path().join("repo");
    let output = run(&root, "settle\nframe\nkeys <space>gs\nsettle\nstate\n");
    assert_eq!(last_state(&output)["message"], "hunk staged", "{output}");
    assert_eq!(
        git(&root, &["show", ":note"]),
        b"",
        "index holds the empty file"
    );
    assert_eq!(
        git(&root, &["ls-files", "note"]),
        b"note\n",
        "still tracked"
    );
}

#[test]
fn eof_deletion_stages_from_the_last_row() {
    let dir = repository("one\ntwo\n");
    let root = dir.path().join("repo");
    let output = run(&root, "settle\nframe\nkeys G<space>gs\nsettle\nstate\n");
    assert_eq!(last_state(&output)["message"], "hunk staged", "{output}");
    assert_eq!(git(&root, &["show", ":note"]), b"one\ntwo\n");
}
