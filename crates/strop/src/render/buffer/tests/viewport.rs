use super::*;

// ---- 0031 R6 viewport regressions ------------------------------------
//
// Expected cells/cursors are derived from absolute tab/grapheme cell
// arithmetic (configured stops, wide=2, combining=0, control→1-cell
// replacement), not from observed passes.

#[test]
fn narrower_than_the_gutter_keeps_the_margin_fixed() {
    // R6: a 4-column pane cannot fit the 5-cell number gutter — the
    // content width collapses to zero, nothing overwrites the gutter,
    // and restoring width brings the same origin back. Cells: 40 x's
    // (0..40), a=40, b=41, Z=42; origin 38, window 38..42 + track.
    let mut e = Editor::new(Buffer::from_text(&format!("{}abZ\n", "x".repeat(40))));
    e.set_head(42); // Z
    let mut terminal = viewport_terminal(11, 4);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    let origin = e.view().hscroll.get();
    assert_eq!(origin, 38);
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 5, 0, 6),
        ["x", "x", "a", "b", "Z", "\u{2502}"],
        "Z at the caret, origin {origin}"
    );
    terminal.backend_mut().assert_cursor_position((9, 0));
    // shrink below the gutter: zero content width keeps the origin
    terminal.backend_mut().resize(4, 4);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(e.view().hscroll.get(), origin, "zero width must not scroll");
    // restore: same cells, same cursor
    terminal.backend_mut().resize(11, 4);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(e.view().hscroll.get(), origin);
    terminal.backend_mut().assert_cursor_position((9, 0));
}

#[test]
fn block_mode_highlights_the_rectangle() {
    // ctrl-v lj selects cells 0-1 on rows 0-1 — the SELECT_BG must
    // land on exactly those cells (0017: the rect, not the bytes)
    let mut e = Editor::new(Buffer::from_text("aabb\nccdd\n"));
    e.feed_text("<c-v>lj");
    let backend = ratatui::backend::TestBackend::new(30, 6);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
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
        e.buf()
            .cell_col_with_tab(e.head(), e.config().tab_size)
            .get(),
        1
    ); // 界 starts at cell 1
    e.feed_text("l"); // onto b (byte 4)
    assert_eq!(
        e.buf()
            .cell_col_with_tab(e.head(), e.config().tab_size)
            .get(),
        3
    ); // b at cell 3
}

#[test]
fn long_line_tabs_unicode_and_native_caret_share_origin() {
    for (tab, origin) in [(3, 70_004), (4, 70_003)] {
        let mut e = Editor::new(Buffer::from_text(&format!(
            "{}ab\t界e\u{301}\x1bZ\n",
            "x".repeat(70_000)
        )));
        let mut config = e.config().clone();
        config.tab_size = tab;
        e.set_config(config);
        e.reresolve_indents(); // config changes re-resolve open documents
        e.set_head(70_010); // Z, after a CJK cluster, combining cluster and ESC
        let mut terminal = viewport_terminal(11, 4); // 5 fixed + 5 content + track
        terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
        e.wait_analysis();
        terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
        // the reserved track column (0064 §1) narrows the budget by
        // one: the same caret visibility needs one more scroll step
        assert_eq!(e.view().hscroll.get(), origin + 1);
        assert_eq!(
            row_symbols(terminal.backend().buffer(), 0, 0, 7),
            [" ", " ", " ", "1", " ", "界", "x"]
        );
        // TestBackend retains arbitrary old storage under wide glyphs; it is
        // not a visible cell. Assert the next visible positions, not that storage.
        assert_eq!(
            row_symbols(terminal.backend().buffer(), 7, 0, 4),
            ["e\u{301}", "\u{fffd}", "Z", "\u{2502}"]
        );
        // the tab's clipped remainder is a blank, 界 renders whole (its
        // continuation cell stays untouched), ESC is the replacement
        // glyph — the native caret sits on Z and the track column
        // (0064 §1) closes the pane
        terminal.backend_mut().assert_cursor_position((9, 0));
        assert_eq!(e.buf().cell_col_with_tab(e.head(), tab).get(), origin + 5);
    }
}

