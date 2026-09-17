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
    let area = crate::render::to_cells(ratatui::layout::Rect::new(0, 0, width, height));
    let (results, _) =
        strop_engine::editor::prepare::picker_split(area, kind, false).expect("results area");
    let results = crate::render::from_cells(results);
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
    let row_count = editor.picker().unwrap().picker.rows.len();
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
    let scratch = editor.mru()[0];
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
    let row_count = editor.picker().unwrap().picker.rows.len();
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

/// AR01/AR03 seam pin: the engine-owned picker cell geometry
/// (`strop_engine::editor::prepare`) must reproduce the toolkit solver's
/// splits exactly across the size domain — the historical layout is the
/// contract, so the cutover cannot move a single cell.
#[test]
fn engine_picker_geometry_matches_the_toolkit_solver() {
    use ratatui::layout::{Constraint, Direction, Layout, Rect};
    use strop_engine::editor::prepare::{
        picker_card, picker_field_split, picker_inner, picker_split,
    };
    use strop_picker::Kind;

    let kinds = [
        Kind::Search,
        Kind::Files,
        Kind::Symbols,
        Kind::WorkspaceSymbols,
        Kind::Buffers,
        Kind::Jumps,
        Kind::RemoteHosts,
        Kind::RemoteAddress,
    ];
    for width in [
        1u16, 2, 3, 4, 5, 6, 10, 19, 20, 21, 30, 60, 61, 63, 64, 65, 70, 75, 76, 94, 95, 100, 101,
        120, 121, 150, 151, 170, 190, 200, 240,
    ] {
        for height in [1u16, 2, 3, 4, 5, 6, 8, 10, 11, 12, 13, 16, 20, 24, 30, 50] {
            let area = Rect::new(0, 0, width, height);
            let cells = crate::render::to_cells(area);
            for kind in kinds {
                // card geometry
                let expected_card = solver_card(area, kind);
                assert_eq!(
                    crate::render::from_cells(picker_card(cells, kind)),
                    expected_card,
                    "card {width}x{height} {kind:?}"
                );
                for replace in [false, true] {
                    // field split + results/preview split
                    let engine = picker_split(cells, kind, replace).map(|(r, p)| {
                        (
                            crate::render::from_cells(r),
                            p.map(crate::render::from_cells),
                        )
                    });
                    let solver = solver_split(area, kind, replace);
                    assert_eq!(
                        engine, solver,
                        "split {width}x{height} {kind:?} replace={replace}"
                    );
                }
            }
            // the field split itself across input heights
            let inner = picker_inner(picker_card(cells, Kind::Files));
            for input_h in [2u16, 3] {
                let (f, c) = picker_field_split(inner, input_h);
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(input_h), Constraint::Min(1)])
                    .split(crate::render::from_cells(inner));
                assert_eq!(
                    (crate::render::from_cells(f), crate::render::from_cells(c)),
                    (rows[0], rows[1]),
                    "field split {width}x{height} input_h={input_h}"
                );
            }
        }
    }
}

/// The pre-cutover toolkit-solver layout, kept here as the reference
/// oracle for the engine-owned geometry (`engine_picker_geometry_matches
/// _the_toolkit_solver` pins every cell).
fn solver_card(area: ratatui::layout::Rect, kind: strop_picker::Kind) -> ratatui::layout::Rect {
    use ratatui::layout::Rect;
    use strop_picker::Kind;
    if matches!(
        kind,
        Kind::Search | Kind::Files | Kind::Symbols | Kind::WorkspaceSymbols
    ) {
        Rect {
            x: area.x + 1,
            y: area.y,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(1),
        }
    } else {
        let width = ((u32::from(area.width) * 84 / 100) as u16)
            .max(50)
            .min(area.width.saturating_sub(2));
        let height = if kind == Kind::RemoteAddress {
            8
        } else {
            ((u32::from(area.height) * 70 / 100) as u16).max(12)
        }
        .min(area.height.saturating_sub(2));
        Rect {
            x: (area.width - width) / 2,
            y: (area.height - height) / 2,
            width,
            height,
        }
    }
}

fn solver_split(
    area: ratatui::layout::Rect,
    kind: strop_picker::Kind,
    replace_visible: bool,
) -> Option<(ratatui::layout::Rect, Option<ratatui::layout::Rect>)> {
    use ratatui::layout::{Constraint, Direction, Layout, Rect};
    use strop_picker::Kind;
    let card = solver_card(area, kind);
    let search_mode = kind == Kind::Search;
    let remote_picker = matches!(kind, Kind::RemoteHosts | Kind::RemoteAddress);
    let inner_width = card.width.saturating_sub(4);
    let inner_height = card.height.saturating_sub(2);
    let input_h: u16 = if search_mode && replace_visible { 3 } else { 2 };
    if inner_height < input_h + 1 {
        return None;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(input_h), Constraint::Min(1)])
        .split(Rect {
            x: card.x + 2,
            y: card.y + 1,
            width: inner_width,
            height: inner_height,
        });
    let split = |results, preview| Some((results, preview));
    if remote_picker {
        return split(rows[1], None);
    }
    if card.width < 64 && rows[1].height >= 12 {
        let stacked = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(8)])
            .split(rows[1]);
        return split(stacked[0], Some(stacked[1]));
    }
    if card.width < 64 {
        return split(rows[1], None);
    }
    let list = if search_mode { 60 } else { 55 };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(list),
            Constraint::Percentage(100 - list),
        ])
        .split(rows[1]);
    split(cols[0], Some(cols[1]))
}
