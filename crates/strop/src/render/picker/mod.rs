//! The picker card (0003 §2): centered floating card over a dimmed
//! backdrop; input top, results left, preview right. House style:
//! `▌` selection marker, accent+bold matched chars, hints in the
//! bottom border, border-column scrollbar.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::Editor;

mod preview;
mod rows;

use rows::render_replace_results;
use rows::render_results;

use super::{dim_color, ACCENT, BASE, MUTED, TEXT};

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
    let narrow_card = card.width < 64;
    let hint = if narrow_card {
        // whole groups by priority; never cut through a chord (0050 §8)
        if collectable {
            " enter open · ctrl-o collect · esc "
        } else {
            " enter open · esc "
        }
    } else if replace_mode {
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
    } else if row_count != total {
        // 0050 §8: a filtered list says so — visible / total
        format!(" {row_count} / {total} ")
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

    // 0050 §8: the decision list wins the budget — symbols/files 60%,
    // grep 65%; narrow terminals stack a short preview below the list
    // instead of two unreadable slivers; very little height lists only.
    let (results, preview_area) = if remote_picker {
        (rows[1], None)
    } else if card.width < 64 && rows[1].height >= 12 {
        let stacked = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(8)])
            .split(rows[1]);
        (stacked[0], Some(stacked[1]))
    } else if card.width < 64 {
        (rows[1], None)
    } else {
        let list = if kind == strop_picker::Kind::Grep {
            65
        } else {
            60
        };
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(list),
                Constraint::Percentage(100 - list),
            ])
            .split(rows[1]);
        (cols[0], Some(cols[1]))
    };

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
        // the scrollbar track is reserved BEFORE text budgets (0050 §8)
        let text_area = Rect {
            width: results.width.saturating_sub(1),
            ..results
        };
        render_results(frame, text_area, p, selected);
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
    if let Some(preview_area) = preview_area {
        preview::render_preview(editor, frame, preview_area);
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
        crate::render::frame_capture::place_cursor(frame, (caret_x, rows[0].y + caret_row));
    }
}