#[test]
fn split_resize_focus_preserves_independent_origins_and_static_caret() {
    let mut e = Editor::new(Buffer::from_text("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ\n"));
    e.set_head(3);
    e.split(true, None); // :vs of the same document keeps the whole view
    e.set_head(20);
    let mut terminal = viewport_terminal(31, 5);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes()[0].hscroll.get(), e.panes()[1].hscroll.get()),
        (0, 12)
    );
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 5, 1, 10),
        ["0", "1", "2", "3", "4", "5", "6", "7", "8", "\u{2502}"]
    );
    // the inactive pane's saved cursor (head 3 → cell 3) paints a
    // muted block, pane-local, through its own zero origin
    assert_eq!(
        terminal.backend().buffer()[(8, 1)].bg,
        ratatui::style::Color::Rgb(0x3a, 0x3d, 0x4d)
    );
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 21, 1, 10),
        ["C", "D", "E", "F", "G", "H", "I", "J", "K", "\u{2502}"]
    );
    terminal.backend_mut().assert_cursor_position((29, 1));
    // 0064 §1: pull the pane-1 caret one column inside the reserved
    // budget (a frozen-origin cursor at the old last column now clips
    // honestly rather than painting into the track).
    e.set_head(19);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    e.focus_pane(0);
    e.set_head(25);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes()[0].hscroll.get(), e.panes()[1].hscroll.get()),
        (17, 12)
    );
    assert_eq!(
        terminal.backend().buffer()[(28, 1)].bg,
        ratatui::style::Color::Rgb(0x3a, 0x3d, 0x4d)
    );
    assert_eq!(terminal.backend().buffer()[(30, 1)].symbol(), "\u{2502}");
    terminal.backend_mut().resize(21, 5);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes()[0].hscroll.get(), e.panes()[1].hscroll.get()),
        (22, 12)
    );
    terminal.backend_mut().assert_cursor_position((8, 1));
    e.focus_pane(1);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes()[0].hscroll.get(), e.panes()[1].hscroll.get()),
        (22, 16)
    );
    terminal.backend_mut().assert_cursor_position((19, 1));
    terminal.backend_mut().resize(61, 5);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(
        (e.panes()[0].hscroll.get(), e.panes()[1].hscroll.get()),
        (22, 16)
    );
    assert_eq!(terminal.backend().buffer()[(36, 1)].symbol(), "G");
    terminal.backend_mut().assert_cursor_position((39, 1));
}

#[test]
fn extra_carets_and_vertical_bounds_do_not_alias() {
    let mut e = Editor::new(Buffer::from_text("abcdefghijk\nabcdefghijk\nabcdefghijk\n"));
    e.set_head(10);
    e.fixture_sels_mut().set_extras([0, 8]);
    let mut terminal = viewport_terminal(10, 4);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(e.view().hscroll.get(), 7);
    assert_eq!(terminal.backend().buffer()[(6, 0)].symbol(), "i");
    assert_eq!(terminal.backend().buffer()[(6, 0)].bg, crate::render::TEXT);
    assert_ne!(terminal.backend().buffer()[(4, 0)].bg, crate::render::TEXT);
    let area = ratatui::layout::Rect::new(2, 3, 10, 2);
    let origin = strop_core::id::DisplayColumn::new(6);
    // rows above the top and columns left of the origin are absent,
    // never aliased onto row 0 / column 0
    assert_eq!(
        super::super::caret_position(&e, area, e.current(), 10, 1, origin),
        None
    );
    assert_eq!(
        super::super::caret_position(&e, area, e.current(), 34, 1, origin),
        Some((11, 4))
    );
    assert_eq!(
        super::super::caret_position(&e, area, e.current(), 12, 1, origin),
        None
    );
}

