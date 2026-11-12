//! Golden frames for the surfaces of 0100 — the commit files, the diff
//! and hunk views, the log and the collection's blocks — in `hone` and
//! `vellum` at 48×16, 80×24, 120×40 and 200×50. The repository is built
//! with fixed names and dates, so every sha in a frame is the same on
//! every run; ages come from injected log rows.
use std::path::Path;

use strop_engine::editor::test_support::git::{editor_in, pump, pump_hunks, settle_dives};

use super::golden::assert_golden;
use super::golden_tests::{frame, settle_analysis, Settle, THEMES};
use crate::editor::Editor;

pub(super) const SIZES: [(u16, u16); 4] = [(48, 16), (80, 24), (120, 40), (200, 50)];

/// The 90-cell path of §9's commit-files scene.
pub(super) const LONG_PATH: &str =
    "crates/strop-engine/src/editor/collections/tests/a_very_long_module_name_for_projection.rs";

pub(super) const LONG_SUBJECT: &str = "rename the doc, change the logo, add a token, and say in one long subject line why every surface is a block now";

/// `git` with a fixed author, committer and clock: the shas never move.
fn git(root: &Path, args: &[&str], day: u32) {
    let date = format!("2026-11-{day:02}T12:00:00Z");
    let done = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "Ada Example")
        .env("GIT_AUTHOR_EMAIL", "ada@example.invalid")
        .env("GIT_COMMITTER_NAME", "Ada Example")
        .env("GIT_COMMITTER_EMAIL", "ada@example.invalid")
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .unwrap();
    assert!(
        done.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&done.stderr)
    );
}

fn source(lines: usize, change: impl Fn(usize) -> Option<String>) -> String {
    (1..=lines)
        .map(|n| change(n).unwrap_or_else(|| format!("    let value_{n} = compute({n});\n")))
        .collect()
}

/// base → a branch of two commits merged back over a main commit → one
/// commit that changes `src/lib.rs` in two places, changes a binary file
/// and adds a file with a 90-cell path, under a 111-cell subject.
pub(super) fn history() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"], 1);
    git(root, &["config", "commit.gpgsign", "false"], 1);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), source(60, |_| None)).unwrap();
    std::fs::write(root.join("notes.md"), "# notes\n").unwrap();
    std::fs::write(root.join("logo.bin"), [0u8, 1, 2, 3, 0, 255]).unwrap();
    git(root, &["add", "."], 1);
    git(
        root,
        &["commit", "-qm", "base: the first commit of the project"],
        1,
    );
    git(root, &["checkout", "-qb", "feature/surfaces"], 2);
    std::fs::write(root.join("notes.md"), "# notes\n\none token\n").unwrap();
    git(root, &["commit", "-qam", "layout: one token"], 2);
    std::fs::write(
        root.join("notes.md"),
        "# notes\n\none token\nthree places\n",
    )
    .unwrap();
    git(
        root,
        &[
            "commit",
            "-qam",
            "surfaces: parse tokens in three places with care",
        ],
        3,
    );
    git(root, &["checkout", "-q", "main"], 4);
    std::fs::write(root.join("README.md"), "# strop\n").unwrap();
    git(root, &["add", "README.md"], 4);
    git(root, &["commit", "-qm", "notes: main moved on"], 4);
    git(
        root,
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/surfaces",
            "-m",
            "Merge branch 'feature/surfaces'",
        ],
        5,
    );
    let changed = source(61, |n| match n {
        2 => Some("    let second = compute(2);\n".into()),
        59 => Some("    let token_59 = parse_token(input, 59);\n".into()),
        61 => Some("    let token_61 = parse_token(input, 61);\n".into()),
        _ => None,
    });
    std::fs::write(root.join("src/lib.rs"), changed).unwrap();
    std::fs::write(root.join("logo.bin"), [0u8, 9, 9, 9, 0, 255, 7]).unwrap();
    let long = root.join(LONG_PATH);
    std::fs::create_dir_all(long.parent().unwrap()).unwrap();
    std::fs::write(&long, "// projection updates\n").unwrap();
    git(root, &["add", "."], 6);
    git(root, &["commit", "-qm", LONG_SUBJECT], 6);
    let editor = editor_in(root, "src/lib.rs");
    (dir, editor)
}

pub(super) fn each(name: &str, scene: impl Fn(&Path, &mut Editor) -> Settle) {
    for theme in THEMES {
        for size in SIZES {
            let (root, mut editor) = history();
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

/// `Space g l`, then Enter on the newest commit: its changed files.
pub(super) fn open_commit_files(editor: &mut Editor) {
    editor.feed_text(" gl");
    pump(editor);
    editor.feed(strop_core::frontend_input::Key::Enter);
    settle_dives(editor);
}

/// From the commit's files, Enter on the file `keys` move to.
fn open_file_delta(editor: &mut Editor, keys: &str) {
    open_commit_files(editor);
    editor.feed_text(keys);
    editor.feed(strop_core::frontend_input::Key::Enter);
    settle_dives(editor);
}

/// A file of a commit, two hunks, beside the commit's file sidebar.
#[test]
fn diff_with_its_sidebar() {
    each("git-diff", |_, editor| {
        open_file_delta(editor, "G");
        settle_analysis()
    });
}

/// A changed binary file: a heading and nothing to show.
#[test]
fn binary_diff() {
    each("git-diff-binary", |_, editor| {
        open_file_delta(editor, "Gk");
        settle_analysis()
    });
}

/// `Space g p` on a changed line: the hunk, named for its file.
#[test]
fn hunk_preview() {
    each("git-hunk", |_, editor| {
        editor.feed_text("30Gcclet thirty = compute(30);<esc>");
        pump_hunks(editor);
        editor.feed_text(" gp");
        settle_dives(editor);
        settle_analysis()
    });
}
