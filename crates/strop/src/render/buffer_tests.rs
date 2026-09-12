use crate::editor::Editor;
use strop_core::Buffer;
#[path = "buffer/matching_tests.rs"]
mod matching;

#[test]
fn cursor_line_shows_eol_diagnostic() {
    let mut e = Editor::new(Buffer::from_text("let x = 1;\n"));
    let rel = "strop-eol-diag-test.rs";
    e.buf_mut().path = Some(rel.into());
    e.diags.insert(
        e.current(),
        crate::editor::DocumentDiagnostics {
            revision: e.buf().revision(),
            items: vec![strop_lsp::ResolvedDiag {
                line: strop_core::id::LineIndex::new(0),
                col: strop_core::id::ByteColumn::new(4),
                severity: strop_lsp::Severity::Error,
                end_line: strop_core::id::LineIndex::new(0),
                end_col: strop_core::id::ByteColumn::new(8),
                message: "mismatched types".into(),
            }],
        },
    );
    let frame = crate::headless::frame_string(&mut e, 60, 10).unwrap();
    assert!(frame.contains("●"), "gutter sign: {frame}");
    assert!(frame.contains("▍ mismatched types"), "eol note: {frame}");
}

#[test]
fn switching_buffers_leaves_no_stale_cells() {
    // invariant: every pane row is written full-width, so ratatui's
    // double-buffer can never resurrect two-frames-old glyphs on a
    // buffer switch (the user-reported "lingering >" symptom class)
    let dir = tempfile::tempdir().unwrap();
    let wide = dir.path().join("wide.txt");
    let narrow = dir.path().join("narrow.txt");
    let junk = format!("{}\n", ">".repeat(60)).repeat(30);
    std::fs::write(&wide, &junk).unwrap();
    std::fs::write(&narrow, "hi\n").unwrap();
    let mut e = Editor::new(Buffer::from_text(""));
    e.open_fixture(&wide).unwrap();
    e.open_fixture(&narrow).unwrap();
    // same terminal, two frames: ratatui TestBackend diffing is the
    // real path, so drive both frames through one terminal
    let backend = ratatui::backend::TestBackend::new(40, 12);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let draw = |e: &mut Editor, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        terminal.draw(|f| crate::render::render(e, f)).unwrap();
        let buf = terminal.backend().buffer();
        (0..12)
            .map(|y| {
                (0..40)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    let wide_id = e
        .mru
        .iter()
        .copied()
        .find(|&id| {
            e.docs
                .get(id)
                .and_then(|d| d.buf.path.as_deref())
                .is_some_and(|p| p.ends_with("wide.txt"))
        })
        .unwrap();
    e.view_mut().doc = wide_id;
    let wide_frame = draw(&mut e, &mut terminal);
    assert!(
        wide_frame.iter().any(|r| r.contains('>')),
        "wide content rendered"
    );
    let narrow_id = e
        .mru
        .iter()
        .copied()
        .find(|&id| {
            e.docs
                .get(id)
                .and_then(|d| d.buf.path.as_deref())
                .is_some_and(|p| p.ends_with("narrow.txt"))
        })
        .unwrap();
    e.view_mut().doc = narrow_id; // narrow
                                  // ratatui double-buffers: stale cells surface one swap later,
                                  // on the SECOND narrow frame
    let _ = draw(&mut e, &mut terminal);
    let narrow_frame = draw(&mut e, &mut terminal);
    let leftover = narrow_frame.iter().filter(|row| row.contains('>')).count();
    assert_eq!(leftover, 0, "stale cells: {}", narrow_frame.join("\n"));
}

#[test]
fn edit_that_shortens_a_line_leaves_no_stale_cells() {
    // issue 14's regression ask: a long line, an edit through the
    // real feed path that shortens it, and the double-buffered
    // terminal keeps NO glyph past the new line end (the pad_row
    // invariant pinned end to end)
    let mut e = Editor::new(Buffer::from_text(
        "if(int a, const void*, const void*);\nshort\n",
    ));
    let backend = ratatui::backend::TestBackend::new(50, 10);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let grid = |e: &mut Editor, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        terminal.draw(|f| crate::render::render(e, f)).unwrap();
        let buf = terminal.backend().buffer();
        (0..10)
            .map(|y| {
                (0..50)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    let long = grid(&mut e, &mut terminal);
    assert!(
        long.iter().any(|r| r.contains("const void*);")),
        "long content rendered: {}",
        long[0]
    );
    // shorten the line in insert mode: append + backspaces eat the tail
    e.feed_text("A");
    for _ in 0..20 {
        e.feed_text("\x7f");
    }
    e.feed_text("<esc>");
    assert!(
        e.buf().line_text(0).trim_end().len() <= 16,
        "line shortened: {:?}",
        e.buf().line_text(0)
    );
    // ratatui double-buffers: staleness surfaces one swap later —
    // take the SECOND short frame
    let _ = grid(&mut e, &mut terminal);
    let short = grid(&mut e, &mut terminal);
    let stale = short.iter().filter(|r| r.contains("void")).count();
    assert_eq!(stale, 0, "stale tail cells: {}", short.join("\n"));
}

#[test]
fn zero_height_frames_do_not_panic() {
    // 0027 §2: a resize can deliver a 0-height area — the render
    // path saturates instead of underflowing
    let mut e = Editor::new(Buffer::from_text("x\n"));
    for rows in [0u16, 1, 2] {
        let backend = ratatui::backend::TestBackend::new(20, rows);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    }
}

#[test]
fn narrower_than_the_gutter_keeps_the_margin_fixed() {
    // R6: a 4-column pane cannot fit the 5-cell number gutter — the
    // content width collapses to zero, nothing overwrites the gutter,
    // and restoring width brings the same origin back. Cells: 40 x's
    // (0..40), a=40, b=41, Z=42; origin 37, window 37..43.
    let mut e = Editor::new(Buffer::from_text(&format!("{}abZ\n", "x".repeat(40))));
    e.set_head(42); // Z
    let mut terminal = viewport_terminal(11, 4);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    let origin = e.view().hscroll.get();
    assert_eq!(origin, 37);
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 5, 0, 6),
        ["x", "x", "x", "a", "b", "Z"],
        "Z at the caret, origin {origin}"
    );
    terminal.backend_mut().assert_cursor_position((10, 0));
    // shrink below the gutter: zero content width keeps the origin
    terminal.backend_mut().resize(4, 4);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(e.view().hscroll.get(), origin, "zero width must not scroll");
    // restore: same cells, same cursor
    terminal.backend_mut().resize(11, 4);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(e.view().hscroll.get(), origin);
    terminal.backend_mut().assert_cursor_position((10, 0));
}

#[test]
fn block_mode_highlights_the_rectangle() {
    // ctrl-v lj selects cells 0-1 on rows 0-1 — the SELECT_BG must
    // land on exactly those cells (0017: the rect, not the bytes)
    let mut e = Editor::new(Buffer::from_text("aabb\nccdd\n"));
    e.feed_text("<c-v>lj");
    let backend = ratatui::backend::TestBackend::new(30, 6);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    let buf = terminal.backend().buffer();
    let bg = |x: u16, y: u16| buf[(x, y)].bg;
    // text starts after the 5-cell gutter ("▎  1 ")
    let sel = crate::render::SELECT_BG;
    assert_eq!(bg(5, 0), sel, "block corner");
    assert_eq!(bg(6, 0), sel, "block col 2 row 0");
    assert_eq!(bg(5, 1), sel, "block row 1");
    assert_ne!(bg(7, 0), sel, "outside the rectangle");
    assert_ne!(bg(5, 2), sel, "past the rectangle's last row");
}

#[test]
fn cursor_cell_tracks_wide_chars() {
    // 0017: l through a wide char lands on the next char, and the
    // caret's display CELL tracks layout, not byte columns
    let mut e = Editor::new(Buffer::from_text("a界b\n"));
    e.feed_text("l"); // onto 界
    assert_eq!(
        e.buf().cell_col_with_tab(e.head(), e.config.tab_size).get(),
        1
    ); // 界 starts at cell 1
    e.feed_text("l"); // onto b (byte 4)
    assert_eq!(
        e.buf().cell_col_with_tab(e.head(), e.config.tab_size).get(),
        3
    ); // b at cell 3
}

// ---- 0031 R6 viewport regressions ------------------------------------
//
// Expected cells/cursors are derived from absolute tab/grapheme cell
// arithmetic (configured stops, wide=2, combining=0, control→1-cell
// replacement), not from observed passes.

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
    e.open_picker(strop_picker::Kind::Grep);
    e.picker_items_fixture(vec![strop_picker::Item {
        badge: None,
        text: "a.txt:3".into(),
        payload: strop_picker::Payload::Grep {
            path: a.clone(),
            line: 3,
            col: 1,
            match_len: 5,
            line_text: "alpha three".into(),
        },
    }]);
    e.feed(crate::editor::Key::CtrlO);
    assert_eq!(
        e.buf().name.as_deref(),
        Some("collection: grep"),
        "the modeline names the collection"
    );
    let mut terminal = viewport_terminal(50, 8);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
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
    e.open_picker(strop_picker::Kind::Grep);
    e.picker_items_fixture(vec![strop_picker::Item {
        badge: None,
        text: "a.rs:1 · fn send_request".into(),
        payload: strop_picker::Payload::Grep {
            path: a.clone(),
            line: 1,
            col: 4,
            match_len: 12,
            line_text: "fn send_request(retries: u32) -> bool {".into(),
        },
    }]);
    e.feed(crate::editor::Key::CtrlO);
    let mut terminal = viewport_terminal(60, 8);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    e.wait_analysis();
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
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

fn viewport_terminal(width: u16, height: u16) -> ratatui::Terminal<ratatui::backend::TestBackend> {
    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap()
}

fn row_symbols(grid: &ratatui::buffer::Buffer, x: u16, y: u16, width: u16) -> Vec<String> {
    (x..x + width)
        .map(|x| grid[(x, y)].symbol().to_string())
        .collect()
}

#[test]
fn long_line_tabs_unicode_and_native_caret_share_origin() {
    for (tab, origin) in [(3, 70_004), (4, 70_003)] {
        let mut e = Editor::new(Buffer::from_text(&format!(
            "{}ab\t界e\u{301}\x1bZ\n",
            "x".repeat(70_000)
        )));
        e.config.tab_size = tab;
        e.reresolve_indents(); // config changes re-resolve open documents
        e.set_head(70_010); // Z, after a CJK cluster, combining cluster and ESC
        let mut terminal = viewport_terminal(11, 4); // 5 fixed + 6 content cells
        terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
        e.wait_analysis();
        terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
        assert_eq!(e.view().hscroll.get(), origin);
        assert_eq!(
            row_symbols(terminal.backend().buffer(), 0, 0, 7),
            [" ", " ", " ", "1", " ", " ", "界"]
        );
        // TestBackend retains arbitrary old storage under wide glyphs; it is
        // not a visible cell. Assert the next visible positions, not that storage.
        assert_eq!(
            row_symbols(terminal.backend().buffer(), 8, 0, 3),
            ["e\u{301}", "\u{fffd}", "Z"]
        );
        // the tab's clipped remainder is a blank, 界 renders whole (its
        // continuation cell stays untouched), ESC is the replacement
        // glyph — and the native caret sits on Z through the SAME origin
        terminal.backend_mut().assert_cursor_position((10, 0));
        assert_eq!(e.buf().cell_col_with_tab(e.head(), tab).get(), origin + 5);
    }
}

#[test]
fn split_resize_focus_preserves_independent_origins_and_static_caret() {
    let mut e = Editor::new(Buffer::from_text("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ\n"));
    e.set_head(3);
    e.panes.push(e.view().clone());
    e.layout = crate::editor::LayoutDir::Row;
    e.active_pane = 1;
    e.set_head(20);
    let mut terminal = viewport_terminal(31, 5);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes[0].hscroll.get(), e.panes[1].hscroll.get()),
        (0, 11)
    );
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 5, 1, 10),
        ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]
    );
    // the inactive pane's saved cursor (head 3 → cell 3) paints a
    // muted block, pane-local, through its own zero origin
    assert_eq!(
        terminal.backend().buffer()[(8, 1)].bg,
        ratatui::style::Color::Rgb(0x3a, 0x3d, 0x4d)
    );
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 21, 1, 10),
        ["B", "C", "D", "E", "F", "G", "H", "I", "J", "K"]
    );
    terminal.backend_mut().assert_cursor_position((30, 1));
    e.active_pane = 0;
    e.set_head(25);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes[0].hscroll.get(), e.panes[1].hscroll.get()),
        (16, 11)
    );
    assert_eq!(
        terminal.backend().buffer()[(30, 1)].bg,
        ratatui::style::Color::Rgb(0x3a, 0x3d, 0x4d)
    );
    terminal.backend_mut().resize(21, 5);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes[0].hscroll.get(), e.panes[1].hscroll.get()),
        (21, 11)
    );
    terminal.backend_mut().assert_cursor_position((9, 1));
    e.active_pane = 1;
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes[0].hscroll.get(), e.panes[1].hscroll.get()),
        (21, 16)
    );
    terminal.backend_mut().assert_cursor_position((20, 1));
    terminal.backend_mut().resize(61, 5);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes[0].hscroll.get(), e.panes[1].hscroll.get()),
        (21, 16)
    );
    assert_eq!(terminal.backend().buffer()[(36, 1)].symbol(), "G");
    terminal.backend_mut().assert_cursor_position((40, 1));
}