#[test]
fn clipped_graphemes_keep_overlay_ranges_and_precedence() {
    use strop_core::{
        id::{ByteOffset, DisplayColumn},
        Range,
    };
    let mut e = Editor::new(Buffer::from_text("ab\t界e\u{301}\x1bZ\n"));
    let mut config = e.config().clone();
    config.tab_size = 4;
    e.set_config(config);
    e.set_head(3);
    let hits = [strop_grammar::SearchMatch {
        start: ByteOffset::new(3),
        end: ByteOffset::new(10),
    }];
    let style = super::super::RowStyle {
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
    let bg = super::super::diff::origin_bg(line.origin).unwrap();
    let style = super::super::RowStyle {
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
    assert_eq!(grid[(0, 0)].bg, super::super::diff::ADD_STRONG_BG);
    assert_eq!(grid[(5, 0)].bg, bg);
    let e = Editor::new(Buffer::from_text("abcd\n"));
    let style = super::super::RowStyle {
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
            workdir: e.cwd().to_owned(),
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
    let projected = e.left_inset(doc);
    let mut terminal = viewport_terminal(50, 6);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
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
    let mut buffer = Buffer::from_text("# Heading\n\n**bold** and *italic*\n");
    buffer.path = Some(std::path::PathBuf::from("guide.md"));
    let mut editor = Editor::new(buffer);
    editor.analysis_fixture_width(79); // 0064 §1: the track reserves one column
    let mut terminal = viewport_terminal(80, 10);
    terminal
        .draw(|frame| crate::render::paint(&mut editor, frame))
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

#[test]
fn pane_scrollbar_carries_track_thumb_and_git_overview() {
    // 0064 §1: one reserved column per pane — quiet track, viewport
    // thumb, sparse Git spans; detached documents keep the honest
    // empty track.
    let text = format!("{}\n", "line\n".repeat(200));
    let mut e = Editor::new(Buffer::from_text(&text));
    let mut terminal = viewport_terminal(40, 10);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    let grid = terminal.backend().buffer();
    // The pane rows of the right column are the track (the bottom
    // row is the status line); the thumb sits at the top while the
    // viewport is at line 0, painted over the track.
    assert_eq!(grid[(39, 0)].symbol(), "\u{25ae}");
    assert!((1..9).all(|y| grid[(39, y)].symbol() == "\u{2502}"));
    // No hunks: no change spans anywhere in the track.
    assert!((0..9).all(|y| grid[(39, y)].symbol() != "\u{258e}"));

    // Seed one addition at line 150: a green span appears, mapped
    // fractionally into the track (150/200 of 9 rows ≈ row 7).
    let hunk = strop_git::Hunk {
        kind: strop_git::HunkKind::Add,
        new_start: 150,
        new_count: 1,
        old_start: 149,
        old_count: 0,
        lines: vec![strop_git::DiffLine {
            origin: strop_git::LineOrigin::Addition,
            old_lineno: None,
            new_lineno: Some(150),
            text: b"added\n".to_vec(),
            has_newline: true,
        }],
    };
    e.fixture_set_hunks(
        strop_engine::editor::git_memory::hunk_set::HunkSet::from_parts(vec![hunk], 201),
    );
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    let grid = terminal.backend().buffer();
    assert_eq!(
        grid[(39, 6)].symbol(),
        "\u{258e}",
        "the addition maps to its track row"
    );
    assert_eq!(
        grid[(39, 6)].fg,
        ratatui::style::Color::Rgb(0xa9, 0xc4, 0x7c)
    );

    // Scrolling moves the thumb down the same fractional mapping;
    // move the caret (the viewport follows it) — a bare view_top is
    // clamped back to the caret by admission.
    e.set_head(99 * 5); // start of line 100 in the 5-byte-lines fixture
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    let grid = terminal.backend().buffer();
    let thumb_row = (0..10)
        .find(|&y| grid[(39, y)].symbol() == "\u{25ae}")
        .unwrap();
    assert!(
        thumb_row >= 4,
        "half the document scrolled past: thumb at {thumb_row}"
    );
}

#[test]
fn cursor_fade_paints_the_block_then_restores_the_unfaded_cursor() {
    // 0064 §2: mid-fade the caret cell is software-painted toward the
    // block's final appearance; when the window closes the frame is
    // exactly the unfaded cursor — untouched cell, native position.
    use ratatui::backend::Backend;
    let mut e = Editor::new(Buffer::from_text("alpha\nbeta\n"));
    let mut terminal = viewport_terminal(20, 4);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    let caret = terminal.backend_mut().get_cursor_position().unwrap();
    let caret = (caret.x, caret.y);
    let unfaded = terminal.backend().buffer()[caret].clone();

    // Mid-fade (50%): the cell ramps toward the block appearance.
    e.fixture_set_cursor_fade(Some(e.tape().now()));
    let mut tick = e.tape().now();
    tick.monotonic_ms += 80;
    e.tape().set_tick(tick).unwrap();
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    eprintln!(
        "progress={:?} fade={:?} now={:?}",
        e.cursor_fade_progress(),
        e.cursor_fade(),
        e.tape().now()
    );
    let faded = &terminal.backend().buffer()[caret];
    assert_eq!(
        faded.bg,
        ratatui::style::Color::Rgb(0x7f, 0x7d, 0x7c),
        "half-faded block bg"
    );
    assert_eq!(
        faded.fg,
        ratatui::style::Color::Rgb(0x7f, 0x7d, 0x7c),
        "half-faded block fg"
    );

    // Window closed: exactly the unfaded frame, native cursor back.
    tick.monotonic_ms += 200;
    e.tape().set_tick(tick).unwrap();
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(terminal.backend().buffer()[caret], unfaded);
    terminal.backend_mut().assert_cursor_position(caret);
}
