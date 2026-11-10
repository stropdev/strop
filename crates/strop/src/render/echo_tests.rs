//! The echo row's contract (0088 §2, 0087 U1): precedence, glyphs,
//! hints and clipping, on the real painted frame.
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use strop_core::Buffer;

use crate::editor::Editor;
use crate::render::tokens;

fn grid(editor: &mut Editor, columns: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(columns, height)).unwrap();
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn row(grid: &ratatui::buffer::Buffer, y: u16) -> String {
    crate::headless::row_symbols(grid, y).collect()
}

/// U1: the response to the last key is never hidden by retained info
/// feedback — the echo row shows the fresh message first.
#[test]
fn a_fresh_message_outranks_retained_info_feedback() {
    let mut editor = Editor::new(Buffer::from_text("alpha\n"));
    editor.fixture_feedback_info("Dev container discovery completed");
    let frame = grid(&mut editor, 80, 6);
    let echo = row(&frame, 4);
    assert!(
        echo.contains("● Dev container discovery completed"),
        "{echo:?}"
    );
    assert!(echo.contains(" space a  actions"), "{echo:?}");
    editor.set_message("unsaved changes — :e! to force");
    let frame = grid(&mut editor, 80, 6);
    let echo = row(&frame, 4);
    assert!(
        echo.contains("› unsaved changes — :e! to force"),
        "{echo:?}"
    );
    assert!(!echo.contains("Dev container"), "{echo:?}");
    assert!(
        !row(&frame, 5).contains("unsaved"),
        "messages never enter the statusline: {:?}",
        row(&frame, 5)
    );
}

/// Failure feedback wears the error glyph and its hint on the surface
/// plane; with nothing live the row is not painted at all.
#[test]
fn failures_wear_the_error_glyph_and_an_empty_row_is_not_painted() {
    let mut editor = Editor::new(Buffer::from_text("unsaved\n"));
    let frame = grid(&mut editor, 80, 6);
    assert_eq!(
        row(&frame, 4).trim(),
        "~",
        "no echo row without a message: {:?}",
        row(&frame, 4)
    );
    editor.feed_text(":w\r");
    editor.set_message("");
    let frame = grid(&mut editor, 80, 6);
    let echo = row(&frame, 4);
    assert!(echo.contains("✗ local save failed"), "{echo:?}");
    assert!(echo.contains(" space a  actions"), "{echo:?}");
    let glyph = (0..80).find(|&x| frame[(x, 4)].symbol() == "✗").unwrap();
    assert_eq!(frame[(glyph, 4)].fg, tokens().error);
    assert_eq!(frame[(glyph, 4)].bg, tokens().surface);
}

/// B5: a long message wraps to at most three rows above the statusline,
/// continuation rows aligned under the text, and clips whole graphemes
/// only on the last row, at every width.
#[test]
fn long_messages_wrap_to_three_rows_then_clip() {
    let mut editor = Editor::new(Buffer::from_text("alpha\n"));
    editor.set_message("界".repeat(60));
    for columns in [40u16, 24, 9] {
        let frame = grid(&mut editor, columns, 8);
        assert!(
            row(&frame, 4).starts_with(" › 界"),
            "{columns}: {:?}",
            row(&frame, 4)
        );
        assert!(
            row(&frame, 5).starts_with("   界"),
            "{columns}: {:?}",
            row(&frame, 5)
        );
        assert!(row(&frame, 6).ends_with('…') || row(&frame, 6).trim_end().ends_with('…'));
        assert!(
            row(&frame, 3).trim() == "~",
            "three rows only: {:?}",
            row(&frame, 3)
        );
    }
    editor.set_message("short");
    let frame = grid(&mut editor, 40, 8);
    assert!(
        row(&frame, 6).starts_with(" › short"),
        "{:?}",
        row(&frame, 6)
    );
    assert_eq!(row(&frame, 5).trim(), "~", "one row for a short message");
}

/// The rows step up above the cursor line instead of covering it.
#[test]
fn wrapped_rows_never_cover_the_cursor_line() {
    let text: String = (0..10).map(|n| format!("line {n}\n")).collect();
    let mut editor = Editor::new(Buffer::from_text(&text));
    editor.feed_text("G");
    editor.set_message(format!("{} end", "word ".repeat(12)));
    // 10 rows: panes 0..=8, statusline 9; the cursor (last line) sits on
    // row 8 where the echo would end.
    let frame = grid(&mut editor, 30, 10);
    let caret_row = (0..10)
        .find(|&y| row(&frame, y).contains("line 9"))
        .expect("cursor line visible");
    let echo_rows: Vec<u16> = (0..10)
        .filter(|&y| row(&frame, y).contains("word"))
        .collect();
    assert!(!echo_rows.is_empty());
    assert!(
        echo_rows.iter().all(|&y| y < caret_row),
        "echo {echo_rows:?} above the cursor row {caret_row}"
    );
}

/// The picker owns the frame: no echo row paints beneath its card.
#[test]
fn the_echo_row_yields_to_an_open_picker() {
    let mut editor = Editor::new(Buffer::from_text("alpha\n"));
    editor.set_message("a message that must not leak under the card");
    editor.open_picker(strop_picker::Kind::Buffers);
    editor.wait_picker();
    let frame = grid(&mut editor, 100, 30);
    let text: String = (0..30).map(|y| row(&frame, y)).collect();
    assert!(!text.contains("must not leak"), "{text}");
}