#[test]
fn extra_carets_and_vertical_bounds_do_not_alias() {
    let mut e = Editor::new(Buffer::from_text("abcdefghijk\nabcdefghijk\nabcdefghijk\n"));
    e.set_head(10);
    e.view_mut().sels.set_extras([0, 8]);
    let mut terminal = viewport_terminal(10, 4);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(e.view().hscroll.get(), 6);
    assert_eq!(terminal.backend().buffer()[(7, 0)].symbol(), "i");
    assert_eq!(terminal.backend().buffer()[(7, 0)].bg, crate::render::TEXT);
    assert_ne!(terminal.backend().buffer()[(5, 0)].bg, crate::render::TEXT);
    let area = ratatui::layout::Rect::new(2, 3, 10, 2);
    let origin = strop_core::id::DisplayColumn::new(6);
    // rows above the top and columns left of the origin are absent,
    // never aliased onto row 0 / column 0
    assert_eq!(
        super::caret_position(&e, area, e.current(), 10, 1, origin),
        None
    );
    assert_eq!(
        super::caret_position(&e, area, e.current(), 34, 1, origin),
        Some((11, 4))
    );
    assert_eq!(
        super::caret_position(&e, area, e.current(), 12, 1, origin),
        None
    );
}

fn clipped_row(
    e: &Editor,
    origin: usize,
    width: u16,
    style: &super::RowStyle<'_>,
) -> ratatui::buffer::Buffer {
    let view = super::PaneView {
        doc: e.current(),
        cursor: e.head(),
        view_top: 0,
        hscroll: strop_core::id::DisplayColumn::new(origin),
        overlays: true,
    };
    let text = e.buf().text().byte_slice(0..e.buf().line_end(0));
    let spans = super::content_spans(e, &view, 0, text, style, usize::from(width));
    let mut terminal = viewport_terminal(width, 1);
    terminal
        .draw(|f| {
            f.render_widget(
                ratatui::widgets::Paragraph::new(ratatui::text::Line::from(spans))
                    .style(ratatui::style::Style::default().bg(crate::render::BASE)),
                f.area(),
            )
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn clipped_graphemes_keep_overlay_ranges_and_precedence() {
    use strop_core::{
        id::{ByteOffset, DisplayColumn},
        Range,
    };
    let mut e = Editor::new(Buffer::from_text("ab\t界e\u{301}\x1bZ\n"));
    e.config.tab_size = 4;
    e.set_head(3);
    let hits = [strop_grammar::SearchMatch {
        start: ByteOffset::new(3),
        end: ByteOffset::new(10),
    }];
    let style = super::RowStyle {
        block: Some(crate::editor::BlockRect {
            first_line: 0,
            last_line: 0,
            left_cell: DisplayColumn::new(5),
            right_cell: DisplayColumn::new(6),
        }),
        search_hits: &hits,
        preview: vec![Range::charwise(6, 9)],
        flash: Some(Range::charwise(9, 10)),
        diags: vec![(6, 9, strop_lsp::Severity::Error)],
        ..Default::default()
    };
    let grid = clipped_row(&e, 5, 4, &style);
    assert_eq!(
        row_symbols(&grid, 0, 0, 4),
        [" ", "e\u{301}", "\u{fffd}", "Z"]
    );
    // a partially clipped CJK cluster is a styled blank that still
    // carries every overlay it intersects (block, search, …)
    assert_eq!(grid[(0, 0)].bg, crate::render::SELECT_BG);
    assert_eq!(grid[(0, 0)].fg, crate::render::ACCENT);
    assert_eq!(grid[(1, 0)].bg, crate::render::PREVIEW_BG);
    assert!(grid[(1, 0)]
        .modifier
        .contains(ratatui::style::Modifier::UNDERLINED));
    assert_eq!(grid[(2, 0)].bg, crate::render::FLASH_BG);
    assert_eq!(grid[(3, 0)].bg, crate::render::BASE);
    let right = clipped_row(&e, 3, 2, &style);
    assert_eq!(row_symbols(&right, 0, 0, 2), [" ", " "]);
    assert_eq!(right[(1, 0)].bg, crate::render::SELECT_BG);
}

#[test]
fn diff_background_and_annotation_use_absolute_cells() {
    let e = Editor::new(Buffer::from_text("ab\t界e\u{301}\x1bZ\n"));
    let line = strop_git::DiffLine {
        origin: strop_git::LineOrigin::Addition,
        old_lineno: None,
        new_lineno: Some(1),
        text: b"unused".to_vec(),
        has_newline: true,
    };
    let bg = super::diff::origin_bg(line.origin).unwrap();
    let style = super::RowStyle {
        diff_line: Some(&line),
        emphasis: Some((3, 6)),
        row_bg: Some(bg),
        ..Default::default()
    };
    let grid = clipped_row(&e, 5, 6, &style);
    assert_eq!(
        row_symbols(&grid, 0, 0, 6),
        [" ", "e\u{301}", "\u{fffd}", "Z", " ", " "]
    );
    // the intra-line emphasis lands on the clipped 界 placeholder and
    // the row background pads in CELLS past the last glyph
    assert_eq!(grid[(0, 0)].bg, super::diff::ADD_STRONG_BG);
    assert_eq!(grid[(5, 0)].bg, bg);
    let e = Editor::new(Buffer::from_text("abcd\n"));
    let style = super::RowStyle {
        note: Some((
            "  ▍\t界Z".into(),
            ratatui::style::Style::default().fg(crate::render::MUTED),
        )),
        ..Default::default()
    };
    let grid = clipped_row(&e, 7, 4, &style);
    // the note starts at the line's ABSOLUTE end cell (4); its tab
    // expands from absolute stops (4→8), so scrolling keeps it aligned
    assert_eq!(row_symbols(&grid, 0, 0, 4), [" ", "界", " ", "Z"]);
    assert_eq!(grid[(3, 0)].fg, crate::render::MUTED);
}

#[test]
fn last_text_row_and_four_digit_gutter_keep_caret_alignment() {
    let mut e = Editor::new(Buffer::from_text(&"x\n".repeat(1001)));
    e.set_head(e.buf().line_start(1000));
    let mut terminal = viewport_terminal(12, 4);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 0, 2, 7),
        [" ", "1", "0", "0", "1", " ", "x"]
    );
    // the caret is allowed on the LAST text row (the old renderer hid it)
    terminal.backend_mut().assert_cursor_position((6, 2));
}

#[test]
fn typed_diff_rows_keep_fixed_numbers_and_scrolled_content() {
    let mut e = Editor::new(Buffer::from_text("scratch\n"));
    e.fixture_git_context();
    let hunk = strop_git::Hunk {
        kind: strop_git::HunkKind::Add,
        new_start: 1,
        new_count: 1,
        old_start: 0,
        old_count: 0,
        lines: vec![strop_git::DiffLine {
            origin: strop_git::LineOrigin::Addition,
            old_lineno: None,
            new_lineno: Some(1),
            text: "ab\t界e\u{301}\x1bZ".as_bytes().to_vec(),
            has_newline: true,
        }],
    };
    e.open_delta(
        "delta",
        crate::editor::PreparedDiff::new("abcdef".into(), vec![hunk]),
        None,
        None,
    );
    e.set_head(e.buf().line_start(2) + 10);
    let mut terminal = viewport_terminal(13, 6); // 9 fixed diff cells + 4 content
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    assert_eq!(e.view().hscroll.get(), 5);
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 0, 2, 13),
        ["▸", " ", " ", " ", " ", " ", " ", "1", " ", " ", "e\u{301}", "\u{fffd}", "Z"]
    );
    // the stats band scrolls WITH the content; the number gutter does not
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 9, 0, 4),
        ["f", " ", "+", "1"]
    );
    terminal.backend_mut().assert_cursor_position((12, 2));
}

