//! Render evidence for every built-in palette, isolated from parallel unit tests.
use ratatui::{backend::TestBackend, buffer::Buffer as Grid, Terminal};
use strop_core::{frontend_input::Key, theme::ThemeId, Buffer};

use super::{paint, seed};
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

fn contains_bg(grid: &Grid, role: strop_core::theme::Rgb) -> bool {
    grid.content.iter().any(|cell| cell.bg == seed(role))
}

fn contains_fg(grid: &Grid, role: strop_core::theme::Rgb) -> bool {
    grid.content.iter().any(|cell| cell.fg == seed(role))
}

#[test]
fn all_themes_reach_source_picker_diff_help_and_statusline() {
    const CHILD: &str = "STROP_THEME_RENDER_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "render::theme_tests::all_themes_reach_source_picker_diff_help_and_statusline",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated palette rendering failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    // Only this exact test runs in the child. No other renderer can observe
    // the global selection changing underneath its assertions.
    for id in ThemeId::ALL {
        strop_core::theme::set_current(id);
        let theme = id.palette();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.txt");
        std::fs::write(&path, "foo foo\n    source evidence\n").unwrap();
        let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
        editor.open_fixture(&path).unwrap();
        let grid = draw(&mut editor);
        assert!(contains_fg(&grid, theme.text), "{} source text", id.name());
        assert!(contains_bg(&grid, theme.base), "{} base", id.name());
        assert!(
            contains_bg(&grid, theme.statusline_bg),
            "{} modeline",
            id.name()
        );
        assert_eq!(
            super::mode_color(crate::editor::Mode::Insert),
            seed(theme.class_string)
        );
        assert_eq!(
            super::mode_color(crate::editor::Mode::Visual),
            seed(theme.class_keyword)
        );
        assert_eq!(
            super::severity_color(strop_lsp::Severity::Warning),
            seed(theme.warning)
        );
        let mix = |a: u8, b: u8| (u16::from(a) * 45 / 100 + u16::from(b) * 55 / 100) as u8;
        assert_eq!(
            super::dim_color(seed(theme.text)),
            ratatui::style::Color::Rgb(
                mix(theme.text.r, theme.base.r),
                mix(theme.text.g, theme.base.g),
                mix(theme.text.b, theme.base.b),
            ),
            "{} backdrop blends against its own base",
            id.name()
        );

        editor.feed_text("v");
        let grid = draw(&mut editor);
        assert!(
            contains_bg(&grid, theme.select_bg),
            "{} visual selection",
            id.name()
        );
        editor.feed(Key::Esc);
        editor.open_picker(strop_picker::Kind::Files);
        editor.wait_picker();
        let grid = draw(&mut editor);
        assert_eq!(grid[(1, 0)].symbol(), "╭");
        assert!(
            contains_bg(&grid, theme.select_bg),
            "{} picker selection",
            id.name()
        );
        assert!(
            contains_bg(&grid, theme.surface),
            "{} preview surface",
            id.name()
        );
        editor.feed(Key::Esc);

        editor.open_search(true);
        editor.paste_bracketed("foo");
        editor.wait_picker();
        editor.feed(Key::Tab);
        editor.paste_bracketed("bar");
        editor.feed(Key::Enter);
        editor.wait_io().unwrap();
        let grid = draw(&mut editor);
        assert!(
            contains_fg(&grid, theme.diag_error),
            "{} removed text",
            id.name()
        );
        assert!(
            contains_fg(&grid, theme.class_string),
            "{} added text",
            id.name()
        );
        assert!(
            contains_bg(&grid, theme.diff_del_bg),
            "{} removed row",
            id.name()
        );
        assert!(
            contains_bg(&grid, theme.diff_add_bg),
            "{} added row",
            id.name()
        );

        editor.feed_text(":help\r");
        assert_eq!(editor.buf().name.as_deref(), Some("help"));
        let grid = draw(&mut editor);
        assert!(contains_fg(&grid, theme.border), "{} help rules", id.name());
        assert!(
            contains_bg(&grid, theme.statusline_bg),
            "{} help modeline",
            id.name()
        );
    }
}

#[test]
fn theme_picker_exposes_full_detail_at_48_and_degrades_at_24() {
    const CHILD: &str = "STROP_THEME_PICKER_RENDER_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "render::theme_tests::theme_picker_exposes_full_detail_at_48_and_degrades_at_24",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated theme picker rendering failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    for id in ThemeId::ALL {
        strop_core::theme::set_current(id);
        let root = tempfile::tempdir().unwrap();
        let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
        editor.feed_text(":theme\r");
        editor.paste_bracketed(id.name());
        editor.wait_picker();
        let grid = draw_at(&mut editor, 48, 16);
        let rendered = grid_text(&grid);
        let text = normalized_grid_text(&grid);
        let detail = rendered
            .lines()
            .find(|line| line.contains("· applied"))
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
        assert!(text.contains("· applied"), "{} marker: {text}", id.name());
        assert!(rendered.contains('✓'), "{} check", id.name());
        assert!(rendered.contains("■■■■"), "{} swatches", id.name());
    }

    strop_core::theme::set_current(ThemeId::Dark);
    let root = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.feed_text(":theme\r");
    editor.wait_picker();
    let grid = draw_at(&mut editor, 24, 10);
    let text = grid_text(&grid);
    assert!(text.contains("Dark"), "tiny label: {text}");
    assert!(text.contains('✓'), "tiny applied check: {text}");
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
    assert_eq!(strop_core::theme::current_id(), ThemeId::PaperDark);

    editor.feed_text(":theme\r");
    editor.paste_bracketed(ThemeId::Light.name());
    editor.wait_picker();
    editor.feed(Key::Esc);
    editor.feed(Key::Esc);
    assert!(!editor.picker_open());
    assert_eq!(strop_core::theme::current_id(), ThemeId::PaperDark);
}
