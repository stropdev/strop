//! The picker's source preview: windowed, syntax-spanned, header first
//! (0050: filename:line identifies before parent/root details). One
//! faint rule separates it from the list; the header is the
//! workspace-relative path (0088 §2).
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::editor::{Editor, PreviewSource};

use super::super::file_name::{self, Role};
use super::super::style::Tokens;

pub(super) fn render_preview(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect) {
    let Some((title, focus_line, source)) = editor.picker_preview_cached() else {
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(t.surface)),
            area,
        );
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
                    t,
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
                    t,
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
            Style::default().fg(t.muted),
        ))],
        PreviewSource::Failed(error) => vec![Line::from(format!("preview: {error}"))],
        PreviewSource::Cancelled(reason) => {
            vec![Line::from(format!("preview cancelled: {reason:?}"))]
        }
    };

    // the authority stays whole and the path clips from its start, so
    // the name survives (0088 §2 "File names")
    let label = file_name::Label {
        authority: title.authority,
        path: relative_path(editor, &title.path),
    };
    let line = focus_line
        .map(|line| format!(":{line}"))
        .unwrap_or_default();
    let budget = usize::from(area.width.saturating_sub(2)).saturating_sub(line.len());
    let mut header = vec![Span::raw(" ")];
    header.extend(label.clipped(budget.max(4)).spans(Role::Heading, t));
    header.push(Span::styled(
        format!("{line} "),
        Style::default().fg(t.secondary),
    ));
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(t.faint))
        .style(Style::default().bg(t.surface))
        .title(Line::from(header));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The header names the file relative to the workspace: an absolute
/// path under the editor's root loses the root prefix, anything else
/// (outside paths, document labels) keeps its full form.
fn relative_path(editor: &Editor, path: &str) -> String {
    let root = editor.cwd().to_string_lossy();
    let root = root.trim_end_matches('/');
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .map_or_else(|| path.to_owned(), str::to_owned)
}

use strop_engine::editor::prepare::{preview_source_lines, preview_window};

