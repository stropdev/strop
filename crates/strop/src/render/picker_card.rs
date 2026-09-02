//! The picker card (0003 §2): centered floating card over a dimmed
//! backdrop; input top, results left, preview right. House style:
//! `▌` selection marker, accent+bold matched chars, hints in the
//! bottom border, border-column scrollbar.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::{Editor, PreviewSource};

use super::{class_color, dim_color, ACCENT, BASE, MUTED, SELECT_BG, TEXT};

/// Dim the backdrop: the editor stays readable under the card (0003 §2.1
/// live backdrop), with fg colors pulled toward the base.
pub fn dim_backdrop(frame: &mut Frame, area: Rect) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut frame.buffer_mut()[(x, y)];
            cell.set_fg(dim_color(cell.fg));
            if cell.bg != BASE {
                cell.set_bg(dim_color(cell.bg));
            }
        }
    }
}

pub fn render_picker(editor: &mut Editor, frame: &mut Frame) {
    if !editor.picker_open() {
        return;
    }
    let area = frame.area();
    dim_backdrop(frame, area);

    // centered card, clamped to the viewport (0003 §5.2)
    let width = (area.width * 84 / 100).clamp(50, area.width.saturating_sub(2));
    let height = (area.height * 70 / 100).clamp(12, area.height.saturating_sub(2));
    let card = Rect {
        x: (area.width - width) / 2,
        y: (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, card);

    let (title, input, rows_data, selected, streaming, total) = {
        let glue = editor.picker.as_ref().expect("picker open");
        let p = &glue.picker;
        (
            p.kind.title(),
            p.input.clone(),
            p.rows.clone(),
            p.selected,
            p.streaming,
            p.items.len(),
        )
    };

    let hint = " enter open · esc close · ↑↓/tab move ";
    let count = if streaming {
        format!(" {total}… ")
    } else {
        format!(" {total} ")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BASE))
        .title(Span::styled(
            title,
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(hint, Style::default().fg(MUTED)))
        .title_top(Line::from(Span::styled(count, Style::default().fg(MUTED))).right_aligned());
    // 1-cell inner padding (0001 §4: floating panes breathe)
    let inner = block.inner(card);
    let inner = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };
    frame.render_widget(&block, card);
    frame.render_widget(&block, card);

    // input row + content split
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(inner);
    let prompt = Line::from(vec![
        Span::styled("❯ ", Style::default().fg(ACCENT)),
        Span::styled(input.clone(), Style::default().fg(TEXT)),
        Span::styled("▏", Style::default().fg(ACCENT)),
    ]);
    frame.render_widget(Paragraph::new(prompt), rows[0]);
    // section definition: a rule separates where you type from results
    let rule_y = rows[0].y + 1;
    if rule_y < rows[1].y {
        let rule: String = "─".repeat(rows[0].width as usize);
        frame.render_widget(
            Paragraph::new(rule).style(Style::default().fg(Color::Rgb(0x3a, 0x3d, 0x4d))),
            Rect {
                y: rule_y,
                height: 1,
                ..rows[0]
            },
        );
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(rows[1]);

    render_results(frame, cols[0], &rows_data, selected);
    // border-column scrollbar for the results list (0003 §5.5)
    if rows_data.len() > cols[0].height as usize && !rows_data.is_empty() {
        let track_x = cols[0].x + cols[0].width - 1;
        let track_h = cols[0].height as usize;
        let frac = selected as f32 / rows_data.len().max(1) as f32;
        let thumb = ((track_h - 1) as f32 * frac) as usize;
        for y in 0..track_h {
            let cell = &mut frame.buffer_mut()[(track_x, cols[0].y + y as u16)];
            if y == thumb {
                cell.set_symbol("▮");
                cell.set_fg(ACCENT);
            } else {
                cell.set_symbol("│");
                cell.set_fg(Color::Rgb(0x2a, 0x2c, 0x3a));
            }
        }
    }
    render_preview(editor, frame, cols[1]);
    let _ = streaming; // spinner lands with the 100ms rule (0001 §4)

    // input caret
    let caret_x = rows[0].x + 2 + input.chars().count() as u16;
    if caret_x < rows[0].x + rows[0].width {
        frame.set_cursor_position((caret_x, rows[0].y));
    }
}

fn render_results(frame: &mut Frame, area: Rect, rows: &[strop_picker::Row], selected: usize) {
    let visible = area.height as usize;
    let start = if selected >= visible {
        selected + 1 - visible
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::with_capacity(visible);
    for (vi, row) in rows.iter().enumerate().skip(start).take(visible) {
        let active = vi == selected;
        let marker = if active { "▌" } else { " " };
        let style = Style::default().fg(if active { ACCENT } else { MUTED });
        let mut spans = vec![Span::styled(marker, style)];
        // matched chars accent+bold, never background blocks (0001 §4)
        let text = row_text(row);
        let match_cols: Vec<u32> = row.match_cols.clone();
        let base_fg = if active {
            TEXT
        } else {
            Color::Rgb(0xb8, 0xb4, 0xa9)
        };
        for (ci, ch) in text.chars().enumerate() {
            let mut st = Style::default().fg(base_fg);
            if match_cols.contains(&(ci as u32)) {
                st = st.fg(ACCENT).add_modifier(Modifier::BOLD);
            }
            if active {
                st = st.bg(SELECT_BG);
            }
            spans.push(Span::styled(ch.to_string(), st));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(BASE)), area);
}

fn row_text(row: &strop_picker::Row) -> &str {
    &row.text
}

fn render_preview(editor: &mut Editor, frame: &mut Frame, area: Rect) {
    let Some((title, focus_line, source)) = editor.picker_preview() else {
        frame.render_widget(Paragraph::new("").style(Style::default().bg(BASE)), area);
        return;
    };
    let visible = area.height as usize;

    let lines: Vec<Line> = match source {
        PreviewSource::Live(rope) => highlight_lines_owned(rope, None, focus_line, visible),
        PreviewSource::Cached(entry) => {
            let rope = entry.rope.clone();
            let spans = entry
                .hl
                .as_mut()
                .map(|hl| hl.highlight(&entry.rope, 0, entry.rope.len_bytes()));
            highlight_lines_owned(&rope, spans.as_deref(), focus_line, visible)
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

fn text_len(rope: &ropey::Rope, line: usize) -> usize {
    rope.line(line).len_bytes().saturating_sub(1) // exclude \n
}

fn highlight_lines_owned(
    rope: &ropey::Rope,
    spans: Option<&[strop_syntax::Span]>,
    focus_line: Option<usize>,
    visible: usize,
) -> Vec<Line<'static>> {
    let total = rope.len_lines();
    let top = match focus_line {
        Some(l) => l.saturating_sub(1).saturating_sub(visible / 3),
        None => 0,
    };
    let spans = spans.map(|s| s.to_vec()).unwrap_or_default();
    let mut out = Vec::with_capacity(visible);
    for li in top..(top + visible).min(total) {
        let start = rope.line_to_byte(li);
        let end = start + text_len(rope, li);
        let line_spans: Vec<&strop_syntax::Span> = spans
            .iter()
            .filter(|s| s.start < end && s.end > start)
            .collect();
        let text = rope.line(li).to_string();
        let mut spans_out = Vec::new();
        for (i, ch) in text.trim_end_matches('\n').chars().enumerate() {
            let pos = start + i;
            let mut style = Style::default().fg(TEXT);
            // most specific (smallest) span wins
            if let Some(sp) = line_spans
                .iter()
                .filter(|s| s.start <= pos && pos < s.end)
                .min_by_key(|s| s.end - s.start)
            {
                style = style.fg(class_color(sp.class));
            }
            if focus_line == Some(li + 1) {
                style = style.bg(SELECT_BG);
            }
            spans_out.push(Span::styled(ch.to_string(), style));
        }
        out.push(Line::from(spans_out));
    }
    out
}
