//! 0083: Weft and Splicery highlighting at the real draw boundary —
//! styled cells through `render::paint` on a TestBackend, one 100×30
//! pane, one narrow pane, and a frame after an edit inside a heredoc
//! body. Fixtures are original miniatures in the showcase families.
use crate::editor::Editor;
use ratatui::style::Color;
use strop_core::Buffer;

const WEFT: &str = "// weft miniature: review context profile target
review showcase {
    context \"realm\" at \"release\";
    profile \"p\";
    target plan = proposal \"plan\";
    asset contract = citation \"contract-4\";
    shared := {1, 2, 3};
    algebra := shared | {9} & shared - {2};
    need contract;
    assume ready: accepts(\"sdk\");
    check members: forall x in algebra { x in shared };
}
";

const SPLICERY: &str = "# splicery miniature: edit replace between heredoc
edit \"docs/operations.md\" {
  @begin := one line \"<!-- policy:begin -->\"
  @end_marker := one line \"<!-- policy:end -->\"
  replace between @begin and @end_marker with <<'POLICY'
Attempts: 3
Backoff: exponential
POLICY
}
";

type Grid = Vec<Vec<(String, Color)>>;

fn draw(
    editor: &mut Editor,
    terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
) -> Grid {
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| {
                    let cell = &buffer[(x, y)];
                    (cell.symbol().to_string(), cell.fg)
                })
                .collect()
        })
        .collect()
}

fn text(row: &[(String, Color)]) -> String {
    row.iter().map(|(symbol, _)| symbol.as_str()).collect()
}

fn row_with<'a>(grid: &'a Grid, needle: &str) -> &'a [(String, Color)] {
    grid.iter()
        .find(|row| text(row).contains(needle))
        .unwrap_or_else(|| panic!("no row contains {needle:?}"))
}

fn fg_at(row: &[(String, Color)], needle: &str) -> Color {
    // `needle`'s byte offset in the concatenated row is NOT its cell
    // index once multibyte cells (the scrollbar's `│`) precede it.
    let row_text = text(row);
    let at = row_text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in row"));
    let mut offset = 0;
    for (symbol, color) in row {
        if at < offset + symbol.len() {
            return *color;
        }
        offset += symbol.len();
    }
    panic!("{needle:?} resolves past the row end");
}

fn editor_with(name: &str, source: &str) -> Editor {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, source).unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    editor.open_fixture(&path).unwrap();
    editor
}

#[test]
fn weft_cells_carry_class_colors_at_both_widths() {
    let class = |class| crate::render::tokens().class(class);
    let keyword = class(strop_syntax::Class::Keyword);
    let string = class(strop_syntax::Class::String);
    let number = class(strop_syntax::Class::Number);
    let operator = class(strop_syntax::Class::Operator);
    let comment = class(strop_syntax::Class::Comment);
    // the theme keeps classes distinguishable on the syntax surface
    assert_ne!(keyword, string);
    assert_ne!(keyword, number);
    assert_ne!(keyword, operator);

    let mut editor = editor_with("showcase.weft", WEFT);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    let _ = draw(&mut editor, &mut terminal);
    editor.wait_analysis();
    let wide = draw(&mut editor, &mut terminal);

    assert_eq!(fg_at(row_with(&wide, "review showcase"), "review"), keyword);
    assert_eq!(
        fg_at(row_with(&wide, "review showcase"), "showcase"),
        crate::render::tokens().class(strop_syntax::Class::Variable),
        "the review name is a plain variable, not a guessed keyword"
    );
    assert_eq!(
        fg_at(row_with(&wide, "context \"realm\""), "\"realm\""),
        string
    );
    assert_eq!(fg_at(row_with(&wide, "// weft"), "//"), comment);
    assert_eq!(fg_at(row_with(&wide, "shared :="), "1"), number);
    // the narrow pane keeps the classes a side-by-side split would
    // show; a width change is a new analysis window, so settle it too
    let mut narrow = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 15)).unwrap();
    let _ = draw(&mut editor, &mut narrow);
    editor.wait_analysis();
    let thin = draw(&mut editor, &mut narrow);
    assert_eq!(
        fg_at(row_with(&thin, "review showcase"), "review"),
        keyword,
        "narrow pane keeps keyword color"
    );
}

#[test]
fn splicery_heredoc_body_stays_colored_across_an_edit_inside_it() {
    let string = crate::render::tokens().class(strop_syntax::Class::String);
    let keyword = crate::render::tokens().class(strop_syntax::Class::Keyword);
    let comment = crate::render::tokens().class(strop_syntax::Class::Comment);

    let mut editor = editor_with("policy.spl", SPLICERY);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    let _ = draw(&mut editor, &mut terminal);
    editor.wait_analysis();
    let settled = draw(&mut editor, &mut terminal);

    assert_eq!(fg_at(row_with(&settled, "edit \"docs"), "edit"), keyword);
    assert_eq!(fg_at(row_with(&settled, "# splicery"), "#"), comment);
    let body = row_with(&settled, "Backoff: exponential");
    assert_eq!(
        fg_at(body, "Backoff"),
        string,
        "heredoc body renders as one raw string span"
    );

    // edit inside the heredoc body: the interim frame must not crash or
    // drop the body's color, and the settled frame must agree
    editor.feed_text("gg6j0i# <esc>");
    let interim = draw(&mut editor, &mut terminal);
    let edited = row_with(&interim, "# Backoff");
    assert_eq!(
        fg_at(edited, "Backoff"),
        string,
        "interim frame keeps the heredoc body string-colored"
    );
    editor.wait_analysis();
    let after = draw(&mut editor, &mut terminal);
    let edited = row_with(&after, "# Backoff");
    assert_eq!(
        fg_at(edited, "#"),
        string,
        "the inserted text joins the raw body span after settle"
    );
    assert_eq!(fg_at(edited, "Backoff"), string);
}

#[test]
fn picker_previews_highlight_both_grammars_through_preparation() {
    // The preview's syntax analysis is admitted with the same window;
    // the keyword paints a different foreground than the declared name.
    for (name, source, head, keyword_spell, name_spell) in [
        (
            "buried.weft",
            "review buried {\n    context \"c\" at \"s\";\n}\n",
            "review buried",
            "review",
            "buried",
        ),
        (
            "buried.spl",
            "edit \"buried.txt\" {\n  @h := one text \"x\"\n}\n",
            "edit \"buried",
            "edit",
            "buried",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(name), source).unwrap();
        let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
        editor.open_picker(strop_picker::Kind::Files);
        editor.wait_picker();
        // The preview's analysis is admitted by preparation (AR01), so
        // the draw must go through the frame boundary, not bare paint.
        let preview = |editor: &mut Editor| -> Grid {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
            terminal
                .draw(|frame| crate::render::frame_capture::draw(editor, frame, false))
                .unwrap();
            let buffer = terminal.backend().buffer();
            (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| {
                            let cell = &buffer[(x, y)];
                            (cell.symbol().to_string(), cell.fg)
                        })
                        .collect()
                })
                .collect()
        };
        let _ = preview(&mut editor);
        editor.wait_previews();
        // the read lands; the next draw admits the preview's syntax
        // analysis, and one more settle lets the cached spans paint
        let _ = preview(&mut editor);
        editor.wait_analysis();
        let grid = preview(&mut editor);
        let row = grid
            .iter()
            .find(|row| text(row).contains(head))
            .unwrap_or_else(|| panic!("{name}: preview row {head:?} rendered"));
        assert_ne!(
            fg_at(row, keyword_spell),
            fg_at(row, name_spell),
            "{name}: preview keyword and name carry different colors"
        );
    }
}
