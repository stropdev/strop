//! Nested pane chrome (plans/0104 §6, §8.3): where dividers meet title
//! rules, the junction glyph, and its ink.
use super::super::chrome::junction;
use super::*;

fn draw(editor: &mut Editor, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = viewport_terminal(width, height);
    terminal.draw(|f| crate::render::paint(editor, f)).unwrap();
    terminal.backend().buffer().clone()
}

fn scratch(keys: &str) -> Editor {
    let mut editor = Editor::new(Buffer::from_text("alpha\nbeta\n"));
    editor.feed_text(keys);
    editor
}

#[test]
fn junction_glyphs_follow_the_rules_they_meet() {
    // (left, right, up) → glyph, §6 rule 3
    assert_eq!(junction(false, false, false), "│");
    assert_eq!(junction(false, false, true), "│");
    assert_eq!(junction(true, false, false), "┬");
    assert_eq!(junction(false, true, false), "┬");
    assert_eq!(junction(true, true, false), "┬");
    assert_eq!(junction(true, true, true), "┼");
    assert_eq!(junction(false, true, true), "├");
    assert_eq!(junction(true, false, true), "┤");
}

/// §6 (a)–(c): each junction at its cell, in the divider's ink; focus
/// moves inks, never a symbol; no corners anywhere.
#[test]
fn nested_layouts_draw_their_junctions_in_the_divider_ink() {
    // the keys, the pane whose title row the junction is on, the glyph
    let cases: [(&str, usize, &str); 4] = [
        (":sp<cr>:vs<cr>", 1, "┬"),
        (":vs<cr>:sp<cr>", 2, "├"),
        (":sp<cr>:vs<cr><c-w>h:sp<cr>", 2, "┤"),
        (":sp<cr><c-w>k:vs<cr><c-w>j:vs<cr>", 2, "┼"),
    ];
    for (keys, pane, glyph) in cases {
        let mut editor = scratch(keys);
        let t = crate::render::style::Tokens::new(editor.theme());
        let grid = draw(&mut editor, 80, 24);
        let (x, y) = (39, editor.prepared_view().panes[pane].rect.y - 1);
        assert_eq!(grid[(x, y)].symbol(), glyph, "{keys}");
        assert_eq!(grid[(x, y)].fg, t.faint, "{keys}");
        assert_eq!(grid[(x, y + 1)].symbol(), "│", "{keys}");
        let before: String = grid.content().iter().map(|cell| cell.symbol()).collect();
        assert!(!before.contains(['╭', '╮', '╰', '╯']), "{keys}");
        editor.focus_pane(0);
        let after: String = draw(&mut editor, 80, 24)
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert_eq!(before, after, "{keys}: focus changes no symbol");
    }
}

/// §6 (d): a divider ending on a title rule below it draws `┴` in that
/// title's ink; where it ends over the title's words, the words win.
#[test]
fn a_divider_ends_on_the_title_below_in_its_ink() {
    let mut editor = scratch(":sp<cr>:vs<cr><c-w>k:vs<cr>:vs<cr>");
    let t = crate::render::style::Tokens::new(editor.theme());
    let grid = draw(&mut editor, 80, 24);
    // three over two: upper dividers at 26 and 53, the lower titles on row 11
    assert_eq!(grid[(26, 10)].symbol(), "│");
    assert_eq!(grid[(26, 11)].symbol(), "┴");
    assert_eq!(grid[(26, 11)].fg, grid[(25, 11)].fg, "the title's ink");
    assert_eq!(grid[(26, 11)].fg, t.faint, "pane 4 is not focused");
    // `─ 5 [scratch] ` covers 40..=53: the space after the name stays
    assert_eq!(grid[(53, 10)].symbol(), "│");
    assert_eq!(grid[(53, 11)].symbol(), " ");
    assert_eq!(grid[(39, 11)].symbol(), "┬");
}
