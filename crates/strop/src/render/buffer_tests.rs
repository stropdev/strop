use crate::editor::Editor;
use strop_core::Buffer;

#[test]
fn cursor_line_shows_eol_diagnostic() {
    let mut e = Editor::new(Buffer::from_text("let x = 1;\n"));
    let rel = "strop-eol-diag-test.rs";
    e.buf_mut().path = Some(rel.into());
    let abs = e.cwd.join(rel);
    e.diags.insert(
        abs,
        vec![strop_lsp::Diag {
            line: 0,
            col: 4,
            severity: 1,
            end_line: 0,
            end_col: 8,
            message: "mismatched types".into(),
        }],
    );
    let frame = crate::headless::frame_string(&mut e, 60, 10);
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
    e.open_buffer(&wide).unwrap();
    e.open_buffer(&narrow).unwrap();
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
            e.doc(id)
                .buf
                .path
                .as_deref()
                .is_some_and(|p| p.ends_with("wide.txt"))
        })
        .unwrap();
    e.view_mut().doc = wide_id;
    let wide_frame = draw(&mut e, &mut terminal);
    assert!(
        wide_frame[0].contains(">>>"),
        "wide buffer rendered: {}",
        wide_frame[0]
    );
    let narrow_id = e
        .mru
        .iter()
        .copied()
        .find(|&id| {
            e.doc(id)
                .buf
                .path
                .as_deref()
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
    assert_eq!(e.buf().cell_col_of(e.head()), 1); // 界 starts at cell 1
    e.feed_text("l"); // onto b (byte 4)
    assert_eq!(e.buf().cell_col_of(e.head()), 3); // b at cell 3
}
