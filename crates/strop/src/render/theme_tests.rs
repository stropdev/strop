//! Render evidence for every built-in palette. The theme lives on the
//! editor (0088 §3), so each palette renders in-process beside every
//! other test — no child process, no global to race.
use ratatui::{backend::TestBackend, buffer::Buffer as Grid, Terminal};
use strop_core::{frontend_input::Key, theme::ThemeId, Buffer};

use super::{paint, Tokens};
use crate::editor::Editor;

fn draw(editor: &mut Editor) -> Grid {
    draw_at(editor, 100, 30)
}

fn draw_at(editor: &mut Editor, width: u16, height: u16) -> Grid {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| paint(editor, frame)).unwrap();
    terminal.backend().buffer().clone()
}

fn grid_text(grid: &Grid) -> String {
    let mut text = String::new();
    for y in 0..grid.area.height {
        for x in 0..grid.area.width {
            text.push_str(grid[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

fn normalized_grid_text(grid: &Grid) -> String {
    grid_text(grid)
        .split_whitespace()
        .filter(|word| *word != "│")
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_bg(grid: &Grid, color: ratatui::style::Color) -> bool {
    grid.content.iter().any(|cell| cell.bg == color)
}

fn contains_fg(grid: &Grid, color: ratatui::style::Color) -> bool {
    grid.content.iter().any(|cell| cell.fg == color)
}

#[test]
fn all_themes_reach_source_picker_diff_help_and_statusline() {
    for id in ThemeId::ALL {
        let t = Tokens::new(id);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.txt");
        std::fs::write(&path, "foo foo\n    source evidence\n").unwrap();
        let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
        editor.set_theme(id);
        editor.open_fixture(&path).unwrap();
        let grid = draw(&mut editor);
        assert!(contains_fg(&grid, t.text), "{} source text", id.name());
        assert!(contains_bg(&grid, t.base), "{} base", id.name());
        assert!(
            contains_bg(&grid, t.surface),
            "{} modeline on surface",
            id.name()
        );
        assert!(
            contains_bg(&grid, t.cursorline),
            "{} cursor line",
            id.name()
        );
        assert_eq!(t.mode(crate::editor::Mode::Insert), t.mode_insert);
        assert_eq!(t.mode(crate::editor::Mode::Visual), t.mode_visual);
        assert_eq!(t.mode(crate::editor::Mode::Normal), t.accent);
        assert_eq!(t.severity(strop_lsp::Severity::Warning), t.warning);

        editor.feed_text("v");
        let grid = draw(&mut editor);
        assert!(
            contains_bg(&grid, t.selection),
            "{} visual selection",
            id.name()
        );
        editor.feed(Key::Esc);
        editor.open_picker(strop_picker::Kind::Files);
        editor.wait_picker();
        let grid = draw(&mut editor);
        assert_eq!(grid[(2, 0)].symbol(), "╭");
        assert!(
            contains_bg(&grid, t.raised),
            "{} picker selection",
            id.name()
        );
        assert!(contains_bg(&grid, t.surface), "{} card surface", id.name());
        editor.feed(Key::Esc);
        editor.feed(Key::Esc);

        editor.open_search(true);
        editor.paste_bracketed("foo");
        editor.wait_picker();
        editor.feed(Key::Tab);
        editor.paste_bracketed("bar");
        editor.feed(Key::Enter);
        editor.wait_io().unwrap();
        let grid = draw(&mut editor);
        assert!(contains_fg(&grid, t.diff_del), "{} removed text", id.name());
        assert!(contains_fg(&grid, t.diff_add), "{} added text", id.name());
        assert!(
            contains_bg(&grid, t.diff_del_bg),
            "{} removed row",
            id.name()
        );
        assert!(contains_bg(&grid, t.diff_add_bg), "{} added row", id.name());

        editor.feed_text(":help\r");
        assert_eq!(editor.buf().name.as_deref(), Some("help"));
        let grid = draw(&mut editor);
        assert!(contains_fg(&grid, t.faint), "{} help rules", id.name());
        assert!(contains_bg(&grid, t.raised), "{} help keycaps", id.name());
    }
}

#[test]
fn theme_picker_exposes_full_detail_at_48_and_degrades_at_24() {
    for id in ThemeId::ALL {
        let root = tempfile::tempdir().unwrap();
        let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
        editor.set_theme(id);
        editor.feed_text(":theme\r");
        editor.paste_bracketed(id.name());
        editor.wait_picker();
        let grid = draw_at(&mut editor, 48, 16);
        let rendered = grid_text(&grid);
        let text = normalized_grid_text(&grid);
        let detail = rendered
            .lines()
            .find(|line| line.contains("● applied"))
            .unwrap_or_else(|| panic!("{} selected detail: {rendered}", id.name()));
        assert!(text.contains(id.label()), "{} label: {text}", id.name());
        assert!(
            detail.contains(id.name()),
            "{} canonical detail: {detail}",
            id.name()
        );
        for word in id.description().split_whitespace() {
            assert!(
                text.contains(word),
                "{} complete description missing {word:?}: {text}",
                id.name()
            );
        }
        assert!(rendered.contains("■■■■■■"), "{} six swatches", id.name());
    }

    let root = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.feed_text(":theme\r");
    editor.wait_picker();
    let grid = draw_at(&mut editor, 24, 10);
    let text = grid_text(&grid);
    assert!(text.contains("Hone"), "tiny label: {text}");
    assert!(text.contains('●'), "tiny applied mark: {text}");
    assert!(text.contains('▌'), "tiny selection: {text}");
    assert!(text.contains('❯'), "input remains visible: {text}");
    assert!(!text.contains('■'), "tiny swatches omitted: {text}");
    assert!(
        !text.contains("applied"),
        "tiny detail yields to results: {text}"
    );

    editor.feed(Key::Down);
    assert!(editor.picker_open());
    editor.paste_bracketed(ThemeId::PaperDark.name());
    editor.wait_picker();
    editor.feed(Key::Enter);
    assert!(!editor.picker_open());
    assert_eq!(editor.theme(), ThemeId::PaperDark);

    editor.feed_text(":theme\r");
    editor.paste_bracketed(ThemeId::Vellum.name());
    editor.wait_picker();
    editor.feed(Key::Esc);
    editor.feed(Key::Esc);
    assert!(!editor.picker_open());
    assert_eq!(editor.theme(), ThemeId::PaperDark);
}

/// `:theme NAME` with an unknown name keeps the session palette and
/// names the six choices; a deleted theme is just an unknown name.
#[test]
fn unknown_theme_names_keep_the_palette_and_list_the_choices() {
    let mut editor = Editor::new(Buffer::from_text("x\n"));
    editor.feed_text(":theme nord\r");
    assert_eq!(editor.theme(), ThemeId::Hone);
    assert!(editor
        .message()
        .contains("hone, slate, carbide, vellum, paper, paper-dark"));
    assert!(editor.message().contains("nord"));
}
