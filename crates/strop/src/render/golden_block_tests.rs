//! Golden frames for the surfaces of 0100 — the commit files, the diff
//! and hunk views, the log and the collection's blocks — in `hone` and
//! `vellum` at 48×16, 80×24, 120×40 and 200×50. The repository is built
//! with fixed names and dates, so every sha in a frame is the same on
//! every run; ages come from injected log rows.
use std::path::Path;

use strop_engine::editor::test_support::git::{
    editor_in, log_rows_fixture, pump, pump_hunks, settle_dives,
};

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
    each_in(name, history, scene);
}

fn each_in(
    name: &str,
    fixture: fn() -> (tempfile::TempDir, Editor),
    scene: impl Fn(&Path, &mut Editor) -> Settle,
) {
    for theme in THEMES {
        for size in SIZES {
            let (root, mut editor) = fixture();
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
    read_at_log_now(editor);
}

/// From the commit's files, Enter on the file `keys` move to.
fn open_file_delta(editor: &mut Editor, keys: &str) {
    open_commit_files(editor);
    editor.feed_text(keys);
    editor.feed(strop_core::frontend_input::Key::Enter);
    settle_dives(editor);
}

/// The log rows of §4.3 step B: a merge, two lanes, refs, a subject long
/// enough to be cut, ages counted against [`LOG_NOW`].
fn log_rows() -> Vec<strop_git::log::LogRow> {
    use strop_git::log::{LogCommit, LogRow};
    let refs = |short: &str| match short {
        "0d21f9e" => vec!["HEAD -> main".to_owned()],
        "e8c1034" => vec!["feature/surfaces".to_owned()],
        _ => Vec::new(),
    };
    let commit = |graph: &str, short: &str, days: i64, subject: &str| LogRow {
        graph: graph.to_owned(),
        commit: Some(LogCommit {
            sha: format!("{short}{}", "0".repeat(40 - short.len())),
            short: short.to_owned(),
            refs: refs(short),
            subject: subject.to_owned(),
            author: "Ada Example".to_owned(),
            time: LOG_NOW - days * 86_400,
        }),
    };
    let connector = |graph: &str| LogRow {
        graph: graph.to_owned(),
        commit: None,
    };
    vec![
        commit("* ", "0d21f9e", 2, "notes: revision 5"),
        commit("* ", "fd21dde", 2, "notes: revision 4"),
        commit("* ", "19b4e9a", 3, LONG_SUBJECT),
        commit("*   ", "0d964a0", 3, "Merge branch 'feature/surfaces'"),
        connector("|\\"),
        commit("| * ", "e8c1034", 4, "layout: one token"),
        commit(
            "| * ",
            "ff9ea27",
            4,
            "surfaces: parse tokens in three places with care",
        ),
        commit("* | ", "c756836", 3, "notes: main moved on"),
        connector("|/"),
        commit("* ", "8cc6dd6", 5, "base: the first commit of the project"),
    ]
}

/// The clock the injected log rows' ages are counted against.
const LOG_NOW: i64 = 1_794_916_800;

/// The fixed rows, read at [`LOG_NOW`].
fn inject_log(editor: &mut Editor) {
    log_rows_fixture(editor, log_rows());
    read_at_log_now(editor);
}

/// Ages are counted against [`LOG_NOW`], not the wall clock.
fn read_at_log_now(editor: &mut Editor) {
    let tick = strop_trace::replay::Tick {
        unix_seconds: LOG_NOW,
        ..editor.tape().now()
    };
    editor.tape().set_tick(tick).unwrap();
}

/// `Space g l`: the commit log, step B of §4.3.
#[test]
fn commit_log() {
    each("git-log", |_, editor| {
        editor.feed_text(" gl");
        pump(editor);
        inject_log(editor);
        settle_analysis()
    });
}

/// `Space g h`: the same rows, named for the file.
#[test]
fn file_history() {
    each("git-log-file", |_, editor| {
        editor.feed_text(" gh");
        pump(editor);
        inject_log(editor);
        settle_analysis()
    });
}

/// Enter on a commit: its files, one with a 90-cell path.
#[test]
fn commit_files() {
    each("git-files", |_, editor| {
        open_commit_files(editor);
        settle_analysis()
    });
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

/// The collection's long path of §1 (97 cells).
pub(super) const COLLECTION_PATH: &str =
    "crates/strop-engine/src/editor/collections/tests/a_very_long_module_name_for_projection_updates.rs";

/// `token` lines among `value` lines: a hit on the first and on the last
/// line, three on adjacent lines, two on every hit line.
fn tokens(lines: usize, hits: &[usize]) -> String {
    source(lines, |n| {
        hits.contains(&n)
            .then(|| format!("    let token_{n} = parse_token(input, {n});\n"))
    })
}

/// §1's scene: a 97-cell path with hits on its first and last lines and
/// on adjacent lines, a second file, and a read-only third.
pub(super) fn collection_workspace() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let long = root.join(COLLECTION_PATH);
    std::fs::create_dir_all(long.parent().unwrap()).unwrap();
    std::fs::write(&long, tokens(60, &[1, 3, 20, 21, 22, 40, 58, 60])).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/b.rs"), tokens(12, &[6])).unwrap();
    let readonly = root.join("src/long.rs");
    std::fs::write(&readonly, tokens(8, &[4])).unwrap();
    let mut permissions = std::fs::metadata(&readonly).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&readonly, permissions).unwrap();
    let mut editor = Editor::new_in(strop_core::Buffer::from_text(""), root.to_path_buf());
    editor.open_fixture(&root.join("src/b.rs")).unwrap();
    editor.wait_analysis();
    (dir, editor)
}

/// `Ctrl-O` on a search for `token`, then `]f`: the cursor on the second
/// file's heading.
#[test]
fn collection_blocks() {
    each_in("collection-blocks", collection_workspace, |_, editor| {
        editor.open_search(false);
        editor.paste_bracketed("token");
        editor.wait_picker();
        editor.feed(strop_core::frontend_input::Key::CtrlO);
        editor.wait_io().unwrap();
        editor.feed_text("]f");
        settle_analysis()
    });
}
