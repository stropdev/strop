//! 0100 §9's properties over painted frames: every block surface, both
//! themes, every width from 20 to 200. Which row is a heading, a break or
//! a blank comes from the surfaces' typed rows, never from the glyphs;
//! the frame is then checked against what those rows promise.
use ratatui::backend::{Backend, TestBackend};
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use strop_engine::editor::test_support::git::{
    log_rows_fixture, pump, pump_hunks, settle_dives, settle_work, work_fixture,
};

use super::golden_block_tests::{collection_workspace, history, open_commit_files};
use super::golden_git_tests::{change, change_binary, keys};
use super::golden_tests::THEMES;
use super::style::Tokens;
use crate::editor::{CollectionRow, DiffRow, Editor, FilesRow, StatusRow, Surface};
use strop_engine::editor::prepare::Gutter;

/// What a buffer line is, read from its surface's typed rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Heading,
    Break,
    Blank,
    /// A line of a file or a diff: it scrolls, it is never cut.
    Line,
    Other,
}

fn kinds(editor: &Editor) -> Vec<Kind> {
    let doc = editor.current();
    let lines = editor.buf().last_content_line() + 1;
    (0..lines)
        .map(|line| {
            if let Some(kind) = editor.collection_row_kind(doc, line) {
                return match kind {
                    CollectionRow::Heading(_) => Kind::Heading,
                    CollectionRow::Gap => Kind::Break,
                    CollectionRow::Blank => Kind::Blank,
                    CollectionRow::Title => Kind::Other,
                    CollectionRow::Body => Kind::Line,
                };
            }
            match editor.surface() {
                Some(surface @ Surface::Diff { .. }) => match surface.diff_row(line) {
                    Some(DiffRow::Heading) => Kind::Heading,
                    Some(DiffRow::HunkHeader(_)) => Kind::Break,
                    _ => Kind::Line,
                },
                Some(Surface::Status { view, .. }) => match view.row(line) {
                    Some(StatusRow::Section(_)) => Kind::Heading,
                    Some(StatusRow::HunkHeader { .. }) => Kind::Break,
                    Some(StatusRow::Blank) => Kind::Blank,
                    Some(StatusRow::Line { .. }) => Kind::Line,
                    _ => Kind::Other,
                },
                Some(Surface::ChangedFiles { files, .. }) => match files.row(line) {
                    Some(FilesRow::Commit | FilesRow::Files) => Kind::Heading,
                    Some(FilesRow::Blank) => Kind::Blank,
                    _ => Kind::Other,
                },
                _ => Kind::Other,
            }
        })
        .collect()
}

fn paint(editor: &mut Editor, columns: u16, rows: u16) -> (Buffer, (u16, u16)) {
    let mut terminal = Terminal::new(TestBackend::new(columns, rows)).unwrap();
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    editor.wait_analysis();
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    let caret = terminal.backend_mut().get_cursor_position().unwrap();
    (terminal.backend().buffer().clone(), (caret.x, caret.y))
}

fn cells(grid: &Buffer, y: u16, from: usize, to: usize) -> String {
    (from..to)
        .map(|x| grid[(x as u16, y)].symbol())
        .collect::<String>()
}

/// Check one frame of the current buffer against §9.
fn check(editor: &Editor, grid: &Buffer, accent_only_marks: bool, what: &str) {
    let t = Tokens::new(editor.theme());
    let doc = editor.current();
    let kinds = kinds(editor);
    let (gutter_kind, gutter) = editor.gutter(doc);
    let text_column = editor.left_inset(doc);
    let first = text_column - gutter;
    // the scrollbar's column is reserved: content ends one cell before it
    let last = usize::from(grid.area.width).saturating_sub(2);
    if text_column > last {
        return;
    }
    let view = editor.view();
    // the statusline and the echo row (which covers the pane, §8) are
    // not the pane's rows
    let rows = usize::from(grid.area.height)
        .saturating_sub(1 + usize::from(super::echo::rows(editor, &t, grid.area.width)));
    let mut heading_ends = Vec::new();
    for row in 0..rows {
        let line = view.view_top + row;
        let Some(kind) = kinds.get(line).copied() else {
            break;
        };
        let y = row as u16;
        let text = editor.buf().line_text(line);
        let shown = cells(grid, y, text_column, last + 1);
        match kind {
            Kind::Heading => {
                // rules line up: from the pane's first column
                assert_eq!(
                    grid[(first as u16, y)].symbol(),
                    "─",
                    "{what}: rule at {row}"
                );
                let end = (first..=last)
                    .rev()
                    .find(|x| grid[(*x as u16, y)].symbol() != " ")
                    .unwrap_or(first);
                // a name that leaves no room for a leader or a fact ends
                // where it ends; every other heading reaches the edge
                let name_only = text.starts_with(shown.trim_end().trim_end_matches('…'));
                if !name_only {
                    heading_ends.push(end);
                }
                // the name is the buffer's words, or a cut of them
                let name = shown.split(' ').next().unwrap_or_default();
                assert!(
                    text.starts_with(name) || text.ends_with(name) || name.contains('…'),
                    "{what}: heading {row} shows {name:?} for {text:?}"
                );
            }
            Kind::Break => {
                // bands are whole: one background, first to last column
                let bg = grid[(first as u16, y)].bg;
                assert!(
                    (first..=last).all(|x| grid[(x as u16, y)].bg == bg),
                    "{what}: break {row} band"
                );
                assert_ne!(bg, t.base, "{what}: break {row} has a band");
            }
            Kind::Other | Kind::Blank | Kind::Line if view.hscroll.get() == 0 => {
                // caret truth: the painted cells begin with the line
                let visible: String = text.chars().take(shown.chars().count()).collect();
                let cut = crate::render::clip::width(&text) > last + 1 - text_column;
                if cut && kind == Kind::Other && gutter_kind == Gutter::List {
                    assert!(shown.contains('…'), "{what}: row {row} cut without …");
                }
                // a row cut to keep its fact (0100 §3.8) shows its words
                // up to the `…`, then the fact
                let prefix: String = match shown.split_once('…') {
                    Some((words, _)) => words.to_owned(),
                    None => visible
                        .chars()
                        .take(shown.chars().count().saturating_sub(1))
                        .filter(|c| !c.is_control())
                        .collect(),
                };
                // a commit file's path yields its start to keep its stats
                // (0068 B4): the one row §3.8 lets paint less than its text
                let clipped_path = shown.starts_with('…');
                if !text.contains('\t') && text.is_ascii() && !clipped_path {
                    assert!(
                        shown.starts_with(&prefix) && text.starts_with(&prefix),
                        "{what}: row {row} shows {shown:?} for {text:?}"
                    );
                }
            }
            Kind::Other | Kind::Blank | Kind::Line => {}
        }
        if accent_only_marks {
            for x in 0..=last {
                let cell = &grid[(x as u16, y)];
                if cell.fg == t.accent && cell.symbol() != " " {
                    let row_text = cells(grid, y, 0, last + 1);
                    let mark = x == first && cell.symbol() == "▌";
                    // the branch you are on: the status's `Head:`, the
                    // log's `HEAD -> main`
                    let branch =
                        row_text.trim_start().starts_with("Head:") || row_text.contains("HEAD -> ");
                    assert!(mark || branch, "{what}: accent at {x},{row}: {row_text:?}");
                }
            }
        }
    }
    heading_ends.dedup();
    assert!(
        heading_ends.len() <= 1,
        "{what}: headings end at {heading_ends:?}"
    );
    // separation: one blank row before each heading but the buffer's
    // first, none after one
    for (line, kind) in kinds.iter().enumerate() {
        if *kind != Kind::Heading {
            continue;
        }
        if line > 0 {
            assert_eq!(kinds[line - 1], Kind::Blank, "{what}: blank before {line}");
            assert!(
                line < 2 || kinds[line - 2] != Kind::Blank,
                "{what}: one blank before {line}"
            );
        }
        if let Some(next) = kinds.get(line + 1) {
            assert_ne!(*next, Kind::Blank, "{what}: nothing after heading {line}");
        }
    }
}

