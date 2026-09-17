use super::*;

#[test]
fn collection_view_gutters_source_line_numbers() {
    // 0049 §6: body rows gutter their SOURCE line numbers; the title
    // and file-header rows keep the gutter blank; the modeline names
    // the collection, never [scratch].
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    std::fs::write(&a, "alpha one\nalpha two\nalpha three\n").unwrap();
    let mut e = Editor::new(Buffer::from_text("scratch\n"));
    e.open_fixture(&a).unwrap();
    e.open_picker(strop_picker::Kind::Search);
    e.picker_items_fixture(vec![strop_picker::Item {
        badge: None,
        text: "a.txt:3".into(),
        payload: strop_picker::Payload::Grep {
            location: strop_workspace::ResourceLocation::local(a.clone()),
            line: 3,
            col: 1,
            match_len: 5,
            line_text: "alpha three".into(),
        },
    }]);
    e.feed(crate::editor::Key::CtrlO);
    let mut terminal = viewport_terminal(50, 8);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    let grid = terminal.backend().buffer();
    // the gutter's number cell (after the sign bar) is the first cells
    // of the row: blank on chrome rows, the source line on body rows
    let gutter1: String = row_symbols(grid, 0, 1, 4).concat();
    let source_row =
        e.buf()
            .line_of(e.buf().text().to_string().find("alpha three").unwrap()) as u16;
    let gutter2: String = row_symbols(grid, 0, source_row, 4).concat();
    assert!(
        gutter1.trim().is_empty(),
        "header row has no line number: {gutter1:?}"
    );
    assert!(
        gutter2.trim().ends_with('3'),
        "body rows gutter the source line: {gutter2:?}"
    );
    let row2: String = row_symbols(grid, 0, source_row, 20).concat();
    assert!(row2.contains("alpha three"), "{row2:?}");
}

#[test]
fn collection_bodies_project_syntax_and_paint_hits() {
    // 0049 §6 + 0050 §7 cell-level: a keyword in an excerpt body carries
    // its source language's syntax color, and the query's match cells
    // carry the accent.
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.rs");
    std::fs::write(
        &a,
        "fn send_request(retries: u32) -> bool {\n    retries > 0\n}\n",
    )
    .unwrap();
    let mut e = Editor::new(Buffer::from_text("scratch\n"));
    e.open_fixture(&a).unwrap();
    e.open_picker(strop_picker::Kind::Search);
    e.picker_items_fixture(vec![strop_picker::Item {
        badge: None,
        text: "a.rs:1 · fn send_request".into(),
        payload: strop_picker::Payload::Grep {
            location: strop_workspace::ResourceLocation::local(a.clone()),
            line: 1,
            col: 4,
            match_len: 12,
            line_text: "fn send_request(retries: u32) -> bool {".into(),
        },
    }]);
    e.feed(crate::editor::Key::CtrlO);
    let mut terminal = viewport_terminal(60, 8);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    e.wait_analysis();
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    let grid = terminal.backend().buffer();
    // body row 2: "fn send_request…" — find the "fn" and the match
    let row2: String = row_symbols(grid, 0, 2, 40).concat();
    let fn_at = row2.find("fn").unwrap();
    let kw = &grid[(fn_at as u16, 2)];
    assert_ne!(
        kw.fg,
        crate::render::TEXT,
        "the keyword carries a syntax color, not plain text"
    );
    let hit_at = row2.find("send_request").unwrap();
    let hit = &grid[(hit_at as u16, 2)];
    assert_eq!(
        hit.fg,
        crate::render::ACCENT,
        "the query match paints amber in the excerpt"
    );
}
