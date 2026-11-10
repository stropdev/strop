use super::*;
use ratatui::backend::{Backend, TestBackend};
use strop_core::Buffer;
use strop_engine::editor::events;

fn fixture(text: &str, keys: &str) -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text(text), "/completion-cells".into());
    let (sender, receiver) = events::channel();
    editor.connect_events(sender);
    editor.feed_text(keys);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while editor
        .completion_menu()
        .is_none_or(|menu| menu.selected().is_none())
    {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!remaining.is_zero(), "completion never became selectable");
        editor.handle_app_event(receiver.recv_timeout(remaining).unwrap());
    }
    editor
}

fn t() -> Tokens {
    crate::render::tokens()
}

#[test]
fn kind_and_source_colors_reuse_picker_families() {
    let t = t();
    assert_eq!(t.kind("func"), t.syn.function);
    assert_eq!(t.kind("ctor"), t.syn.function);
    assert_eq!(t.kind("class"), t.syn.r#type);
    assert_eq!(t.kind("type"), t.syn.r#type);
    assert_eq!(t.kind("var"), t.syn.string);
    assert_eq!(t.kind("mod"), t.syn.namespace);
    assert_eq!(t.kind("word"), t.secondary);
    assert_eq!(t.source("lsp"), t.syn.r#type);
    assert_eq!(t.source("buf"), t.syn.string);
}

#[test]
fn selected_word_cells_keep_prefix_hierarchy_and_real_unicode_anchor() {
    let mut editor = fixture("界界response 界界result\n\t界界re\n", "G$a<c-n>");
    let mut terminal = ratatui::Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| crate::render::paint(&mut editor, frame))
        .unwrap();
    let caret = terminal.backend_mut().get_cursor_position().unwrap();
    let grid = terminal.backend().buffer();
    // Tab placement comes from the real caret; the bounded keyword has two
    // wide glyphs and two ASCII cells, not eight Unicode scalar/byte columns.
    let anchor = caret.x - 6;
    assert_eq!(grid[(anchor, caret.y + 1)].symbol(), "╭");
    let row = caret.y + 2;
    assert_eq!(grid[(anchor + 1, row)].symbol(), "▌");
    let card_right = (anchor + 1..100)
        .find(|&x| grid[(x, caret.y + 1)].symbol() == "╮")
        .unwrap();
    let mut column = anchor + 1;
    while column < card_right {
        let cell = &grid[(column, row)];
        assert_eq!(cell.bg, t().raised, "selection band at {column}");
        // A wide glyph paints its covered cell; TestBackend stores a reset
        // placeholder there, just as the canonical frame dumper accounts for.
        column += Span::raw(cell.symbol()).width().max(1) as u16;
    }
    let label = anchor + 10;
    assert_eq!(grid[(label, row)].symbol(), "界");
    assert_eq!(grid[(label + 4, row)].symbol(), "r");
    assert_eq!(grid[(label + 5, row)].symbol(), "e");
    for column in [label, label + 2, label + 4, label + 5] {
        assert_eq!(grid[(column, row)].fg, t().accent);
        assert!(grid[(column, row)].modifier.contains(Modifier::BOLD));
    }
    assert_eq!(grid[(label + 6, row)].fg, t().text);
    assert_eq!(grid[(anchor + 3, row)].fg, t().secondary); // kind
    for column in card_right - 4..card_right - 1 {
        assert_eq!(
            grid[(column, row)].fg,
            t().syn.string,
            "source tag at {column}"
        );
    }
    assert_eq!(
        editor.buf().text().to_string(),
        "界界response 界界result\n\t界界re\n"
    );
}

#[test]
fn narrow_split_and_bottom_edge_keep_card_inside_owning_pane_off_caret() {
    for (columns, rows, text, keys, above) in [
        (26, 12, "response result\nre\n", "G$a<c-n>", false),
        (100, 16, "response result\nre\n", ":vs<cr>G$a<c-n>", false),
        (
            60,
            10,
            "response result\n\n\n\n\n\n\nre\n",
            "G$a<c-n>",
            true,
        ),
    ] {
        let mut editor = fixture(text, keys);
        let mut terminal = ratatui::Terminal::new(TestBackend::new(columns, rows)).unwrap();
        terminal
            .draw(|frame| crate::render::paint(&mut editor, frame))
            .unwrap();
        let caret = terminal.backend_mut().get_cursor_position().unwrap();
        let pane = editor.prepared_view().panes[editor.active_pane()].rect;
        let grid = terminal.backend().buffer();
        let mut selected_cells = Vec::new();
        for y in 0..rows {
            for x in 0..columns {
                if grid[(x, y)].bg == t().raised {
                    selected_cells.push((x, y));
                }
            }
        }
        let &(first, selected_row) = selected_cells.first().unwrap();
        assert_eq!(grid[(first, selected_row)].symbol(), "▌");
        assert_eq!(selected_row < caret.y, above);
        for (x, y) in selected_cells {
            assert!(x >= pane.x && x < pane.x + pane.width - 1);
            assert!(y >= pane.y && y < pane.y + pane.height);
            assert_ne!(y, caret.y, "completion must leave the typed line visible");
        }
        let source = (first..pane.x + pane.width - 3)
            .find(|x| {
                grid[(*x, selected_row)].symbol() == "b"
                    && grid[(*x + 1, selected_row)].symbol() == "u"
                    && grid[(*x + 2, selected_row)].symbol() == "f"
            })
            .unwrap();
        assert_eq!(grid[(source, selected_row)].fg, t().source("buf"));
        assert_eq!(editor.buf().text().to_string(), text);
    }
}

#[test]
fn documentation_uses_shared_syntax_styles_and_never_reclaims_the_caret_row() {
    let bounds = Rect::new(0, 0, 120, 30);
    let menu = Rect::new(4, 12, 58, 8);
    assert_eq!(
        documentation_area(bounds, menu, 11),
        Some(Rect::new(62, 12, 40, 8))
    );
    assert_eq!(
        documentation_area(Rect::new(0, 0, 60, 20), Rect::new(0, 10, 58, 8), 18),
        Some(Rect::new(0, 2, 58, 8))
    );
    assert_eq!(
        documentation_area(Rect::new(0, 0, 60, 10), Rect::new(0, 0, 58, 8), 8),
        None
    );
    let mut terminal = ratatui::Terminal::new(TestBackend::new(40, 8)).unwrap();
    let spans = [strop_syntax::Span {
        start: 0,
        end: 2,
        class: strop_syntax::Class::Keyword,
        emphasis: strop_syntax::Emphasis::default(),
    }];
    terminal
        .draw(|frame| {
            render_documentation(
                &t(),
                CompletionDocumentation {
                    text: "fn response()",
                    spans: &spans,
                    truncated: false,
                },
                frame,
                Rect::new(0, 0, 40, 8),
            )
        })
        .unwrap();
    let grid = terminal.backend().buffer();
    assert_eq!(grid[(2, 1)].symbol(), "f");
    assert_eq!(grid[(2, 1)].fg, t().class(strop_syntax::Class::Keyword));
    assert_eq!(grid[(5, 1)].fg, t().text);
    assert_eq!(grid[(0, 1)].fg, t().faint, "unfocused card border");
}