/// Every width from 20 to 200, both themes.
fn sweep(name: &str, accent_only_marks: bool, scene: impl Fn() -> (tempfile::TempDir, Editor)) {
    for theme in THEMES {
        let (_root, mut editor) = scene();
        editor.set_theme(theme);
        for columns in 20..=200 {
            let (grid, caret) = paint(&mut editor, columns, 24);
            caret_truth(&editor, caret, columns);
            check(
                &editor,
                &grid,
                accent_only_marks,
                &format!("{name} {} {columns}", theme.name()),
            );
        }
    }
}

#[test]
fn the_collection_keeps_its_promises_at_every_width() {
    sweep("collection", false, || {
        let (root, mut editor) = collection_workspace();
        editor.open_search(false);
        editor.paste_bracketed("token");
        editor.wait_picker();
        editor.feed(strop_core::frontend_input::Key::CtrlO);
        editor.wait_io().unwrap();
        (root, editor)
    });
}

#[test]
fn the_status_keeps_its_promises_at_every_width() {
    sweep("status", true, || {
        let (root, mut editor) = work_fixture();
        change_binary(root.path());
        change(root.path());
        keys(&mut editor, " gg");
        keys(&mut editor, "3j=");
        keys(&mut editor, "3j=");
        keys(&mut editor, "]c]c");
        settle_work(&mut editor);
        (root, editor)
    });
}

#[test]
fn the_log_keeps_its_promises_at_every_width() {
    sweep("log", true, || {
        let (root, mut editor) = history();
        editor.feed_text(" gl");
        pump(&mut editor);
        let rows = match editor.surface() {
            Some(Surface::CommitLog { rows, .. }) => rows.clone(),
            _ => unreachable!(),
        };
        log_rows_fixture(&mut editor, rows);
        (root, editor)
    });
}

#[test]
fn the_commit_files_keep_their_promises_at_every_width() {
    sweep("files", true, || {
        let (root, mut editor) = history();
        open_commit_files(&mut editor);
        (root, editor)
    });
}

#[test]
fn the_diff_and_the_hunk_keep_their_promises_at_every_width() {
    sweep("diff", false, || {
        let (root, mut editor) = history();
        open_commit_files(&mut editor);
        editor.feed_text("G");
        editor.feed(strop_core::frontend_input::Key::Enter);
        settle_dives(&mut editor);
        (root, editor)
    });
    sweep("hunk", false, || {
        let (root, mut editor) = history();
        editor.feed_text("30Gcclet thirty = compute(30);<esc>");
        pump_hunks(&mut editor);
        editor.feed_text(" gp");
        settle_dives(&mut editor);
        (root, editor)
    });
}

/// The caret column is the engine's inset plus the buffer column, on a
/// heading, a break and a list row alike, for all four gutters (§9).
fn caret_truth(editor: &Editor, (x, y): (u16, u16), columns: u16) {
    let view = editor.view();
    let head = editor.head();
    let line = editor.buf().line_of(head);
    let expected = editor.left_inset(editor.current()) + head - editor.buf().line_start(line);
    if view.hscroll.get() != 0
        || !editor.buf().line_text(line).is_ascii()
        || expected + 1 >= usize::from(columns)
    {
        return;
    }
    assert_eq!(
        (usize::from(x), usize::from(y)),
        (expected, line - view.view_top),
        "caret at {columns} columns"
    );
}
