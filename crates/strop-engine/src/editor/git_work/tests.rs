//! The working-tree flows against real repositories and the real
//! `git`: what a user sees in the status buffer, and what git sees
//! afterwards.

mod commit;
mod status;

use std::path::Path;

use crate::editor::test_support::git::{git_out, settle_work, work_fixture};
use crate::editor::Editor;

/// `git <args>` in the fixture repository, for setting scenes.
fn git(root: &Path, args: &[&str]) {
    let done = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        done.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&done.stderr)
    );
}

/// `git status --porcelain`, one row per line.
fn porcelain(root: &Path) -> String {
    let done = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .unwrap();
    String::from_utf8_lossy(&done.stdout).into_owned()
}

/// Lines 2 and 19 of `src/lib.rs` edited (two hunks), `notes.md`
/// edited, and one untracked file.
fn dirty(root: &Path) {
    let lines: String = (1..=20)
        .map(|n| match n {
            2 => "line two\n".to_string(),
            19 => "line nineteen\n".to_string(),
            n => format!("line {n}\n"),
        })
        .collect();
    std::fs::write(root.join("src/lib.rs"), lines).unwrap();
    std::fs::write(root.join("notes.md"), "# notes\nmore\n").unwrap();
    std::fs::write(root.join("scratch.txt"), "todo\n").unwrap();
}

/// The dirty fixture with `Space g g` open and its snapshot landed.
fn status_fixture() -> (tempfile::TempDir, Editor) {
    let (dir, mut e) = work_fixture();
    dirty(dir.path());
    e.feed_text(" gg");
    settle_work(&mut e);
    (dir, e)
}

fn text(e: &Editor) -> String {
    e.buf().text().to_string()
}

fn cursor_line(e: &Editor) -> String {
    e.buf().line_text(e.buf().line_of(e.head()))
}

/// Put the cursor on the first line that starts with `prefix`.
fn goto(e: &mut Editor, prefix: &str) {
    let line = text(e)
        .lines()
        .position(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no line starts with {prefix:?} in\n{}", text(e)));
    let at = e.buf().line_start(line);
    e.set_head(at);
}

fn key(e: &mut Editor, keys: &str) {
    e.feed_text(keys);
    settle_work(e);
}

fn subject(root: &Path) -> String {
    git_out(root, &["log", "-1", "--format=%s"])
}
