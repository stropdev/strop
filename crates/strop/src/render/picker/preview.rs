//! The picker's source preview: windowed, syntax-spanned, header first
//! (0050: filename:line identifies before parent/root details).
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::editor::{Editor, PreviewSource};

use super::super::{syntax_style, ACCENT, BASE, MUTED, SELECT_BG, TEXT};

const SURFACE: Color = Color::Rgb(0x20, 0x22, 0x2e);
pub(super) fn render_preview(editor: &Editor, frame: &mut Frame, area: Rect) {
    let Some((title, focus_line, source)) = editor.picker_preview_cached() else {
        frame.render_widget(Paragraph::new("").style(Style::default().bg(BASE)), area);
        return;
    };
    let visible = area.height.saturating_sub(1) as usize;
    let width = usize::from(area.width.saturating_sub(1));
    let matched = editor.picker_preview_range_cached(&source);

    let lines: Vec<Line> = match source {
        PreviewSource::Buffer(document) => {
            // A cached preview source can outlive its document (the
            // picker streams while buffers close): render nothing for a
            // stale id instead of panicking mid-paint (0056 AR02).
            let Some(doc) = editor.document(document) else {
                return;
            };
            let tab = doc.indent.width;
            let rope = doc.buf.snapshot();
            if let Err(message) = matched {
                vec![Line::from(message)]
            } else {
                let window = preview_window(&rope, focus_line, visible);
                let analysis = editor.document_analysis_cached(
                    document,
                    rope.line_to_byte(window.start),
                    rope.line_to_byte(window.end),
                    0,
                    width,
                );
                highlight_lines_owned(
                    &rope,
                    analysis.as_ref().map(|a| a.spans.as_slice()),
                    focus_line,
                    window,
                    width,
                    tab,
                    matched.ok().flatten(),
                )
            }
        }
        PreviewSource::Cached(path) => {
            // 0051 R08: a cached path's hard-tab display resolves like
            // its buffer's would — the open document's setting when one
            // matches, else the configured fallback; never a second source.
            let tab = editor.tab_width_for_location(&path);
            let Some(entry) = editor.previews().get(&path) else {
                return;
            };
            let rope = entry.rope.clone();
            if let Err(message) = matched {
                vec![Line::from(message)]
            } else {
                let window = preview_window(&rope, focus_line, visible);
                let analysis = editor.preview_analysis_cached(
                    &path,
                    rope.line_to_byte(window.start),
                    rope.line_to_byte(window.end),
                    width,
                );
                highlight_lines_owned(
                    &rope,
                    analysis.as_ref().map(|a| a.spans.as_slice()),
                    focus_line,
                    window,
                    width,
                    tab,
                    matched.ok().flatten(),
                )
            }
        }
        PreviewSource::Loading => vec![Line::from(Span::styled(
            " loading…",
            Style::default().fg(MUTED),
        ))],
        PreviewSource::Failed(error) => vec![Line::from(format!("preview: {error}"))],
        PreviewSource::Cancelled(reason) => {
            vec![Line::from(format!("preview cancelled: {reason:?}"))]
        }
    };

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(Color::Rgb(0x3a, 0x3d, 0x4d)))
        .style(Style::default().bg(SURFACE))
        .title(Span::styled(
            format!(
                " {} ",
                super::super::text::clip_end(&title, area.width.saturating_sub(2) as usize)
            ),
            Style::default().fg(MUTED),
        ));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

use strop_engine::editor::prepare::{preview_source_lines, preview_window};

