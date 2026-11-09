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

/// A long message clips whole graphemes inside its row at every width.
#[test]
fn long_messages_clip_inside_the_row() {
    let mut editor = Editor::new(Buffer::from_text("alpha\n"));
    editor.set_message("界".repeat(60));
    for columns in [40u16, 24, 9] {
        let frame = grid(&mut editor, columns, 6);
        let echo = row(&frame, 4);
        assert!(echo.starts_with(" › 界"), "{columns}: {echo:?}");
        assert!(echo.contains('…'), "{columns}: {echo:?}");
    }
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
