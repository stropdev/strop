//! The welcome card belongs to the pane layer: cards paint over it
//! (0087 functionality critique B6).
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use strop_core::Buffer;

use crate::editor::Editor;

#[test]
fn the_command_card_paints_over_the_welcome_card() {
    let mut editor = Editor::new(Buffer::from_text(""));
    editor.feed_text(":te\t");
    let mut terminal = Terminal::new(TestBackend::new(140, 30)).unwrap();
    terminal
        .draw(|frame| crate::render::paint(&mut editor, frame))
        .unwrap();
    let grid = terminal.backend().buffer().clone();
    let rows: Vec<String> = (0..30)
        .map(|y| crate::headless::row_symbols(&grid, y).collect())
        .collect();
    assert!(
        rows.iter().any(|row| row.contains("terminal")),
        "the ex completion lists :terminal\n{}",
        rows.join("\n")
    );
    for row in rows.iter().filter(|row| row.contains("terminal")) {
        for welcome in [
            "╭",
            "╰",
            "Nothing to configure",
            "keybindings",
            "everything",
        ] {
            assert!(
                !row.contains(welcome),
                "welcome card over the command card: {row:?}"
            );
        }
    }
}
