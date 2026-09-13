use super::*;

// ---- 0051 §7 R09: matching-delimiter overlay -------------------------------

/// Draw, pump the match job, draw again; collect the PAIR_BG cells.
fn pair_cells(
    e: &mut Editor,
    terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
    width: u16,
    height: u16,
) -> Vec<(u16, u16)> {
    terminal.draw(|f| crate::render::render(e, f)).unwrap();
    e.wait_analysis();
    terminal.draw(|f| crate::render::render(e, f)).unwrap();
    let grid = terminal.backend().buffer();
    let mut cells = Vec::new();
    for y in 0..height {
        for x in 0..width {
            if grid[(x, y)].bg == crate::render::PAIR_BG {
                cells.push((x, y));
            }
        }
    }
    cells
}

/// The pane rect a full-frame draw gives a single pane.
fn pane_area(width: u16, height: u16) -> ratatui::layout::Rect {
    ratatui::layout::Rect::new(0, 0, width, height - 1)
}

fn cell_of(e: &Editor, area: ratatui::layout::Rect, byte: usize) -> (u16, u16) {
    crate::render::buffer::caret_position(
        e,
        area,
        e.current(),
        byte,
        0,
        strop_core::id::DisplayColumn::new(0),
    )
    .expect("byte on screen")
}

#[test]
fn matching_pair_paints_both_delimiter_cells() {
    let mut e = Editor::new(Buffer::from_text("fn main() {}\n"));
    e.buf_mut().path = Some("x.rs".into());
    e.set_head(10); // the '{'
    let mut terminal = viewport_terminal(40, 6);
    let cells = pair_cells(&mut e, &mut terminal, 40, 6);
    let area = pane_area(40, 6);
    let open = cell_of(&e, area, 10);
    let close = cell_of(&e, area, 11);
    assert!(
        cells.contains(&open) && cells.contains(&close),
        "both endpoints paint: {cells:?}"
    );
    assert_eq!(cells.len(), 2, "only the pair paints: {cells:?}");
}

#[test]
fn offscreen_partner_paints_nothing_and_scrolls_nothing() {
    let mut text = String::from("fn main() {\n");
    for _ in 0..40 {
        text.push_str("    work();\n");
    }
    text.push_str("}\n");
    let mut e = Editor::new(Buffer::from_text(&text));
    e.buf_mut().path = Some("x.rs".into());
    e.set_head(10); // the '{', mate 42 lines down
    let mut terminal = viewport_terminal(40, 8);
    let cells = pair_cells(&mut e, &mut terminal, 40, 8);
    let area = pane_area(40, 8);
    assert_eq!(
        cells,
        vec![cell_of(&e, area, 10)],
        "only the visible endpoint paints"
    );
    assert_eq!(e.panes[0].view_top, 0, "an offscreen partner never scrolls");
}

#[test]
fn unmatched_delimiter_paints_nothing() {
    let mut e = Editor::new(Buffer::from_text("fn main() {\n"));
    e.buf_mut().path = Some("x.rs".into());
    e.set_head(10); // the '{', no mate (incomplete code)
    let mut terminal = viewport_terminal(40, 6);
    let cells = pair_cells(&mut e, &mut terminal, 40, 6);
    assert!(cells.is_empty(), "unmatched paints nothing: {cells:?}");
}

#[test]
fn moving_the_caret_off_leaves_no_stale_highlight() {
    let mut e = Editor::new(Buffer::from_text("fn main() {}\n"));
    e.buf_mut().path = Some("x.rs".into());
    e.set_head(10);
    let mut terminal = viewport_terminal(40, 6);
    assert_eq!(pair_cells(&mut e, &mut terminal, 40, 6).len(), 2);
    // caret onto a plain identifier: the very next frame paints nothing
    e.set_head(0);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    let grid = terminal.backend().buffer();
    let stale = (0..6)
        .flat_map(|y| (0..40).map(move |x| (x, y)))
        .filter(|&(x, y)| grid[(x, y)].bg == crate::render::PAIR_BG)
        .count();
    assert_eq!(stale, 0, "no stale pair cells");
}

#[test]
fn insert_mode_highlights_the_just_typed_delimiter() {
    let mut e = Editor::new(Buffer::from_text("x)\n"));
    e.feed_text("i("); // "(x)", caret past the '('
    assert_eq!(e.mode, crate::editor::Mode::Insert);
    let mut terminal = viewport_terminal(40, 6);
    let cells = pair_cells(&mut e, &mut terminal, 40, 6);
    let area = pane_area(40, 6);
    let typed = cell_of(&e, area, 0);
    let mate = cell_of(&e, area, 2);
    assert!(
        cells.contains(&typed) && cells.contains(&mate),
        "the just-typed pair paints in Insert: {cells:?}"
    );
}

#[test]
fn cpp_angle_brackets_never_paint() {
    let mut e = Editor::new(Buffer::from_text("std::vector<int> v;\n"));
    e.buf_mut().path = Some("x.cpp".into());
    e.set_head(12); // the '<'
    let mut terminal = viewport_terminal(40, 6);
    let cells = pair_cells(&mut e, &mut terminal, 40, 6);
    assert!(
        cells.is_empty(),
        "angle brackets are not delimiters: {cells:?}"
    );
}

#[test]
fn collection_pairing_paints_the_same_sources_excerpts_only() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "fn main() {\n    body\n}\n").unwrap();
    std::fs::write(
        &b,
        "if (x) {\n    body\n    more\n    hidden\n    still_hidden\n}\n",
    )
    .unwrap();
    let mut e = Editor::new(Buffer::from_text("scratch\n"));
    e.open_fixture(&a).unwrap();
    e.open_fixture(&b).unwrap();
    e.open_picker(strop_picker::Kind::Search);
    let item = |path: &std::path::Path, line: usize, text: &str| strop_picker::Item {
        badge: None,
        text: text.into(),
        payload: strop_picker::Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path.to_path_buf()),
            line,
            col: 1,
            match_len: 2,
            line_text: text.into(),
        },
    };
    e.picker_items_fixture(vec![
        item(&a, 1, "fn main() {"),
        item(&a, 3, "}"),
        item(&b, 1, "if (x) {"),
    ]);
    e.feed(crate::editor::Key::CtrlO);
    // view rows: title, card top, body("fn main() {"), gap, body("}"), …
    let probe = e.buf().line_start(2) + 10; // a's '{'
    let mate = e.buf().line_start(4); // a's '}'
    e.set_head(probe);
    let mut terminal = viewport_terminal(50, 10);
    let cells = pair_cells(&mut e, &mut terminal, 50, 10);
    let area = pane_area(50, 10);
    assert!(
        cells.contains(&cell_of(&e, area, probe)) && cells.contains(&cell_of(&e, area, mate)),
        "both excerpts of the same source paint: {cells:?}"
    );
    assert_eq!(cells.len(), 2, "nothing else paints: {cells:?}");

    // b's '{' pairs with a '}' no excerpt shows; the only other visible
    // '}' is a's — painting it would pair across sources
    let b_probe = e.buf().line_start(7) + 7;
    e.set_head(b_probe);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    e.wait_analysis();
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    let grid = terminal.backend().buffer();
    let cells: Vec<(u16, u16)> = (0..10)
        .flat_map(|y| (0..50).map(move |x| (x, y)))
        .filter(|&(x, y)| grid[(x, y)].bg == crate::render::PAIR_BG)
        .collect();
    assert_eq!(
        cells,
        vec![cell_of(&e, area, b_probe)],
        "the unexcerpted mate paints nothing — never another source's row"
    );
}
