//! Render evidence for every built-in palette, isolated from parallel unit tests.
use ratatui::{backend::TestBackend, buffer::Buffer as Grid, Terminal};
use strop_core::{frontend_input::Key, theme::ThemeId, Buffer};

use super::{paint, seed};
use crate::editor::Editor;

fn draw(editor: &mut Editor) -> Grid {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| paint(editor, frame)).unwrap();
    terminal.backend().buffer().clone()
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
