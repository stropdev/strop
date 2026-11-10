//! The completion card (0088 §2 mockup): a non-modal card inside the
//! active pane, one row per candidate — kind in secondary, label in text
//! with the matched prefix in accent, source as a 3-letter tag in muted
//! — and whole-chord keycap hints in the bottom border (U16). Geometry
//! depends on pane/caret cells, never on a candidate label's byte
//! length or asynchronous arrival order.
use super::card::{card_width, row_marker, Card, Hint};
use super::style::Tokens;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;
use strop_engine::editor::{CompletionDocumentation, CompletionMenu, Editor};

const MENU_ROWS: u16 = 6;
const DOC_WIDTH: u16 = 40;
const KIND_W: usize = 6;
const SOURCE_W: usize = 3;

#[cfg(test)]
mod tests;

/// The footer: the keymap's completion chords with one-word verbs, so
/// the whole chord always fits (U16); the long descriptions stay in `?`.
fn hints() -> [Hint<'static>; 3] {
    use strop_engine::keymap::completion::{ACCEPT, CYCLE, DISMISS};
    [
        Hint {
            key: CYCLE.keys,
            desc: "next",
        },
        Hint {
            key: ACCEPT.keys,
            desc: "accept",
        },
        Hint {
            key: DISMISS.keys,
            desc: "dismiss",
        },
    ]
}

/// Cells the first two footer hints need (`cap + space + verb`, gaps,
/// margins): the card is never narrower than that.
fn footer_width() -> usize {
    hints()
        .iter()
        .take(2)
        .enumerate()
        .map(|(index, hint)| {
            (if index == 0 { 1 } else { 2 })
                + super::clip::width(hint.key)
                + 2
                + 1
                + super::clip::width(hint.desc)
        })
        .sum::<usize>()
        + 5
}

/// Width from the visible candidates (rule 9): marker, kind column,
/// the widest label plus its detail, the source tag — capped by the
/// card rule and the pane.
fn menu_width(menu: &CompletionMenu<'_>, bounds: Rect) -> u16 {
    let widest = (0..menu.len())
        .filter_map(|index| menu.row(index))
        .map(|row| {
            let detail = row.unsupported.unwrap_or(row.detail);
            super::clip::width(row.label)
                + if detail.is_empty() {
                    0
                } else {
                    2 + super::clip::width(detail)
                }
        })
        .max()
        .unwrap_or(16);
    let content = (4 + 2 + KIND_W + 1 + widest + 2 + SOURCE_W).max(footer_width());
    card_width(content, bounds.width).min(bounds.width)
}

pub(super) fn render(
    editor: &Editor,
    t: &Tokens,
    frame: &mut Frame,
    pane: Rect,
    caret: Option<(u16, u16)>,
) {
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
    let wanted = if menu.is_empty() {
        4
    } else {
        (menu.len() as u16).min(MENU_ROWS) + 2
    };
    let below_first = below >= wanted || below >= above;
    let height = wanted.min(if below_first { below } else { above });
    if height < 3 {
        return;
    }
    let width = menu_width(&menu, bounds);
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
    // the top-right note carries a live action status first, else why
    // the list is partial; the footer is always the whole-chord keys
    let note = menu.action_status().or(if menu.incomplete() {
        Some("incomplete")
    } else if menu.limited() {
        Some("limited")
    } else {
        None
    });
    let hints = hints();
    let inner = Card::new("")
        .note(note)
        .hints(&hints)
        .render(frame, card, t);
    // the card primitive pads one cell; candidate rows own the full inner
    // width so the selected band reaches both borders
    let inner = Rect {
        x: inner.x.saturating_sub(1),
        width: inner.width.saturating_add(2),
        ..inner
    };
    if menu.is_empty() {
        let lines: Vec<_> = menu
            .providers()
            .into_iter()
            .filter(|provider| provider.state != "off")
            .map(|provider| {
                Line::from(vec![
                    Span::styled(
                        format!(" {}  ", provider.source),
                        Style::default().fg(t.source(provider.source)),
                    ),
                    Span::styled(
                        super::clip::clip_end(
                            provider.explanation,
                            usize::from(inner.width.saturating_sub(6)),
                        ),
                        Style::default().fg(t.secondary),
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
        render_row(t, &menu, index, selected == Some(index), frame, row);
    }
    if menu.len() > rows {
        let indicator = format!(
            " {}/{} ",
            selected.map_or(start + 1, |index| index + 1),
            menu.len()
        );
        let cells = super::clip::width(&indicator).min(usize::from(width.saturating_sub(2))) as u16;
        frame.render_widget(
            Paragraph::new(indicator).style(Style::default().fg(t.muted).bg(t.surface)),
            Rect::new(card.right() - cells - 1, card.y, cells, 1),
        );
    }
    if let Some(documentation) = menu.documentation() {
        if let Some(area) = documentation_area(bounds, card, caret_y) {
            render_documentation(t, documentation, frame, area);
        }
    }
}

fn render_row(
    t: &Tokens,
    menu: &CompletionMenu<'_>,
    index: usize,
    selected: bool,
    frame: &mut Frame,
    area: Rect,
) {
    let background = if selected { t.raised } else { t.surface };
    let base = Style::default().fg(t.text).bg(background);
    frame.buffer_mut().set_style(area, base);
    let Some(row) = menu.row(index) else { return };
    let wide = area.width >= 24;
    let kind_width = if wide { KIND_W + 1 } else { 0 };
    let available = usize::from(area.width).saturating_sub(2 + kind_width + SOURCE_W + 2);
    let label_width = if wide { available.min(27) } else { available };
    let detail_width = available.saturating_sub(label_width + 1);
    let label = super::clip::clip_end(row.label, label_width);
    let matched = prefix_end(&label, menu.prefix());
    let mut spans = Vec::with_capacity(9);
    spans.push(row_marker(selected, t).style(base.fg(t.accent)));
    spans.push(Span::styled(" ", base));
    if wide {
        spans.push(Span::styled(
            format!("{:<KIND_W$} ", row.kind),
            base.fg(t.kind(row.kind)),
        ));
    }
    spans.push(Span::styled(
        label[..matched].to_owned(),
        base.fg(t.accent).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(label[matched..].to_owned(), base));
    let label_cells = super::clip::width(&label);
    spans.push(Span::styled(
        " ".repeat(label_width.saturating_sub(label_cells)),
        base,
    ));
    if detail_width > 0 {
        let detail = super::clip::clip_end(row.unsupported.unwrap_or(row.detail), detail_width);
        let padding = detail_width.saturating_sub(super::clip::width(&detail));
        spans.push(Span::styled(" ", base));
        spans.push(Span::styled(detail.into_owned(), base.fg(t.secondary)));
        spans.push(Span::styled(" ".repeat(padding), base));
    }
    spans.push(Span::styled(
        format!(" {:>SOURCE_W$} ", row.source),
        base.fg(t.source(row.source)),
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

fn render_documentation(
    t: &Tokens,
    documentation: CompletionDocumentation<'_>,
    frame: &mut Frame,
    area: Rect,
) {
    let inner = Card::new("documentation")
        .note(documentation.truncated.then_some("truncated"))
        .focused(false)
        .render(frame, area, t);
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
                    t.syntax(syntax),
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
            .style(Style::default().fg(t.text))
            .wrap(Wrap { trim: false }),
        inner,
    );
}
