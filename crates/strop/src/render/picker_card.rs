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

use super::{dim_color, ACCENT, BASE, MUTED, SELECT_BG, TEXT};

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
        // 0.3.6 form factor: replace keeps the full frame (its two
        // fields + exclusion set need the room); grep is a quick-lookup
        // card again (0003 §5.2)
        if matches!(p.kind, strop_picker::Kind::Replace) {
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
            let height = if p.kind == strop_picker::Kind::RemoteAddress {
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
    };
    frame.render_widget(Clear, card);

    let (
        kind,
        input,
        replace_input,
        input_cursor,
        replace_cursor,
        field,
        row_count,
        selected,
        streaming,
        total,
        excluded,
        normal_mode,
        picker_error,
    ) = {
        let glue = editor.picker.as_ref().expect("picker open");
        let p = &glue.picker;
        (
            p.kind,
            p.input.text.clone(),
            p.replace_input.text.clone(),
            p.input.cursor,
            p.replace_input.cursor,
            p.field,
            p.rows.len(),
            p.selected,
            p.streaming || glue.rank_pending.is_some(),
            p.items.len(),
            // row + file exclusions both count (0007)
            p.excluded_count(),
            p.input_normal(),
            p.error.clone(),
        )
    };
    let replace_mode = kind == strop_picker::Kind::Replace;
    let remote_picker = matches!(
        kind,
        strop_picker::Kind::RemoteHosts | strop_picker::Kind::RemoteAddress
    );

    let collectable = matches!(
        kind,
        strop_picker::Kind::Locations | strop_picker::Kind::Diagnostics | strop_picker::Kind::Grep
    );
    let hint = if replace_mode {
        " enter apply · tab field · ctrl-x row · ctrl-d file · esc  —  -t rs / --glob filters "
    } else if collectable {
        " enter open · ctrl-o collect · esc normal/close · ↑↓/tab move "
    } else if kind == strop_picker::Kind::RemoteAddress {
        " enter connect · esc normal/close "
    } else if remote_picker {
        " enter choose/connect · esc normal/close · ↑↓/tab move "
    } else {
        " enter open · esc normal/close · ↑↓/tab move · j/k after esc "
    };
    let count = if let Some(e) = &picker_error {
        // a failed source is the headline, not the count (0014: rg's
        // bad-flag stderr reads like an empty project otherwise)
        format!(" {e} ")
    } else if replace_mode {
        format!(" {excluded}/{total} excluded ")
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
    // the prompt glyph is the mode indicator: ❯ types into the field,
    // ▮ means Esc parked you in normal mode (j/k walk the results)
    let glyph = if normal_mode { "▮" } else { "❯" };
    if replace_mode {
        let search_label = if normal_mode {
            "▮ find   "
        } else {
            "❯ find   "
        };
        let replace_label = if normal_mode {
            "▮ replace"
        } else {
            "❯ replace"
        };
        let search = field_prompt(search_label, &input, field == strop_picker::Field::Search);
        let replace = field_prompt(
            replace_label,
            &replace_input,
            field == strop_picker::Field::Replace,
        );
        frame.render_widget(Paragraph::new(vec![search, replace]), rows[0]);
    } else {
        let prompt = field_prompt(glyph, &input, true);
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
    let results = if remote_picker { rows[1] } else { cols[0] };

    if kind == strop_picker::Kind::RemoteAddress {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from("host or user@host[:port]"),
                Line::from("ssh://host/absolute/path  —  file or directory"),
                Line::from("New hosts open /; no mount or recursive workspace scan."),
            ])
            .style(Style::default().fg(MUTED)),
            results,
        );
    } else if replace_mode {
        let p = &editor.picker.as_ref().expect("picker open").picker;
        render_replace_results(frame, results, p);
    } else {
        let p = &editor.picker.as_ref().expect("picker open").picker;
        render_results(frame, results, p, selected);
    }
    // border-column scrollbar for the results list (0003 §5.5)
    if !results.is_empty() && row_count > results.height as usize {
        let track_x = results.x + results.width - 1;
        let track_h = results.height as usize;
        let frac = selected as f32 / row_count.max(1) as f32;
        let thumb = ((track_h - 1) as f32 * frac) as usize;
        for y in 0..track_h {
            let cell = &mut frame.buffer_mut()[(track_x, results.y + y as u16)];
            if y == thumb {
                cell.set_symbol("▮");
                cell.set_fg(ACCENT);
            } else {
                cell.set_symbol("│");
                cell.set_fg(Color::Rgb(0x2a, 0x2c, 0x3a));
            }
        }
    }
    if !remote_picker {
        render_preview(editor, frame, cols[1]);
    }
    let _ = streaming; // spinner lands with the 100ms rule (0001 §4)
    let (caret_len, caret_row) = if replace_mode && field == strop_picker::Field::Replace {
        (10 + replace_input[..replace_cursor].chars().count(), 1u16)
    } else if replace_mode {
        (10 + input[..input_cursor].chars().count(), 0u16)
    } else {
        (2 + input[..input_cursor].chars().count(), 0u16)
    };
    let caret_x = rows[0].x + caret_len as u16;
    if caret_x < rows[0].x + rows[0].width {
        crate::editor::trace::frame::place_cursor(frame, (caret_x, rows[0].y + caret_row));
    }
}

fn render_results(frame: &mut Frame, area: Rect, picker: &strop_picker::Picker, selected: usize) {
    let visible = area.height as usize;
    let start = if selected >= visible {
        selected + 1 - visible
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::with_capacity(visible);
    for (vi, row) in picker.rows.iter().enumerate().skip(start).take(visible) {
        let active = vi == selected;
        let marker = if active { "▌" } else { " " };
        let style = Style::default().fg(if active { ACCENT } else { MUTED });
        let mut spans = vec![Span::styled(marker, style)];
        // matched chars accent+bold, never background blocks (0001 §4)

        let Some(item) = picker.items.get(row.item) else {
            continue;
        };
        let text = super::text::clip_end(&item.text, area.width.saturating_sub(1) as usize);
        let match_cols = picker.match_columns(row);
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
        let excluded = p.is_excluded(row.item);
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
                super::text::clip_end(&line_text[..s], area.width as usize).into_owned(),
                Style::default().fg(text_fg),
            ));
            spans.push(Span::styled(
                super::text::clip_end(&line_text[s..e], area.width as usize).into_owned(),
                Style::default()
                    .fg(dim_color(TEXT))
                    .add_modifier(Modifier::CROSSED_OUT),
            ));
            if !p.replace_input.text.is_empty() {
                spans.push(Span::styled(
                    super::text::clip_end(&p.replace_input.text, area.width as usize).into_owned(),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ));
            }
            spans.push(Span::styled(
                super::text::clip_end(&line_text[e..], area.width as usize).into_owned(),
                Style::default().fg(text_fg),
            ));
        } else {
            spans.push(Span::styled(
                super::text::clip_end(&item.text, area.width as usize).into_owned(),
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

fn render_preview(editor: &mut Editor, frame: &mut Frame, area: Rect) {
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
                style = style.patch(super::syntax_style(span));
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
