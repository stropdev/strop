use crate::editor::Editor;
use strop_core::Buffer;

mod collection;
mod diagnostics;
#[path = "../matching_tests.rs"]
mod matching;
mod viewport;

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
        terminal.draw(|f| crate::render::paint(e, f)).unwrap();
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
        .mru()
        .iter()
        .copied()
        .find(|&id| {
            e.document(id)
                .and_then(|d| d.buf.path.as_deref())
                .is_some_and(|p| p.ends_with("wide.txt"))
        })
        .unwrap();
    e.switch_to(wide_id);
    let wide_frame = draw(&mut e, &mut terminal);
    assert!(
        wide_frame.iter().any(|r| r.contains('>')),
        "wide content rendered"
    );
    let narrow_id = e
        .mru()
        .iter()
        .copied()
        .find(|&id| {
            e.document(id)
                .and_then(|d| d.buf.path.as_deref())
                .is_some_and(|p| p.ends_with("narrow.txt"))
        })
        .unwrap();
    e.switch_to(narrow_id);
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
        terminal.draw(|f| crate::render::paint(e, f)).unwrap();
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
        terminal.draw(|f| crate::render::paint(&mut e, f)).unwrap();
    }
}

fn viewport_terminal(width: u16, height: u16) -> ratatui::Terminal<ratatui::backend::TestBackend> {
    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap()
}

fn row_symbols(grid: &ratatui::buffer::Buffer, x: u16, y: u16, width: u16) -> Vec<String> {
    (x..x + width)
        .map(|x| grid[(x, y)].symbol().to_string())
        .collect()
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