#[allow(clippy::too_many_arguments)]
fn highlight_lines_owned(
    t: &Tokens,
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
        let row_bg = if focused { t.raised } else { t.surface };
        let mut spans_out = Vec::new();
        let number_style = Style::default().fg(if focused { t.text } else { t.muted });
        let number_style = if focused {
            number_style.add_modifier(Modifier::BOLD)
        } else {
            number_style
        };
        if gutter > 0 {
            // the focused line wears the one selected-row idiom (rule 5)
            spans_out.push(super::super::card::row_marker(focused, t));
        }
        if gutter == digits + 5 {
            spans_out.push(Span::styled(
                format!(" {:>digits$}", line + 1),
                number_style,
            ));
            spans_out.push(Span::styled(" │ ", Style::default().fg(t.faint)));
        } else if gutter == digits + 2 {
            spans_out.push(Span::styled(
                format!("{:>digits$} ", line + 1),
                number_style,
            ));
        }
        // style runs, not a span per grapheme (A2): consecutive cells with
        // one style share one span
        let mut run = String::new();
        let mut run_style: Option<Style> = None;
        for (placement, grapheme) in RopeGraphemes::new(text, tab) {
            if placement.cell.get() >= code_width {
                break;
            }
            let Some(visible) = clip(placement, strop_core::id::DisplayColumn::new(0), code_width)
            else {
                continue;
            };
            let pos = start + placement.byte;
            let mut style = Style::default().fg(t.text).bg(row_bg);
            if let Some(span) = spans[first_span..]
                .iter()
                .take_while(|span| span.start <= pos)
                .filter(|span| pos < span.end)
                .min_by_key(|span| span.end - span.start)
            {
                style = style.patch(t.syntax(span));
            }
            if matched.is_some_and(|range| range.start.get() <= pos && pos < range.end.get()) {
                style = style.fg(t.accent).add_modifier(Modifier::BOLD);
            }
            if run_style.is_some_and(|current| current != style) {
                spans_out.push(Span::styled(
                    std::mem::take(&mut run),
                    run_style.take().unwrap_or_default(),
                ));
            }
            run_style = Some(style);
            if !visible.complete || grapheme == "\t" {
                run.extend(std::iter::repeat_n(' ', visible.width));
            } else {
                run.push_str(printable_grapheme(&grapheme));
            }
        }
        if let Some(style) = run_style {
            spans_out.push(Span::styled(run, style));
        }
        out.push(Line::from(spans_out).style(Style::default().bg(row_bg)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    fn tokens() -> Tokens {
        Tokens::new(strop_core::theme::ThemeId::Hone)
    }

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
                .draw(|frame| render_preview(&editor, &tokens(), frame, frame.area()))
                .unwrap();
            let grid = terminal.backend().buffer();
            let lines: Vec<String> = (0..8)
                .map(|y| (0..40).map(|x| grid[(x, y)].symbol()).collect())
                .collect();
            if search {
                assert!(
                    lines.iter().any(|line| line.contains("▌ 100 │ needle")),
                    "{lines:?}"
                );
                assert_eq!(lines.iter().filter(|line| line.contains('▌')).count(), 1);
                assert!(
                    lines.iter().any(|line| line.contains("  99 │ plain")),
                    "{lines:?}"
                );
            } else {
                assert!(
                    lines.iter().any(|line| line.contains("   1 │ plain")),
                    "{lines:?}"
                );
                assert!(lines.iter().all(|line| !line.contains('▌')));
            }
            // the header is the workspace-relative path, never the root
            assert!(lines[0].contains(" source.txt"), "{lines:?}");
            assert!(
                !lines[0].contains(&root.path().to_string_lossy().to_string()),
                "{lines:?}"
            );
        }
    }

    #[test]
    fn workspace_symbols_preview_points_at_the_declaration_line() {
        // 0063 §2: the workspace tier shares the symbols pane shape —
        // numbered gutter, one ▌ marker on the declaration line.
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("lib.rs");
        let text = format!("{}fn dispatch() {{}}\n", "filler\n".repeat(41));
        std::fs::write(&path, text).unwrap();
        let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
        editor.open_fixture(&path).unwrap();
        editor.open_picker(strop_picker::Kind::WorkspaceSymbols);
        editor.wait_picker();
        editor.picker_reset_fixture();
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
        let preview = editor.prepared_view().picker.unwrap().preview.unwrap();
        let right: Vec<String> = (0..30)
            .map(|y| (preview.x..100).map(|x| grid[(x, y)].symbol()).collect())
            .collect();
        assert!(
            right.iter().any(|line| line.contains("▌ 42 │")),
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
        editor.wait_picker();
        editor.picker_reset_fixture();
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
        // carry the numbered gutter, the ▌ marker on the symbol's line and
        // the divider — exactly the rootle-style shape, at its prepared width.
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| crate::render::paint(&mut editor, frame))
            .unwrap();
        let grid = terminal.backend().buffer();
        let preview = editor.prepared_view().picker.unwrap().preview.unwrap();
        let right: Vec<String> = (0..30)
            .map(|y| (preview.x..100).map(|x| grid[(x, y)].symbol()).collect())
            .collect();
        assert!(
            right.iter().any(|line| line.contains("▌ 42 │")),
            "symbol marker on line 42: {right:?}"
        );
        assert_eq!(
            right.iter().filter(|line| line.contains('▌')).count(),
            1,
            "exactly one marker in the preview"
        );
        assert!(
            right.iter().any(|line| line.contains("│ fn dispatch")),
            "divider then content: {right:?}"
        );
        let t = tokens();
        let mut marker = None;
        for x in preview.x..100 {
            for y in 0..30u16 {
                if grid[(x, y)].symbol() == "▌" {
                    marker = Some((x, y));
                }
            }
        }
        let (marker_x, marker_y) = marker.unwrap();
        let marker = &grid[(marker_x, marker_y)];
        assert_eq!(marker.fg, t.accent);
        assert_eq!(marker.bg, t.raised);
    }

    #[test]
    fn gutters_remain_cell_bounded_and_do_not_number_a_phantom_eof() {
        let rope = ropey::Rope::from_str("\t界e\u{301}\n");
        for width in 0..16 {
            let window = preview_window(&rope, Some(1), 10);
            assert_eq!(window, 0..1);
            let lines =
                highlight_lines_owned(&tokens(), &rope, None, Some(1), window, width, 4, None);
            assert_eq!(lines.len(), 1);
            assert!(lines[0].width() <= width);
        }
    }
}
