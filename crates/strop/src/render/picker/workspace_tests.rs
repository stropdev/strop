//! The workspace pickers (find-file, symbols, grep) own one stable
//! near-full-frame layout; the file preview carries the evidence.
use crate::editor::{Editor, Key};
use ratatui::{backend::TestBackend, Terminal};
use strop_core::Buffer;

fn draw(editor: &mut Editor, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::render::frame_capture::draw(editor, frame, false))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// (right edge of the selected row's full band, the row it sits on)
fn selected_band(grid: &ratatui::buffer::Buffer, width: u16, height: u16) -> (u16, u16) {
    for y in 0..height {
        let mut edge = None;
        for x in 0..width {
            if grid[(x, y)].bg == crate::render::SELECT_BG {
                edge = Some(x);
            }
        }
        if let Some(edge) = edge {
            return (edge, y);
        }
    }
    panic!("no selected row band in frame");
}

#[test]
fn find_file_and_symbols_share_the_full_frame_workspace_with_a_wider_preview() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.txt");
    std::fs::write(&path, "preview evidence line\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.open_fixture(&path).unwrap();
    editor.open_picker(strop_picker::Kind::Files);
    editor.wait_picker();

    let grid = draw(&mut editor, 100, 30);
    // near-full-frame card: 1-cell margins, leaving the status row free
    assert_eq!(grid[(1, 0)].symbol(), "╭", "card starts at the left margin");
    assert_eq!(
        grid[(98, 0)].symbol(),
        "╮",
        "card spans to the right margin"
    );
    assert_eq!(
        grid[(1, 28)].symbol(),
        "╰",
        "card keeps the status row free"
    );
    let (band_edge, band_row) = selected_band(&grid, 100, 30);
    assert!(
        (50..=58).contains(&band_edge),
        "list column stays near 55% (edge {band_edge})"
    );
    // the preview pane follows the list (scrollbar column, then its border)
    let border_x = (band_edge + 1..70)
        .find(|x| grid[(*x, band_row)].symbol() == "│")
        .unwrap_or_else(|| panic!("no preview border right of list edge {band_edge}"));
    let title: String = (border_x + 1..98)
        .map(|x| grid[(x, band_row)].symbol())
        .collect();
    assert!(
        title.contains("source.txt"),
        "preview names its file: {title:?}"
    );
    let evidence: String = (border_x + 1..98)
        .map(|x| grid[(x, band_row + 1)].symbol())
        .collect();
    assert!(evidence.contains("preview evidence"), "{evidence:?}");

    // symbols join the same workspace; the transient card stays floating
    editor.open_picker(strop_picker::Kind::Symbols);
    editor.picker_items_fixture(vec![strop_picker::Item {
        badge: Some("fn".into()),
        text: "dispatch  container.rs · :12".into(),
        payload: strop_picker::Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path.clone()),
            line: 1,
            col: 1,
            match_len: 0,
            line_text: "dispatch".into(),
        },
    }]);
    let grid = draw(&mut editor, 100, 30);
    assert_eq!(grid[(1, 0)].symbol(), "╭");
    assert_eq!(grid[(98, 0)].symbol(), "╮");
    let (band_edge, band_row) = selected_band(&grid, 100, 30);
    assert!(
        (50..=58).contains(&band_edge),
        "symbols list edge {band_edge}"
    );
    assert!(
        (band_edge + 1..70).any(|x| grid[(x, band_row)].symbol() == "│"),
        "symbols preview follows the list"
    );

    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.open_picker(strop_picker::Kind::RemoteAddress);
    let grid = draw(&mut editor, 100, 30);
    assert_ne!(
        grid[(1, 0)].symbol(),
        "╭",
        "transient pickers stay floating"
    );
}

/// The results-list track column for the open kind, read from the same
/// layout the paint uses so both agree on the reserved column.
fn track_column(kind: strop_picker::Kind, width: u16, height: u16) -> (u16, std::ops::Range<u16>) {
    let area = ratatui::layout::Rect::new(0, 0, width, height);
    let (results, _) = super::layout::split_results(area, kind, false).expect("results area");
    (
        results.x + results.width - 1,
        results.y..results.y + results.height,
    )
}

