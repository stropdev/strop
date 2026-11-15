//! 0105: two blank cells between a line number and its text, one rule
//! for every numbered gutter, and the caret on the text column.
use super::*;
use strop_engine::editor::prepare::{number_gutter, NUMBER_GAP};

#[test]
fn a_number_gutter_is_sign_digits_and_the_gap() {
    assert_eq!(NUMBER_GAP, 2);
    for (largest, cells) in [(1, 6), (99, 6), (999, 6), (1000, 7), (10_000, 8)] {
        assert_eq!(number_gutter(largest), cells, "{largest} lines");
    }
}

#[test]
fn text_starts_two_cells_after_the_number_and_the_caret_follows() {
    let mut e = Editor::new(Buffer::from_text(&"fn x() {}\n".repeat(20)));
    let mut terminal = viewport_terminal(30, 6);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 0, 0, 7),
        [" ", " ", " ", "1", " ", " ", "f"]
    );
    terminal.backend_mut().assert_cursor_position((6, 0));
    e.feed_text("l");
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    terminal.backend_mut().assert_cursor_position((7, 0));
    // G keeps the column (nostartofline, as in Neovim); 0 returns to it
    e.feed_text("G0");
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    let y = terminal.get_cursor_position().unwrap().y;
    assert!(y > 0, "the caret followed G down the pane");
    terminal.backend_mut().assert_cursor_position((6, y));
}

#[test]
fn the_gap_is_blank_and_wears_the_cursor_line() {
    let mut e = Editor::new(Buffer::from_text("alpha\nbeta\n"));
    let mut terminal = viewport_terminal(30, 6);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    let grid = terminal.backend().buffer();
    let t = crate::render::tokens();
    let text = e.left_inset(e.current()) as u16;
    for x in text - NUMBER_GAP as u16..text {
        assert_eq!(grid[(x, 0)].symbol(), " ", "gap cell {x}");
        assert_eq!(grid[(x, 0)].bg, t.cursorline, "cursor-line gap cell {x}");
        assert_eq!(grid[(x, 1)].symbol(), " ", "gap cell {x}, row 2");
        assert_ne!(grid[(x, 1)].bg, t.cursorline, "off the cursor line");
    }
    assert_eq!(grid[(text - NUMBER_GAP as u16 - 1, 1)].symbol(), "2");
    assert_eq!(grid[(text, 1)].symbol(), "b");
}

#[test]
fn a_sign_does_not_move_the_text() {
    let plain = Editor::new(Buffer::from_text("let x = 1;\n"));
    let mut signed = Editor::new(Buffer::from_text("let x = 1;\n"));
    signed.fixture_insert_diagnostics(
        signed.current(),
        crate::editor::DocumentDiagnostics {
            revision: signed.buf().revision(),
            items: vec![strop_lsp::ResolvedDiag {
                line: strop_core::id::LineIndex::new(0),
                col: strop_core::id::ByteColumn::new(4),
                severity: strop_lsp::Severity::Error,
                end_line: strop_core::id::LineIndex::new(0),
                end_col: strop_core::id::ByteColumn::new(5),
                message: "unused".into(),
            }],
        },
    );
    assert_eq!(
        plain.left_inset(plain.current()),
        signed.left_inset(signed.current())
    );
}
