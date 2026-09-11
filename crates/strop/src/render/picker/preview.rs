//! The picker's source preview: windowed, syntax-spanned, header first
//! (0050: filename:line identifies before parent/root details).
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::editor::{Editor, PreviewSource};

use super::super::{syntax_style, BASE, MUTED, SELECT_BG, TEXT};
pub(super) fn render_preview(editor: &mut Editor, frame: &mut Frame, area: Rect) {
    let Some((title, focus_line, source)) = editor.picker_preview() else {
        frame.render_widget(Paragraph::new("").style(Style::default().bg(BASE)), area);
        return;
    };
    let visible = area.height as usize;
    let width = usize::from(area.width.saturating_sub(1));
    let tab = editor.config.tab_size;

    let lines: Vec<Line> = match source {
        PreviewSource::Buffer(document) => {
            let rope = editor.doc(document).buf.snapshot();
            let window = preview_window(&rope, focus_line, visible);
            let analysis = editor.document_analysis(
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
            )
        }
        PreviewSource::Cached(path) => {
            let Some(entry) = editor.previews.get(&path) else {
                return;
            };
            let rope = entry.rope.clone();
            let window = preview_window(&rope, focus_line, visible);
            let analysis = editor.preview_analysis(
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
            )
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
        .style(Style::default().bg(BASE))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(MUTED),
        ));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn preview_window(
    rope: &ropey::Rope,
    focus: Option<usize>,
    visible: usize,
) -> std::ops::Range<usize> {
    let first = focus
        .map_or(0, |line| line.saturating_sub(1).saturating_sub(visible / 3))
        .min(rope.len_lines());
    first..first.saturating_add(visible).min(rope.len_lines())
}

fn highlight_lines_owned(
    rope: &ropey::Rope,
    spans: Option<&[strop_syntax::Span]>,
    focus_line: Option<usize>,
    window: std::ops::Range<usize>,
    width: usize,
    tab: usize,
) -> Vec<Line<'static>> {
    use strop_core::layout::{clip, printable_grapheme, RopeGraphemes};
    let spans = spans.unwrap_or_default();
    let mut out = Vec::with_capacity(window.len());
    for line in window {
        let start = rope.line_to_byte(line);
        let text = rope.line(line);
        let mut end = text.len_bytes();
        while end > 0 && matches!(text.byte(end - 1), b'\r' | b'\n') {
            end -= 1;
        }
        let text = text.byte_slice(..end);
        let first_span = spans.partition_point(|span| span.end <= start);
        let mut spans_out = Vec::new();
        for (placement, grapheme) in RopeGraphemes::new(text, tab) {
            if placement.cell.get() >= width {
                break;
            }
            let Some(visible) = clip(placement, strop_core::id::DisplayColumn::new(0), width)
            else {
                continue;
            };
            let pos = start + placement.byte;
            let mut style = Style::default().fg(TEXT);
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
            let symbol = if !visible.complete || grapheme == "\t" {
                " ".repeat(visible.width)
            } else {
                printable_grapheme(&grapheme).to_owned()
            };
            spans_out.push(Span::styled(symbol, style));
        }
        out.push(Line::from(spans_out));
    }
    out
}
