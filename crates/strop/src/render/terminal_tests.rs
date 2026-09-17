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
            .draw(|frame| crate::render::paint(editor, frame))
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
    editor.fixture_buf_mut().name = Some("name\x1b]52;c;clipboard\x07".into());
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
        .draw(|frame| crate::render::paint(&mut editor, frame))
        .unwrap();
    terminal.backend_mut().resize(7, 3);
    editor.fixture_set_hover_card(Some("long hover".into()));
    terminal
        .draw(|frame| crate::render::paint(&mut editor, frame))
        .unwrap();
    editor.fixture_set_hover_card(None);
    terminal.backend_mut().resize(80, 24);
    terminal
        .draw(|frame| crate::render::paint(&mut editor, frame))
        .unwrap();
    assert_eq!(
        editor.head(),
        5,
        "resize must preserve the live search destination"
    );
    assert_eq!(editor.pending().text(), "/needle");
    let column = editor
        .buf()
        .cell_col_with_tab(editor.head(), editor.config().tab_size)
        .get()
        - editor.view().hscroll.get();
    assert_eq!(
        terminal.backend().buffer()[((5 + column) as u16, 0)].symbol(),
        "n"
    );
}

#[test]
fn horizontal_clipping_does_not_emit_half_clusters_or_protocol_bytes() {
    // R6: a pane scrolled mid-cluster emits styled blanks (never half
    // a wide glyph), ESC stays the replacement cell, and scrolling back
    // restores the unscrolled cells — physically, not just in the model
    let mut editor = Editor::new(Buffer::from_text("ab\t界e\u{301}\x1bZ\n"));
    let mut config = editor.config().clone();
    config.tab_size = 4;
    editor.set_config(config);
    editor.set_head(10);
    let mut screen = Screen::new(9, 4); // 5 gutter + 3 content + track
    screen.draw(&mut editor);
    assert_eq!(editor.view().hscroll.get(), 6);
    let cell = |screen: &Screen, col: u16| {
        screen
            .physical
            .screen()
            .cell(0, col)
            .unwrap()
            .contents()
            .trim_matches(' ')
            .to_string()
    };
    // the e+combining cluster, the ESC replacement, Z — no half 界,
    // no protocol bytes reach the tty; the track column (0064 §1)
    // closes the pane
    assert_eq!(cell(&screen, 5), "e\u{301}");
    assert_eq!(cell(&screen, 6), "\u{fffd}");
    assert_eq!(cell(&screen, 7), "Z");
    assert_eq!(cell(&screen, 8), "\u{2502}");
    editor.set_head(0);
    screen.draw(&mut editor);
    screen.draw(&mut editor);
    assert_eq!(cell(&screen, 5), "a");
    assert_eq!(cell(&screen, 6), "b");
    // scrolled to the head: the reserved track column stays painted
    assert_eq!(cell(&screen, 8), "\u{2502}");
}

/// 0065 goldens: a live fixture terminal in input or inspection state.
fn modal_terminal_editor() -> Editor {
    use strop_terminal::model::{Color as TColor, Phase, Rgb as TRgb, Style as TStyle};
    let mut editor = Editor::new(Buffer::from_text("origin"));
    editor.terminal_fixture(
        &[
            (
                "idx",
                TStyle {
                    foreground: TColor::Indexed(1),
                    ..TStyle::default()
                },
            ),
            (
                "rgb",
                TStyle {
                    foreground: TColor::Rgb(TRgb {
                        red: 1,
                        green: 2,
                        blue: 3,
                    }),
                    ..TStyle::default()
                },
            ),
            ("def", TStyle::default()),
        ],
        Phase::Running,
    );
    editor
}

/// The physical ctrl chord the outer terminal delivers.
fn ctrl(editor: &mut Editor, ch: char) {
    use strop_core::frontend_input::{Input, KeyCode, KeyEvent, Modifiers};
    let code = KeyCode::Char(ch);
    editor.handle_app_event(crate::editor::events::AppEvent::Input(Input::Key(
        KeyEvent {
            code,
            modifiers: Modifiers {
                control: true,
                ..Default::default()
            },
            ..KeyEvent::press(code)
        },
    )));
}

fn row_text(buffer: &CellGrid, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol().to_string())
        .collect()
}

