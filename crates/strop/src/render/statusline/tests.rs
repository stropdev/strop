//! Cell-grid boundaries for the modeline: what survives when the row
//! runs out of cells. These assert layout behavior — position kept,
//! filename kept, printable cells, bounded width — never message
//! wording.

use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use strop_core::Buffer;
use strop_git::GitContext;

use crate::editor::Editor;

use super::render;
use crate::render::text::width;

/// The modeline row as one string, measured in display cells.
fn row(editor: &Editor, columns: u16, rows: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(columns, rows)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            render(editor, frame, area)
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let y = rows - 1;
    crate::headless::row_symbols(buffer, y).collect()
}

fn editor(text: &str, path: Option<&str>) -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text(text), "/w".into());
    if let Some(path) = path {
        editor.buf_mut().path = Some(std::path::PathBuf::from(path));
    }
    editor
}

#[test]
fn a_long_path_cannot_crowd_out_status_or_position() {
    let mut editor = editor(
        "alpha\nbeta\n",
        Some("/w/crates/a/very/deeply/nested/directory/tree/mod.rs"),
    );
    editor.message = "saved the file".into();
    let row = row(&editor, 48, 4);
    assert!(row.contains("mod.rs"), "the filename survives: {row:?}");
    assert!(
        row.contains("saved the file"),
        "the status survives: {row:?}"
    );
    assert!(row.contains("1:1"), "the position survives: {row:?}");
    assert!(
        !row.contains("nested"),
        "the directory yields before anything essential: {row:?}"
    );
    assert!(width(&row) <= 48, "the row fits its cells: {row:?}");
}

#[test]
fn wide_and_control_labels_stay_printable_whole_graphemes() {
    let mut editor = editor("a\n", None);
    editor.buf_mut().name = Some("界界\x1b㌔z\r".into());
    let wide = row(&editor, 30, 4);
    assert!(
        wide.contains("界界"),
        "wide graphemes are not split: {wide:?}"
    );
    assert!(
        wide.contains('�'),
        "controls become replacement cells: {wide:?}"
    );
    assert!(!wide.contains('\x1b') && !wide.contains('\r'), "{wide:?}");
    assert!(width(&wide) <= 30, "{wide:?}");

    let clipped = row(&editor, 22, 4);
    assert!(
        clipped.contains("界界"),
        "the name clips, not the position: {clipped:?}"
    );
    assert!(clipped.contains('…'), "clipping marks itself: {clipped:?}");
    assert!(clipped.contains("1:1"), "{clipped:?}");
    assert!(width(&clipped) <= 22, "{clipped:?}");
}

#[test]
fn narrow_rows_keep_the_mode_accent_and_position() {
    let mut editor = editor("hello\n", Some("/w/longfilename.rs"));
    editor.git = Some(GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: "/w".into(),
        },
        head_sha: None,
        head_branch: Some("main".into()),
        remotes: Vec::new(),
    });
    editor.hunks_untracked = true;
    let row = row(&editor, 16, 4);
    assert!(row.contains("NORMAL"), "the mode stays legible: {row:?}");
    assert!(row.contains("1:1"), "the position stays legible: {row:?}");
    assert!(width(&row) <= 16, "{row:?}");
}

#[test]
fn percent_counts_content_lines_not_the_phantom_row() {
    let mut editor = editor("one\ntwo\nthree\n", None);
    let top = row(&editor, 60, 4);
    assert!(top.contains("33%"), "line 1 of 3: {top:?}");
    editor.feed_text("j");
    let mid = row(&editor, 60, 4);
    assert!(
        mid.contains("66%"),
        "line 2 of 3 — the phantom row is not in the denominator: {mid:?}"
    );
    editor.feed_text("G");
    let last = row(&editor, 60, 4);
    assert!(
        last.contains("100%"),
        "the last content row is 100%: {last:?}"
    );
}

#[test]
fn git_marks_and_flags_render_quietly_beside_the_message() {
    let mut editor = editor("a\n", Some("/w/f.rs"));
    editor.buf_mut().dirty = true;
    editor.buf_mut().readonly = true;
    editor.git = Some(GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: "/w".into(),
        },
        head_sha: Some("abc123".into()),
        head_branch: Some("main".into()),
        remotes: Vec::new(),
    });
    editor.hunks_untracked = true;
    editor.message = "wrote f.rs".into();
    let row = row(&editor, 60, 4);
    assert!(
        row.contains("main*"),
        "branch quiet, worktree dirt marked: {row:?}"
    );
    assert!(
        row.contains("f.rs ●"),
        "the modified signal follows the name: {row:?}"
    );
    assert!(row.contains("[RO]"), "{row:?}");
    assert!(
        row.contains("wrote f.rs"),
        "the message wins the status slot: {row:?}"
    );
}

#[test]
fn historical_delta_names_its_revision_and_file_not_the_worktree() {
    let mut editor = editor("a\n", Some("/w/live.rs"));
    editor.git = Some(GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: "/w".into(),
        },
        head_sha: None,
        head_branch: Some("main".into()),
        remotes: Vec::new(),
    });
    editor.open_delta(
        "delta",
        "src/reader.rs",
        Vec::new(),
        None,
        Some(crate::editor::CommitFiles {
            repo: strop_git::RepoTarget::Local {
                workdir: "/w".into(),
            },
            sha: "abcdef1234567890".into(),
            files: vec![strop_git::memory::ChangedFile {
                path: "src/reader.rs".into(),
                added: 1,
                deleted: 0,
            }],
            current: "src/reader.rs".into(),
        }),
    );
    let row = row(&editor, 80, 4);
    assert!(row.contains("abcdef12"), "the revision is visible: {row}");
    assert!(
        row.contains("reader.rs"),
        "the actual file is visible: {row}"
    );
    assert!(
        !row.contains("main"),
        "history is not labelled as today's branch: {row}"
    );
}

#[test]
fn modeline_respects_its_rectangle_and_empty_areas() {
    let editor = editor("x\n", None);
    let mut terminal = Terminal::new(TestBackend::new(16, 4)).unwrap();
    terminal
        .draw(|frame| {
            for cell in &mut frame.buffer_mut().content {
                cell.set_symbol(".");
            }
            render(&editor, frame, Rect::new(0, 0, 0, 4));
            render(&editor, frame, Rect::new(0, 0, 16, 0));
            render(&editor, frame, Rect::new(2, 1, 10, 2));
        })
        .unwrap();
    let grid = terminal.backend().buffer();
    for y in [0, 1, 3] {
        assert!((0..16).all(|x| grid[(x, y)].symbol() == "."));
    }
    for x in [0, 1, 12, 13, 14, 15] {
        assert_eq!(grid[(x, 2)].symbol(), ".");
    }
    let position: String = (2..12).map(|x| grid[(x, 2)].symbol()).collect();
    assert!(
        position.contains("1:1"),
        "position stays inside the supplied rectangle"
    );
}
