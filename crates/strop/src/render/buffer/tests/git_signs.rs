use super::*;
use strop_engine::editor::git_memory::hunk_set::HunkSet;
use strop_git::{DiffLine, Hunk, LineOrigin};

// ---- #240 gutter signs on reachable rows ------------------------------

fn line(origin: LineOrigin, old: Option<usize>, new: Option<usize>, text: &str) -> DiffLine {
    DiffLine {
        origin,
        old_lineno: old,
        new_lineno: new,
        text: text.as_bytes().to_vec(),
        has_newline: true,
    }
}

/// The colour of the gutter sign on screen row `y`, if one is painted
/// in the gutter (left of the text column).
fn gutter_sign(e: &mut Editor, y: u16) -> Option<ratatui::style::Color> {
    let mut terminal = viewport_terminal(30, 6);
    terminal.draw(|f| crate::render::paint(e, f)).unwrap();
    let grid = terminal.backend().buffer();
    (0..6)
        .map(|x| &grid[(x, y)])
        .find(|cell| cell.symbol() == "\u{258e}")
        .map(|cell| cell.fg)
}

#[test]
fn replacement_row_paints_the_change_colour() {
    // `@@ -1,3 +1,3 @@`: two -> TWO, a one-line change run
    let mut e = Editor::new(Buffer::from_text("one\nTWO\nthree\n"));
    let hunk = Hunk::build(
        1,
        3,
        1,
        3,
        vec![
            line(LineOrigin::Context, Some(1), Some(1), "one"),
            line(LineOrigin::Deletion, Some(2), None, "two"),
            line(LineOrigin::Addition, None, Some(2), "TWO"),
            line(LineOrigin::Context, Some(3), Some(3), "three"),
        ],
    );
    e.fixture_set_hunks(HunkSet::from_parts(vec![hunk], 3));
    assert_eq!(gutter_sign(&mut e, 0), None);
    assert_eq!(
        gutter_sign(&mut e, 1),
        Some(crate::render::tokens().diff_change)
    );
    assert_eq!(gutter_sign(&mut e, 2), None);
}

#[test]
fn emptied_file_paints_the_deletion_on_its_only_row() {
    // `@@ -1,3 +0,0 @@`: Git's zero coordinate lands on row 1
    let mut e = Editor::new(Buffer::from_text(""));
    let hunk = Hunk::build(
        1,
        3,
        0,
        0,
        vec![
            line(LineOrigin::Deletion, Some(1), None, "one"),
            line(LineOrigin::Deletion, Some(2), None, "two"),
            line(LineOrigin::Deletion, Some(3), None, "three"),
        ],
    );
    e.fixture_set_hunks(HunkSet::from_parts(vec![hunk], 1));
    assert_eq!(
        gutter_sign(&mut e, 0),
        Some(crate::render::tokens().diff_del)
    );
}