/// One accent thumb on a quiet track; returns the thumb rows found.
fn assert_track(
    grid: &ratatui::buffer::Buffer,
    track_x: u16,
    rows: std::ops::Range<u16>,
) -> Vec<u16> {
    let mut thumbs = Vec::new();
    for y in rows {
        let cell = &grid[(track_x, y)];
        if cell.symbol() == "▮" {
            assert_eq!(
                cell.fg,
                crate::render::ACCENT,
                "thumb paints in the accent style"
            );
            thumbs.push(y);
        } else {
            assert_eq!(cell.symbol(), "│", "quiet track cell at row {y}");
            assert_eq!(
                cell.fg,
                ratatui::style::Color::Rgb(0x2a, 0x2c, 0x3a),
                "track stays quiet at row {y}"
            );
        }
    }
    assert_eq!(thumbs.len(), 1, "exactly one thumb cell in the track");
    thumbs
}

fn symbol_fixture(path: &std::path::Path, n: usize) -> strop_picker::Item {
    strop_picker::Item {
        badge: Some("fn".into()),
        text: format!("dispatch_{n}  container.rs · :{n}"),
        payload: strop_picker::Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path.to_path_buf()),
            line: 1,
            col: 1,
            match_len: 0,
            line_text: format!("fn dispatch_{n}() {{}}").into(),
        },
    }
}