#[test]
fn unattached_combining_cluster_cannot_rewrite_the_gutter() {
    let e = Editor::new(Buffer::from_text("\u{301}A\n"));
    let grid = clipped_row(&e, 0, 2, &super::RowStyle::default());
    // a zero-width cluster occupies no cell: A stays at cell 0 and the
    // second cell is blank — the cluster can never shift the row
    assert_eq!(row_symbols(&grid, 0, 0, 2), ["A", " "]);
    assert_eq!(
        e.buf().cell_col_with_tab(2usize, e.config.tab_size).get(),
        0
    );
}

#[test]
fn sidebar_emission_matches_the_inset_the_caret_uses() {
    let mut e = Editor::new(Buffer::from_text("scratch\n"));
    e.fixture_git_context();
    let hunk = strop_git::Hunk {
        kind: strop_git::HunkKind::Add,
        new_start: 1,
        new_count: 1,
        old_start: 0,
        old_count: 0,
        lines: vec![strop_git::DiffLine {
            origin: strop_git::LineOrigin::Addition,
            old_lineno: None,
            new_lineno: Some(1),
            text: b"one".to_vec(),
            has_newline: true,
        }],
    };
    let commit = crate::editor::CommitFiles {
        repo: strop_git::RepoTarget::Local {
            workdir: e.cwd.clone(),
        },
        sha: "0123456789abcdef".into(),
        files: crate::editor::PreparedFiles::new(
            "0123456789abcdef".into(),
            vec![strop_git::memory::ChangedFile {
                path: "src/verylongfilename.rs".into(),
                added: 1,
                deleted: 0,
            }],
        ),
        current: "src/verylongfilename.rs".into(),
    };
    e.open_delta(
        "delta",
        crate::editor::PreparedDiff::new("src/verylongfilename.rs".into(), vec![hunk]),
        None,
        Some(commit),
    );
    let doc = e.current();
    let projected = super::diff::left_inset(&e, doc);
    let mut terminal = viewport_terminal(50, 6);
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    let grid = terminal.backend().buffer();
    // the tree: a dim dir row beside the stats line, the current file
    // (native path, quiet marker — the sidebar is not focused) beside
    // the hunk header
    assert_eq!(row_symbols(grid, 0, 0, 5), [" ", "s", "r", "c", "/"]);
    let divider = (0..50).find(|&x| grid[(x, 0)].symbol() == "│").unwrap();
    assert_eq!(grid[(0, 1)].symbol(), "▌");
    let content = (divider + 1..50)
        .find(|&x| grid[(x, 0)].symbol() != " ")
        .unwrap();
    assert_eq!(
        row_symbols(grid, content, 0, 7),
        ["s", "r", "c", "/", "v", "e", "r"]
    );
    assert_eq!(
        projected,
        usize::from(content),
        "measured inset matches the rendered boundary"
    );
    terminal.backend_mut().assert_cursor_position((content, 0));
}

#[test]
fn markdown_semantics_reach_the_real_cell_grid() {
    use ratatui::style::Modifier;
    let mut editor = Editor::new(Buffer::from_text("# Heading\n\n**bold** and *italic*\n"));
    editor.buf_mut().path = Some(std::path::PathBuf::from("guide.md"));
    editor.analysis_fixture();
    let mut terminal = viewport_terminal(80, 10);
    terminal
        .draw(|frame| crate::render::render(&mut editor, frame))
        .unwrap();
    let grid = terminal.backend().buffer();
    assert_eq!(grid[(7, 0)].symbol(), "H");
    assert_eq!(grid[(7, 0)].fg, crate::render::ACCENT);
    assert!(grid[(7, 0)].modifier.contains(Modifier::BOLD));
    assert_eq!(grid[(7, 2)].symbol(), "b");
    assert!(grid[(7, 2)].modifier.contains(Modifier::BOLD));
    assert_eq!(grid[(19, 2)].symbol(), "i");
    assert!(grid[(19, 2)].modifier.contains(Modifier::ITALIC));
}
