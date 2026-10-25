use super::*;
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn embedded_documentation_languages_reach_the_cell_grid() {
    for (path, source, code_row, code_column, prefix) in [
        (
            "doc.rs",
            "/// ```javascript\n/// const documented = 42;\n/// ```\nfn host() {}\n",
            1,
            4,
            Some((0, 1)),
        ),
        (
            "doc.js",
            "/**\n * ```python\n * def documented():\n *     return 42\n * ```\n */\nfunction host() {}\n",
            2,
            3,
            Some((1, 2)),
        ),
        (
            "doc.md",
            "```go\nfunc documented() {}\n```\n",
            1,
            0,
            None,
        ),
    ] {
        let mut buffer = Buffer::from_text(source);
        buffer.path = Some(path.into());
        let mut editor = Editor::new(buffer);
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        for width in [80, 32] {
            terminal.backend_mut().resize(width, 12);
            editor.analysis_fixture_width(usize::from(width - 1));
            terminal
                .draw(|frame| crate::render::paint(&mut editor, frame))
                .unwrap();
            let inset = editor.left_inset(editor.current()) as u16;
            let grid = terminal.backend().buffer();
            assert_eq!(
                grid[(inset + code_column, code_row)].fg,
                crate::render::class_color(strop_syntax::Class::Keyword),
                "{path} at width {width}: embedded keyword"
            );
            if let Some((column, row)) = prefix {
                assert_eq!(
                    grid[(inset + column, row)].fg,
                    crate::render::class_color(strop_syntax::Class::Comment),
                    "{path} at width {width}: host comment prefix"
                );
            }
        }
    }
}