#[test]
fn workspace_symbols_scrollbar_paints_track_thumb_and_moves_with_selection() {
    // 0064 §3: golden cell-grid pin for the picker scrollbar — the
    // reserved column is carved before the text budget, the quiet track
    // and accent thumb paint on a scrollable workspace list, and the
    // thumb follows the selection through the list.
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("container.rs");
    std::fs::write(&path, "fn dispatch() {}\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.open_picker(strop_picker::Kind::WorkspaceSymbols);
    editor.picker_items_fixture((0..40).map(|n| symbol_fixture(&path, n)).collect());
    let (track_x, rows) = track_column(strop_picker::Kind::WorkspaceSymbols, 100, 30);
    let row_count = editor.picker.as_ref().unwrap().picker.rows.len();
    assert!(
        row_count > rows.len(),
        "fixture list scrolls: {row_count} rows"
    );

    let grid = draw(&mut editor, 100, 30);
    // The reserved column is carved before the text budget: the padded
    // selection band ends one column left of the track.
    let (band_edge, _) = selected_band(&grid, 100, 30);
    assert_eq!(
        band_edge + 1,
        track_x,
        "the text budget stops at the reserved column"
    );
    let thumbs = assert_track(&grid, track_x, rows.clone());
    assert_eq!(
        thumbs,
        vec![rows.start],
        "selection at the first row pins the thumb to the top"
    );

    // Drive the selection through the list with the real input path;
    // the thumb follows fractionally.
    for _ in 0..20 {
        editor.feed(Key::Down);
    }
    let grid = draw(&mut editor, 100, 30);
    let thumbs = assert_track(&grid, track_x, rows.clone());
    assert!(
        thumbs[0] >= rows.start + 8,
        "selection 20/{row_count} moves the thumb down the track: {thumbs:?}"
    );
}

#[test]
fn short_lists_keep_the_reserved_column_empty_when_text_overflows() {
    // 0064 §3: the column is carved BEFORE the text budget — with no
    // scrollbar painted over it, overflowing row text must still stop
    // one column short of the results edge.
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("container.rs");
    std::fs::write(&path, "fn dispatch() {}\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.open_picker(strop_picker::Kind::WorkspaceSymbols);
    let long = "x".repeat(200);
    editor.picker_items_fixture(
        (0..3)
            .map(|n| strop_picker::Item {
                text: format!("{long}{n}"),
                ..symbol_fixture(&path, n)
            })
            .collect(),
    );
    let (track_x, rows) = track_column(strop_picker::Kind::WorkspaceSymbols, 100, 30);
    let grid = draw(&mut editor, 100, 30);
    // Each overflowing row fills its text budget exactly: the row's
    // right-aligned location sits on the last budgeted cell, and the
    // reserved column stays empty (no scrollbar paints over it).
    for (n, y) in rows.take(3).enumerate() {
        assert_eq!(
            grid[(track_x - 1, y)].symbol(),
            "1",
            "row {n}: clipped row text reaches the last budgeted cell"
        );
        assert_eq!(
            grid[(track_x, y)].symbol(),
            " ",
            "row {n}: overflowing text stops before the reserved column"
        );
    }
}

#[test]
fn floating_card_picker_uses_the_same_scrollbar_vocabulary() {
    // 0064 §3: the vocabulary is not workspace-only — the floating card
    // paints the same reserved column, quiet track and accent thumb,
    // and the thumb moves with the selection.
    let mut editor = Editor::new(Buffer::from_text("scratch\n"));
    editor.open_picker(strop_picker::Kind::Buffers);
    let scratch = editor.mru[0];
    editor.picker_items_fixture(
        (0..30)
            .map(|n| strop_picker::Item {
                badge: None,
                text: format!("buffer_{n}.rs"),
                payload: strop_picker::Payload::Buffer(scratch),
            })
            .collect(),
    );
    let (track_x, rows) = track_column(strop_picker::Kind::Buffers, 100, 30);
    let row_count = editor.picker.as_ref().unwrap().picker.rows.len();
    assert!(
        row_count > rows.len(),
        "fixture list scrolls: {row_count} rows"
    );

    let grid = draw(&mut editor, 100, 30);
    let thumbs = assert_track(&grid, track_x, rows.clone());
    assert_eq!(thumbs, vec![rows.start], "thumb starts at the top");

    for _ in 0..15 {
        editor.feed(Key::Down);
    }
    let grid = draw(&mut editor, 100, 30);
    let thumbs = assert_track(&grid, track_x, rows.clone());
    assert!(
        thumbs[0] >= rows.start + 6,
        "selection 15/{row_count} moves the thumb down the track: {thumbs:?}"
    );
}

#[test]
fn unopened_file_preview_loads_and_highlights_through_preparation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("buried.rs");
    std::fs::write(&path, "fn buried_evidence() -> u32 {\n    42\n}\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.open_picker(strop_picker::Kind::Files);
    editor.wait_picker();

    // The file was never opened as a document: preparation must admit the
    // bounded preview read (AR01 paint never admits work), the read drains,
    // and the next frame paints real content instead of `loading…`.
    let first = draw(&mut editor, 100, 30);
    assert!(
        (0..30).any(|y| (0..100).any(|x| first[(x, y)].symbol() == "l")),
        "first frame still loading"
    );
    editor.drain_picker();
    let grid = draw(&mut editor, 100, 30);
    let rendered: String = (0..30)
        .flat_map(|y| (0..100).map(move |x| (x, y)))
        .map(|(x, y)| grid[(x, y)].symbol())
        .collect();
    assert!(rendered.contains("buried_evidence"), "{rendered}");

    // The preview's syntax analysis was admitted with the same window:
    // the `fn` keyword paints a different foreground than the identifier.
    let row = (0..30)
        .find(|y| {
            (0..100)
                .map(|x| grid[(x, *y)].symbol())
                .collect::<String>()
                .contains("fn buried_evidence")
        })
        .expect("evidence row rendered");
    let mut keyword_fg = None;
    let mut name_fg = None;
    for x in 0..100 {
        let cell = &grid[(x, row)];
        let symbol = cell.symbol();
        if symbol == "f" && keyword_fg.is_none() {
            let next = &grid[(x + 1, row)];
            if next.symbol() == "n" {
                keyword_fg = Some(cell.fg);
            }
        }
        if symbol == "b" && name_fg.is_none() {
            let rest: String = (x..x + 6).map(|cx| grid[(cx, row)].symbol()).collect();
            if rest == "buried" {
                name_fg = Some(cell.fg);
            }
        }
    }
}
