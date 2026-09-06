//! Physical terminal regression: TestBackend alone cannot observe CR/ESC effects.
use crate::editor::Editor;
use ratatui::backend::{Backend, CrosstermBackend, TestBackend};
use ratatui::{buffer::Buffer as CellGrid, layout::Rect, Terminal};
use strop_core::Buffer;

struct Screen {
    terminal: Terminal<TestBackend>,
    previous: CellGrid,
    physical: vt100::Parser,
}
impl Screen {
    fn new(columns: u16, rows: u16) -> Self {
        Self {
            terminal: Terminal::new(TestBackend::new(columns, rows)).unwrap(),
            previous: CellGrid::empty(Rect::new(0, 0, columns, rows)),
            physical: vt100::Parser::new(rows, columns, 0),
        }
    }
    fn draw(&mut self, editor: &mut Editor) {
        self.terminal
            .draw(|frame| crate::render::render(editor, frame))
            .unwrap();
        let next = self.terminal.backend().buffer();
        let mut output = Vec::new();
        CrosstermBackend::new(&mut output)
            .draw(self.previous.diff(next).into_iter())
            .unwrap();
        self.physical.process(&output);
        for y in 0..next.area.height {
            for x in 0..next.area.width {
                let actual = self.physical.screen().cell(y, x).unwrap().contents();
                let actual = if actual.is_empty() { " " } else { &actual };
                assert_eq!(
                    actual,
                    next[(x, y)].symbol(),
                    "physical/model divergence at {x},{y}"
                );
            }
        }
        self.previous = next.clone();
    }
}

#[test]
fn crlf_shrinking_and_deleted_rows_clear_the_physical_terminal() {
    let mut editor = Editor::new(Buffer::from_text(
        "keep const void*, const void*);\r\nsecond;\r\n",
    ));
    let mut screen = Screen::new(60, 10);
    screen.draw(&mut editor);
    editor.feed_text("0wD");
    assert_eq!(editor.buf().line_text(0), "keep ");
    screen.draw(&mut editor);
    screen.draw(&mut editor);
    editor.feed_text("dd");
    screen.draw(&mut editor);
    assert_eq!(editor.buf().line_text(0), "second;");
    assert!(screen
        .physical
        .screen()
        .rows(0, 60)
        .nth(1)
        .unwrap()
        .starts_with('~'));
}

#[test]
fn text_and_metadata_cannot_inject_terminal_escape_sequences() {
    let mut editor = Editor::new(Buffer::from_text("a\x1b[2Jb\x07c\n"));
    editor.buf_mut().name = Some("name\x1b]52;c;clipboard\x07".into());
    let mut screen = Screen::new(70, 10);
    screen.draw(&mut editor);
    assert!(screen.physical.screen().contents().contains("a�[2Jb�c"));
    editor.feed_text("wD");
    screen.draw(&mut editor);
    screen.draw(&mut editor);
}

#[test]
fn search_and_hover_survive_tiny_resize_and_restore() {
    let mut editor = Editor::new(Buffer::from_text("text needle\n"));
    editor.feed_text("/needle");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| crate::render::render(&mut editor, frame))
        .unwrap();
    let initial = terminal.backend().buffer().clone();
    terminal.backend_mut().resize(7, 3);
    editor.hover_card = Some("long hover".into());
    terminal
        .draw(|frame| crate::render::render(&mut editor, frame))
        .unwrap();
    editor.hover_card = None;
    terminal.backend_mut().resize(80, 24);
    terminal
        .draw(|frame| crate::render::render(&mut editor, frame))
        .unwrap();
    assert_eq!(
        terminal.backend().buffer(),
        &initial,
        "same search frame after resize cycle"
    );
}