fn highlight_lines_owned(
    rope: &ropey::Rope,
    spans: Option<&[strop_syntax::Span]>,
    focus_line: Option<usize>,
    window: std::ops::Range<usize>,
    width: usize,
    tab: usize,
    matched: Option<strop_core::Range>,
) -> Vec<Line<'static>> {
    use strop_core::layout::{clip, printable_grapheme, RopeGraphemes};
    let spans = spans.unwrap_or_default();
    let mut out = Vec::with_capacity(window.len());
    let digits = preview_source_lines(rope).ilog10() as usize + 1;
    let gutter = if width > digits + 5 {
        digits + 5
    } else if width > digits + 2 {
        digits + 2
    } else {
        usize::from(width > 1)
    };
    let code_width = width.saturating_sub(gutter);
    for line in window {
        let start = rope.line_to_byte(line);
        let text = rope.line(line);
        let mut end = text.len_bytes();
        while end > 0 && matches!(text.byte(end - 1), b'\r' | b'\n') {
            end -= 1;
        }
        let text = text.byte_slice(..end);
        let first_span = spans.partition_point(|span| span.end <= start);
        let focused = focus_line == Some(line + 1);
        let mut spans_out = Vec::new();
        let number_style = Style::default().fg(if focused { TEXT } else { MUTED });
        let number_style = if focused {
            number_style.add_modifier(ratatui::style::Modifier::BOLD)
        } else {
            number_style
        };
        if gutter > 0 {
            let mut marker = Style::default().fg(ACCENT);
            if focused {
                marker = marker
                    .add_modifier(ratatui::style::Modifier::BOLD)
                    .bg(SELECT_BG);
            }
            spans_out.push(Span::styled(if focused { "▶" } else { " " }, marker));
        }
        if gutter == digits + 5 {
            spans_out.push(Span::styled(
                format!(" {:>digits$}", line + 1),
                number_style,
            ));
            spans_out.push(Span::styled(" │ ", Style::default().fg(MUTED)));
        } else if gutter == digits + 2 {
            spans_out.push(Span::styled(
                format!("{:>digits$} ", line + 1),
                number_style,
            ));
        }
        for (placement, grapheme) in RopeGraphemes::new(text, tab) {
            if placement.cell.get() >= code_width {
                break;
            }
            let Some(visible) = clip(placement, strop_core::id::DisplayColumn::new(0), code_width)
            else {
                continue;
            };
            let pos = start + placement.byte;
            let mut style = Style::default().fg(TEXT).bg(SURFACE);
            if let Some(span) = spans[first_span..]
                .iter()
                .take_while(|span| span.start <= pos)
                .filter(|span| pos < span.end)
                .min_by_key(|span| span.end - span.start)
            {
                style = style.patch(syntax_style(span));
            }
            if focus_line == Some(line + 1) {
                style = style.bg(SELECT_BG);
            }
            if matched.is_some_and(|range| range.start.get() <= pos && pos < range.end.get()) {
                style = style
                    .fg(ACCENT)
                    .add_modifier(ratatui::style::Modifier::BOLD);
            }
            let symbol = if !visible.complete || grapheme == "\t" {
                " ".repeat(visible.width)
            } else {
                printable_grapheme(&grapheme).to_owned()
            };
            spans_out.push(Span::styled(symbol, style));
        }
        out.push(Line::from(spans_out).style(Style::default().bg(if focused {
            SELECT_BG
        } else {
            SURFACE
        })));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    #[test]
    fn file_and_search_previews_number_source_lines_and_mark_only_the_hit() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.txt");
        let text = format!("{}needle\nlast\n", "plain\n".repeat(99));
        std::fs::write(&path, text).unwrap();
        for search in [false, true] {
            let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
            editor.open_fixture(&path).unwrap();
            if search {
                editor.open_search(false);
                editor.paste_bracketed("needle");
            } else {
                editor.open_picker(strop_picker::Kind::Files);
            }
            editor.wait_picker();
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 8)).unwrap();
            terminal
                .draw(|frame| render_preview(&editor, frame, frame.area()))
                .unwrap();
            let grid = terminal.backend().buffer();
            let lines: Vec<String> = (0..8)
                .map(|y| (0..40).map(|x| grid[(x, y)].symbol()).collect())
                .collect();
            if search {
                assert!(
                    lines.iter().any(|line| line.contains("▶ 100 │ needle")),
                    "{lines:?}"
                );
                assert_eq!(lines.iter().filter(|line| line.contains('▶')).count(), 1);
                assert!(
                    lines.iter().any(|line| line.contains("  99 │ plain")),
                    "{lines:?}"
                );
            } else {
                assert!(
                    lines.iter().any(|line| line.contains("   1 │ plain")),
                    "{lines:?}"
                );
                assert!(lines.iter().all(|line| !line.contains('▶')));
            }
        }
    }

    #[test]
    fn workspace_symbols_preview_points_at_the_declaration_line() {
        // 0063 §2: the workspace tier shares the symbols pane shape —
        // numbered gutter, one ▶ marker on the declaration line.
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("lib.rs");
        let text = format!("{}fn dispatch() {{}}\n", "filler\n".repeat(41));
        std::fs::write(&path, text).unwrap();
        let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
        editor.open_fixture(&path).unwrap();
        editor.open_picker(strop_picker::Kind::WorkspaceSymbols);
        editor.picker_items_fixture(vec![strop_picker::Item {
            badge: Some("fn".into()),
            text: "dispatch  lib.rs · :42".into(),
            payload: strop_picker::Payload::Grep {
                location: strop_workspace::ResourceLocation::local(path.clone()),
                line: 42,
                col: 4,
                match_len: 8,
                line_text: "fn dispatch() {}".into(),
            },
        }]);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| crate::render::paint(&mut editor, frame))
            .unwrap();
        let grid = terminal.backend().buffer();
        let right: Vec<String> = (0..30)
            .map(|y| (56..100).map(|x| grid[(x, y)].symbol()).collect())
            .collect();
        assert!(
            right.iter().any(|line| line.contains("▶ 42 │")),
            "declaration marker on line 42: {right:?}"
        );
    }

    #[test]
    fn symbols_preview_points_at_the_symbol_line_in_the_full_screen_workspace() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mod.rs");
        let text = format!("{}fn dispatch() {{}}\n", "filler\n".repeat(41));
        std::fs::write(&path, text).unwrap();
        let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
        editor.open_fixture(&path).unwrap();
        editor.open_picker(strop_picker::Kind::Symbols);
        editor.picker_items_fixture(vec![strop_picker::Item {
            badge: Some("fn".into()),
            text: "dispatch  mod.rs · :42".into(),
            payload: strop_picker::Payload::Grep {
                location: strop_workspace::ResourceLocation::local(path.clone()),
                line: 42,
                col: 4,
                match_len: 8,
                line_text: "fn dispatch() {}".into(),
            },
        }]);
        // The real full-screen picker workspace: the wider preview pane must
        // carry the numbered gutter, the ▶ marker on the symbol's line and
        // the divider — exactly the rootle-style shape, now at 45% width.
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| crate::render::paint(&mut editor, frame))
            .unwrap();
        let grid = terminal.backend().buffer();
        let right: Vec<String> = (0..30)
            .map(|y| (56..100).map(|x| grid[(x, y)].symbol()).collect())
            .collect();
        assert!(
            right.iter().any(|line| line.contains("▶ 42 │")),
            "symbol marker on line 42: {right:?}"
        );
        assert_eq!(
            right.iter().filter(|line| line.contains('▶')).count(),
            1,
            "exactly one marker"
        );
        assert!(
            right.iter().any(|line| line.contains("│ fn dispatch")),
            "divider then content: {right:?}"
        );
        let mut marker = None;
        for x in 56u16..100 {
            for y in 0..30u16 {
                if grid[(x, y)].symbol() == "▶" {
                    marker = Some((x, y));
                }
            }
        }
        let (marker_x, marker_y) = marker.unwrap();
        let marker = &grid[(marker_x, marker_y)];
        assert_eq!(marker.fg, crate::render::ACCENT);
        assert!(marker.modifier.contains(ratatui::style::Modifier::BOLD));
        assert_eq!(marker.bg, crate::render::SELECT_BG);
    }

    #[test]
    fn gutters_remain_cell_bounded_and_do_not_number_a_phantom_eof() {
        let rope = ropey::Rope::from_str("\t界e\u{301}\n");
        for width in 0..16 {
            let window = preview_window(&rope, Some(1), 10);
            assert_eq!(window, 0..1);
            let lines = highlight_lines_owned(&rope, None, Some(1), window, width, 4, None);
            assert_eq!(lines.len(), 1);
            assert!(lines[0].width() <= width);
        }
    }
}
