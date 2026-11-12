//! Golden frames for the working-tree surfaces (0096): the status
//! buffer, the commit message buffer and the branch picker, in `hone`
//! and `vellum` at 140×40 and 80×24. Each scene is a real repository
//! driven through the keys a user presses.
use std::path::Path;

use strop_engine::editor::test_support::git::{settle_work, work_fixture};

use super::golden::assert_golden;
use super::golden_tests::{frame, settle_analysis, Settle, SIZES, THEMES};
use crate::editor::Editor;

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

/// One staged new file, two modified files and one untracked file.
fn change(root: &Path) {
    let lines: String = (1..=20)
        .map(|n| match n {
            2 => "line two\n".to_string(),
            19 => "line nineteen\n".to_string(),
            n => format!("line {n}\n"),
        })
        .collect();
    std::fs::write(root.join("src/lib.rs"), lines).unwrap();
    std::fs::write(root.join("notes.md"), "# notes\n\nstaging is one key.\n").unwrap();
    std::fs::create_dir(root.join("docs")).unwrap();
    std::fs::write(root.join("docs/git.md"), "# Git\n").unwrap();
    std::fs::write(root.join("scratch.txt"), "todo\n").unwrap();
    git(root, &["add", "docs/git.md"]);
}

fn keys(editor: &mut Editor, text: &str) {
    editor.feed_text(text);
    settle_work(editor);
}

fn each(name: &str, scene: impl Fn(&Path, &mut Editor) -> Settle) {
    for theme in THEMES {
        for size in SIZES {
            let (root, mut editor) = work_fixture();
            editor.set_theme(theme);
            let mut settle = scene(root.path(), &mut editor);
            let grid = frame(&mut editor, &mut settle, size);
            assert_golden(
                &format!("{name}-{}-{}x{}", theme.name(), size.0, size.1),
                &grid,
            );
        }
    }
}

/// A committed binary file, then changed: the status shows it and its
/// expansion is a note, not a diff.
fn change_binary(root: &Path) {
    std::fs::write(root.join("logo.bin"), [0u8, 1, 2, 3, 0, 255]).unwrap();
    git(root, &["add", "logo.bin"]);
    git(root, &["commit", "-qm", "logo"]);
    std::fs::write(root.join("logo.bin"), [0u8, 9, 9, 9, 0, 255, 7]).unwrap();
}

/// `Space g g` with two files expanded — one of them binary — and the
/// cursor in the second hunk (0100 §9).
#[test]
fn status_buffer() {
    each("git-status", |root, editor| {
        change_binary(root);
        change(root);
        keys(editor, " gg");
        keys(editor, "3j=");
        keys(editor, "3j=");
        keys(editor, "]c]c");
        settle_analysis()
    });
}

/// `cc` from the status buffer: the message file with its summary and a
/// subject too long for `git log`.
#[test]
fn commit_buffer() {
    each("git-commit", |root, editor| {
        change(root);
        keys(editor, " gg");
        keys(editor, "cc");
        editor.feed_text("i");
        // 80 cells: the subject runs past column 72 and says so
        editor.paste_bracketed(
            "docs: say how staging works, one key per file or hunk, and what a commit buffer.",
        );
        // back to the subject's start, so the frame shows all of it
        editor.feed_text("<esc>0");
        settle_analysis()
    });
}

/// `Space g b`: local branches, then remote ones, then the create row.
#[test]
fn branch_picker() {
    each("git-branches", |root, editor| {
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
        git(root, &["branch", "feature/status-buffer"]);
        git(root, &["branch", "fix/blame-gutter"]);
        git(root, &["push", "-q", "origin", "main", "fix/blame-gutter"]);
        keys(editor, " gb");
        editor.wait_picker();
        settle_analysis()
    });
}
