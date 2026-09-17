use super::*;

#[test]
fn cursor_line_shows_eol_diagnostic() {
    let mut e = Editor::new(Buffer::from_text("let x = 1;\n"));
    let rel = "strop-eol-diag-test.rs";
    e.fixture_buf_mut().path = Some(rel.into());
    e.fixture_insert_diagnostics(
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
fn last_text_row_and_four_digit_gutter_keep_caret_alignment() {
    let mut e = Editor::new(Buffer::from_text(&"x\n".repeat(1001)));
    e.set_head(e.buf().line_start(1000));
    let mut terminal = viewport_terminal(12, 4);
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
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
    let mut terminal = viewport_terminal(13, 6); // 9 fixed diff cells + 3 content + track
    terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    assert_eq!(e.view().hscroll.get(), 6);
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 0, 2, 13),
        ["▸", " ", " ", " ", " ", " ", " ", "1", " ", "e\u{301}", "\u{fffd}", "Z", "\u{2502}"]
    );
    // the stats band scrolls WITH the content; the number gutter does
    // not — and the reserved track column (0064 §1) closes the band
    assert_eq!(
        row_symbols(terminal.backend().buffer(), 9, 0, 4),
        [" ", "+", "1", "\u{2502}"]
    );
    terminal.backend_mut().assert_cursor_position((11, 2));
}

#[test]
fn unattached_combining_cluster_cannot_rewrite_the_gutter() {
    let e = Editor::new(Buffer::from_text("\u{301}A\n"));
    let grid = clipped_row(&e, 0, 2, &super::super::RowStyle::default());
    // a zero-width cluster occupies no cell: A stays at cell 0 and the
    // second cell is blank — the cluster can never shift the row
    assert_eq!(row_symbols(&grid, 0, 0, 2), ["A", " "]);
    assert_eq!(
        e.buf().cell_col_with_tab(2usize, e.config().tab_size).get(),
        0
    );
}