/// The (x, y) of `needle`'s first cell — matching cell symbols, never
/// byte offsets (▌/│ are multi-byte and would skew a str::find).
fn find_cell(buffer: &CellGrid, needle: &str) -> Option<(u16, u16)> {
    let chars: Vec<char> = needle.chars().collect();
    (0..buffer.area.height).find_map(|y| {
        (0..buffer.area.width).find_map(|x| {
            chars
                .iter()
                .enumerate()
                .all(|(offset, ch)| {
                    buffer
                        .cell((x + offset as u16, y))
                        .is_some_and(|cell| cell.symbol().chars().eq([*ch]))
                })
                .then_some((x, y))
        })
    })
}
/// 0065 S1: terminal-input and inspection are glanceably distinct,
/// honest states — chip text, chip hue and the transient hint all say
/// which side of the glass the keys are on.
#[test]
fn terminal_input_and_inspection_present_distinct_states() {
    let mut editor = modal_terminal_editor();
    let mut terminal = Terminal::new(TestBackend::new(110, 12)).unwrap();
    let statusline_y = 11;
    editor.feed_text("i");
    assert!(editor.terminal_input_active());
    // The entry message names the escape (0065 S2's message surface).
    assert!(
        editor.message().contains("Ctrl-\\ Ctrl-N"),
        "{}",
        editor.message()
    );
    terminal
        .draw(|frame| crate::render::paint(&mut editor, frame))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = row_text(&buffer, statusline_y);
    assert!(row.contains(" TERMINAL "), "{row}");
    let (x, _) = find_cell(&buffer, " TERMINAL ").unwrap();
    assert_eq!(buffer[(x, statusline_y)].bg, super::TERMINAL_CHIP);
    // Ctrl-\ Ctrl-N leaves to inspection: NORMAL chip in the normal
    // accent; with the entry message spent, the steady-state hint names
    // the way back.
    ctrl(&mut editor, '\\');
    ctrl(&mut editor, 'n');
    assert!(!editor.terminal_input_active());
    assert!(
        editor.message().contains("snapshot"),
        "{}",
        editor.message()
    );
    editor.set_message("");
    terminal
        .draw(|frame| crate::render::paint(&mut editor, frame))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = row_text(&buffer, statusline_y);
    assert!(row.contains(" NORMAL "), "{row}");
    assert!(row.contains("snapshot · i returns to input"), "{row}");
    let (x, _) = find_cell(&buffer, " NORMAL ").unwrap();
    assert_eq!(buffer[(x, statusline_y)].bg, super::ACCENT);
}

/// 0065 S3: indexed, RGB and default cell colors resolve through the
/// strop-owned theme seed — on the live mirror and the inspection
/// projection alike. Values are derived from `strop_core::theme`, never
/// restated, so this is the single-source check.
#[test]
fn terminal_content_uses_the_strop_palette_on_both_views() {
    let expected =
        |value: strop_core::theme::Rgb| ratatui::style::Color::Rgb(value.r, value.g, value.b);
    let mut editor = modal_terminal_editor();
    let mut terminal = Terminal::new(TestBackend::new(70, 12)).unwrap();
    let assert_palette = |editor: &mut Editor, terminal: &mut Terminal<TestBackend>| {
        terminal
            .draw(|frame| crate::render::paint(editor, frame))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let idx = find_cell(&buffer, "idx").expect("indexed row");
        assert_eq!(buffer[idx].fg, expected(strop_core::theme::ANSI16[1]));
        let rgb = find_cell(&buffer, "rgb").expect("truecolor row");
        assert_eq!(buffer[rgb].fg, ratatui::style::Color::Rgb(1, 2, 3));
        let def = find_cell(&buffer, "def").expect("default row");
        assert_eq!(buffer[def].fg, expected(strop_core::theme::TEXT));
        assert_eq!(buffer[def].bg, expected(strop_core::theme::BASE));
    };
    editor.feed_text("i");
    assert_palette(&mut editor, &mut terminal);
    ctrl(&mut editor, '\\');
    ctrl(&mut editor, 'n');
    assert!(!editor.terminal_input_active());
    assert_palette(&mut editor, &mut terminal);
}

/// 0065 S3: the palette survives the physical path — the outer terminal
/// receives the theme's RGB values, verified through a vt100 screen, not
/// just the model backend.
#[test]
fn terminal_palette_reaches_the_physical_screen() {
    let mut editor = modal_terminal_editor();
    editor.feed_text("i");
    let mut screen = Screen::new(70, 12);
    // CI shells may export NO_COLOR; crossterm honors it process-wide and
    // would strip the very sequences under test — pin colors on first.
    crossterm::style::Colored::set_ansi_color_disabled(false);
    screen.draw(&mut editor);
    // Cell-based lookup: ▌/│ are multi-byte, so byte offsets lie.
    let physical = |x: u16, y: u16| screen.physical.screen().cell(y, x).unwrap().contents();
    let (x, y) = (0..12)
        .find_map(|y| {
            (0..70).find_map(|x| {
                (physical(x, y) == "i" && physical(x + 1, y) == "d" && physical(x + 2, y) == "x")
                    .then_some((x, y))
            })
        })
        .expect("indexed row on the physical screen");
    let theme = strop_core::theme::ANSI16[1];
    assert_eq!(
        screen.physical.screen().cell(y, x).unwrap().fgcolor(),
        vt100::Color::Rgb(theme.r, theme.g, theme.b)
    );
}
