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

    // power searches breathe: grep/replace take the full frame (one
    // cell margin), lookups stay a centered card (0003 §5.2)
    let card = {
        let glue = editor.picker.as_ref().expect("picker open");
        let p = &glue.picker;
        if matches!(
            p.kind,
            strop_picker::Kind::Grep | strop_picker::Kind::Replace
        ) {
            Rect {
                x: area.x + 1,
                y: area.y,
                width: area.width.saturating_sub(2),
                height: area.height.saturating_sub(1),
            }
        } else {
            let width = (area.width * 84 / 100).clamp(50, area.width.saturating_sub(2));
            let height = (area.height * 70 / 100).clamp(12, area.height.saturating_sub(2));
            Rect {
                x: (area.width - width) / 2,
                y: (area.height - height) / 2,
                width,
                height,
            }
        }
    };
    frame.render_widget(Clear, card);

    let (kind, input, replace_input, field, rows_data, selected, streaming, total, excluded) = {
        let glue = editor.picker.as_ref().expect("picker open");
        let p = &glue.picker;
        (
            p.kind,
            p.input.clone(),
            p.replace_input.clone(),
            p.field,
            p.rows.clone(),
            p.selected,
            p.streaming,
            p.items.len(),
            p.excluded.clone(),
        )
    };
    let replace_mode = kind == strop_picker::Kind::Replace;

    let hint = if replace_mode {
        " enter apply · tab switch field · ctrl-x exclude · esc close "
    } else {
        " enter open · esc close · ↑↓/tab move "
    };
    let count = if replace_mode {
        format!(" {}/{total} excluded ", excluded.len())
    } else if streaming {
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
            kind.title(),
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

    // input row(s) + content split; replace mode adds a second field
    let input_h = if replace_mode { 3 } else { 2 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(input_h), Constraint::Min(1)])
        .split(inner);
    let field_prompt = |label: &str, value: &str, active: bool| {
        let fg = if active { ACCENT } else { MUTED };
        let caret = if active { "▏" } else { " " };
        Line::from(vec![
            Span::styled(format!("{label} "), Style::default().fg(fg)),
            Span::styled(value.to_string(), Style::default().fg(TEXT)),
            Span::styled(caret, Style::default().fg(fg)),
        ])
    };
    if replace_mode {
        let search = field_prompt("❯ find   ", &input, field == strop_picker::Field::Search);
        let replace = field_prompt(
            "❯ replace",
            &replace_input,
            field == strop_picker::Field::Replace,
        );
        frame.render_widget(Paragraph::new(vec![search, replace]), rows[0]);
    } else {
        let prompt = field_prompt("❯", &input, true);
        frame.render_widget(Paragraph::new(vec![prompt]), rows[0]);
    }
    // section definition: a rule separates where you type from results
    let rule_y = rows[0].y + input_h - 1;
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

    if replace_mode {
        let p = &editor.picker.as_ref().expect("picker open").picker;
        render_replace_results(frame, cols[0], p);
    } else {
        render_results(frame, cols[0], &rows_data, selected);
    }
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

    // input caret on the focused field
    let (caret_len, caret_row) = if replace_mode && field == strop_picker::Field::Replace {
        (10 + replace_input.chars().count(), 1u16)
    } else if replace_mode {
        (10 + input.chars().count(), 0u16)
    } else {
        (2 + input.chars().count(), 0u16)
    };
    let caret_x = rows[0].x + caret_len as u16;
    if caret_x < rows[0].x + rows[0].width {
        frame.set_cursor_position((caret_x, rows[0].y + caret_row));
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

/// Replace-mode rows (0007 §2): the replacement previews inline — the
/// matched span strikethrough-dimmed, the replacement in accent — built
/// from the same `replace_span` the apply path uses. Excluded rows dim
/// and wear a ✗.
fn render_replace_results(frame: &mut Frame, area: Rect, p: &strop_picker::Picker) {
    let visible = area.height as usize;
    let start = if p.selected >= visible {
        p.selected + 1 - visible
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::with_capacity(visible);
    for (vi, row) in p.rows.iter().enumerate().skip(start).take(visible) {
        let active = vi == p.selected;
        let excluded = p.excluded.contains(&row.item);
        let marker = if excluded {
            "✗"
        } else if active {
            "▌"
        } else {
            " "
        };
        let marker_fg = if excluded || !active { MUTED } else { ACCENT };
        let mut spans = vec![Span::styled(marker, Style::default().fg(marker_fg))];
        let text_fg = if excluded {
            dim_color(TEXT)
        } else if active {
            TEXT
        } else {
            Color::Rgb(0xb8, 0xb4, 0xa9)
        };
        // respawns clear items before rows catch up — never index blind
        let Some(item) = p.items.get(row.item) else {
            continue;
        };
        if let strop_picker::Payload::Grep {
            path,
            line,
            col,
            match_len,
            line_text,
        } = &item.payload
        {
            spans.push(Span::styled(
                format!(" {}:{line} · ", path.display()),
                Style::default().fg(MUTED),
            ));
            let (s, e) = strop_picker::replace_span(line_text, *col, *match_len);
            spans.push(Span::styled(
                line_text[..s].to_string(),
                Style::default().fg(text_fg),
            ));
            spans.push(Span::styled(
                line_text[s..e].to_string(),
                Style::default()
                    .fg(dim_color(TEXT))
                    .add_modifier(Modifier::CROSSED_OUT),
            ));
            if !p.replace_input.is_empty() {
                spans.push(Span::styled(
                    p.replace_input.clone(),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ));
            }
            spans.push(Span::styled(
                line_text[e..].to_string(),
                Style::default().fg(text_fg),
            ));
        } else {
            spans.push(Span::styled(
                item.text.clone(),
                Style::default().fg(text_fg),
            ));
        }
        if active {
            spans = spans
                .into_iter()
                .map(|sp| sp.patch_style(Style::default().bg(SELECT_BG)))
                .collect();
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
        PreviewSource::Buffer(i) => {
            let rope = editor.buffers[i].rope.clone();
            let spans = editor
                .highlighters
                .get_mut(i)
                .and_then(|h| h.as_mut())
                .map(|hl| hl.highlight(&rope, 0, rope.len_bytes()));
            highlight_lines_owned(&rope, spans.as_deref(), focus_line, visible)
        }
        PreviewSource::Cached(entry) => {
            let rope = entry.rope.clone();
            let spans = entry
                .hl
                .as_mut()
                .map(|hl| hl.highlight(&entry.rope, 0, entry.rope.len_bytes()));
            highlight_lines_owned(&rope, spans.as_deref(), focus_line, visible)
        }
        PreviewSource::Loading => vec![Line::from(Span::styled(
            " loading…",
            Style::default().fg(MUTED),
        ))],
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
