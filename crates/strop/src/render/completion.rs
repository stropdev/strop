//! A non-modal card inside the active pane. Geometry depends on pane/caret
//! cells, never on a candidate label's byte length or asynchronous arrival order.
use super::{accent, base, muted, secondary, select_bg, text};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use strop_engine::editor::{CompletionDocumentation, CompletionMenu, Editor};

const MENU_WIDTH: u16 = 58;
const MENU_ROWS: u16 = 6;
const DOC_WIDTH: u16 = 40;

#[cfg(test)]
mod tests;

pub(super) fn render(editor: &Editor, frame: &mut Frame, pane: Rect, caret: Option<(u16, u16)>) {
    let (Some(menu), Some((caret_x, caret_y))) = (editor.completion_menu(), caret) else {
        return;
    };
    let bounds = Rect {
        width: pane.width.saturating_sub(1),
        ..pane
    }; // keep the pane's scrollbar
    if bounds.width < 12 || bounds.height < 3 || caret_y < bounds.y || caret_y >= bounds.bottom() {
        return;
    }
    // The keyword is bounded and contains no tab stops. Reuse real grapheme
    // widths instead of rescanning the source line or counting Unicode bytes.
    let prefix_cells: usize = strop_core::layout::RopeGraphemes::new(menu.prefix(), 1)
        .map(|(span, _)| span.width)
        .sum();
    let anchor = caret_x
        .saturating_sub(u16::try_from(prefix_cells).unwrap_or(u16::MAX))
        .max(bounds.x);
    let below = bounds.bottom().saturating_sub(caret_y + 1);
    let above = caret_y.saturating_sub(bounds.y);
    let wanted = if menu.is_empty() { 4 } else { MENU_ROWS + 2 };
    let below_first = below >= wanted || below >= above;
    let height = wanted.min(if below_first { below } else { above });
    if height < 3 {
        return;
    }
    let width = MENU_WIDTH.min(bounds.width);
    let card = Rect::new(
        anchor.min(bounds.right() - width),
        if below_first {
            caret_y + 1
        } else {
            caret_y - height
        },
        width,
        height,
    );
    let title = if menu.incomplete() {
        " completions · incomplete "
    } else if menu.limited() {
        " completions · limited "
    } else {
        " completions "
    };
    frame.render_widget(Clear, card);
    let hint = format!(
        "{} {} · {} {} · {} {}",
        strop_engine::keymap::completion::CYCLE.keys,
        strop_engine::keymap::completion::CYCLE.desc,
        strop_engine::keymap::completion::ACCEPT.keys,
        strop_engine::keymap::completion::ACCEPT.desc,
        strop_engine::keymap::completion::DISMISS.keys,
        strop_engine::keymap::completion::DISMISS.desc
    );
    let footer = menu.action_status().unwrap_or(&hint);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(base()).fg(text()))
        .border_style(Style::default().fg(muted()))
        .title(Span::styled(title, Style::default().fg(secondary())))
        .title_bottom(Span::styled(
            super::clip::clip_end(footer, usize::from(width.saturating_sub(4))),
            Style::default().fg(muted()),
        ));
    let inner = block.inner(card);
    frame.render_widget(block, card);
    if menu.is_empty() {
        let lines: Vec<_> = menu
            .providers()
            .into_iter()
            .filter(|provider| provider.state != "off")
            .map(|provider| {
                Line::from(vec![
                    Span::styled(
                        format!("{}  ", provider.source),
                        Style::default().fg(muted()),
                    ),
                    Span::styled(
                        super::clip::clip_end(
                            provider.explanation,
                            usize::from(inner.width.saturating_sub(5)),
                        ),
                        Style::default().fg(secondary()),
                    ),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }
    let selected = menu.selected();
    let rows = usize::from(inner.height);
    let start = selected
        .map_or(0, |index| index.saturating_sub(rows.saturating_sub(1)))
        .min(menu.len().saturating_sub(rows));
    for offset in 0..rows {
        let index = start + offset;
        let row = Rect::new(inner.x, inner.y + offset as u16, inner.width, 1);
        render_row(&menu, index, selected == Some(index), frame, row);
    }
    if menu.len() > rows {
        let indicator = format!(
            " {}/{} ",
            selected.map_or(start + 1, |index| index + 1),
            menu.len()
        );
        let cells = super::clip::width(&indicator).min(usize::from(width.saturating_sub(2))) as u16;
        frame.render_widget(
            Paragraph::new(indicator).style(Style::default().fg(muted()).bg(base())),
            Rect::new(card.right() - cells - 1, card.y, cells, 1),
        );
    }
    if let Some(documentation) = menu.documentation() {
        if let Some(area) = documentation_area(bounds, card, caret_y) {
            render_documentation(documentation, frame, area);
        }
    }
}

fn render_row(
    menu: &CompletionMenu<'_>,
    index: usize,
    selected: bool,
    frame: &mut Frame,
    area: Rect,
) {
    let background = if selected { select_bg() } else { base() };
    let base = Style::default().fg(text()).bg(background);
    frame.buffer_mut().set_style(area, base);
    let Some(row) = menu.row(index) else { return };
    let wide = area.width >= 34;
    let kind_width = if wide { 7 } else { 0 };
    let source_width = 4;
    let available = area.width.saturating_sub(kind_width + source_width + 2);
    let label_width = if wide { available.min(27) } else { available };
    let detail_width = available.saturating_sub(label_width + 1);
    let label = super::clip::clip_end(row.label, usize::from(label_width));
    let matched = prefix_end(&label, menu.prefix());
    let mut spans = Vec::with_capacity(9);
    spans.push(Span::styled(
        if selected { "▌" } else { " " },
        base.fg(accent()),
    ));
    if wide {
        spans.push(Span::styled(format!("{:<6} ", row.kind), base.fg(muted())));
    }
    spans.push(Span::styled(
        &label[..matched],
        base.fg(accent()).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(&label[matched..], base));
    let label_cells = super::clip::width(&label);
    spans.push(Span::styled(
        " ".repeat(usize::from(label_width).saturating_sub(label_cells)),
        base,
    ));
    if detail_width > 0 {
        let detail = super::clip::clip_end(
            row.unsupported.unwrap_or(row.detail),
            usize::from(detail_width),
        );
        let padding = usize::from(detail_width).saturating_sub(super::clip::width(&detail));
        spans.push(Span::styled(" ", base));
        spans.push(Span::styled(detail, base.fg(secondary())));
        spans.push(Span::styled(" ".repeat(padding), base));
    }
    spans.push(Span::styled(
        format!(" {:>3}", row.source),
        base.fg(muted()),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}

fn prefix_end(label: &str, prefix: ropey::RopeSlice<'_>) -> usize {
    let mut label = label.char_indices();
    let mut end = 0;
    for wanted in prefix.chars() {
        let Some((byte, character)) = label.next() else {
            return 0;
        };
        if !character.to_lowercase().eq(wanted.to_lowercase()) {
            return 0;
        }
        end = byte + character.len_utf8();
    }
    end
}

fn documentation_area(bounds: Rect, menu: Rect, caret_y: u16) -> Option<Rect> {
    let right = bounds.right().saturating_sub(menu.right());
    if right >= 28 {
        return Some(Rect::new(
            menu.right(),
            menu.y,
            right.min(DOC_WIDTH),
            menu.height.min(bounds.bottom() - menu.y),
        ));
    }
    let left = menu.x.saturating_sub(bounds.x);
    if left >= 28 {
        let width = left.min(DOC_WIDTH);
        return Some(Rect::new(menu.x - width, menu.y, width, menu.height));
    }
    let below = bounds.bottom().saturating_sub(menu.bottom());
    if menu.bottom() > caret_y && below >= 4 {
        return Some(Rect::new(menu.x, menu.bottom(), menu.width, below.min(8)));
    }
    let above = menu.y.saturating_sub(bounds.y);
    if menu.y < caret_y && above >= 4 {
        let height = above.min(8);
        return Some(Rect::new(menu.x, menu.y - height, menu.width, height));
    }
    None
}

fn render_documentation(documentation: CompletionDocumentation<'_>, frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let title = if documentation.truncated {
        " documentation · truncated "
    } else {
        " documentation "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(text()).bg(base()))
        .border_style(Style::default().fg(muted()))
        .title(Span::styled(title, Style::default().fg(muted())));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = Vec::new();
    let mut byte = 0;
    for raw in documentation.text.split_inclusive('\n') {
        let text = raw.trim_end_matches(['\r', '\n']);
        let end = byte + text.len();
        let mut spans = Vec::new();
        let mut offset = byte;
        for syntax in documentation
            .spans
            .iter()
            .filter(|span| span.end > byte && span.start < end)
        {
            let start = syntax.start.max(byte).max(offset);
            let finish = syntax.end.min(end);
            if start > offset {
                spans.push(Span::raw(strop_core::layout::printable_text(
                    &documentation.text[offset..start],
                )));
            }
            if finish > start {
                spans.push(Span::styled(
                    strop_core::layout::printable_text(&documentation.text[start..finish]),
                    super::syntax_style(syntax),
                ));
                offset = finish;
            }
        }
        if offset < end {
            spans.push(Span::raw(strop_core::layout::printable_text(
                &documentation.text[offset..end],
            )));
        }
        lines.push(Line::from(spans));
        byte += raw.len();
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(text()))
            .wrap(Wrap { trim: false }),
        inner,
    );
}
