//! The chrome primitives (0088 §2 rules 3, 5, 6, 9, 10): one card, one
//! keycap, one selected-row idiom, one hint footer. Every floating
//! surface draws through here, so a border, a title or a keycap looks
//! the same wherever it appears.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use ratatui::Frame;

use super::clip::{clip_end, width};
use super::style::Tokens;

/// A whole chord and what it does: `enter  open`. Hints render as
/// keycaps and are dropped whole when they do not fit (rule 10).
#[derive(Clone, Copy)]
pub(crate) struct Hint<'a> {
    pub key: &'a str,
    pub desc: &'a str,
}

/// A rounded card on the surface plane: title top-left in `fg.text`
/// bold, an optional note top-right in `fg.muted`, keycap hints in the
/// bottom border. Focus is the border going from `fg.faint` to `fg.muted`.
pub(crate) struct Card<'a> {
    pub title: &'a str,
    pub note: Option<&'a str>,
    pub hints: &'a [Hint<'a>],
    pub focused: bool,
}

impl<'a> Card<'a> {
    pub(crate) fn new(title: &'a str) -> Self {
        Self {
            title,
            note: None,
            hints: &[],
            focused: true,
        }
    }

    pub(crate) fn note(mut self, note: Option<&'a str>) -> Self {
        self.note = note;
        self
    }

    pub(crate) fn hints(mut self, hints: &'a [Hint<'a>]) -> Self {
        self.hints = hints;
        self
    }

    pub(crate) fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Clear `area`, draw the card and return its content rect: one
    /// border cell plus one cell of horizontal padding (0001 §4: cards
    /// breathe). Title and note clip whole graphemes; hints drop whole.
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect, t: &Tokens) -> Rect {
        frame.render_widget(Clear, area);
        let border = if self.focused { t.muted } else { t.faint };
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border))
            .style(Style::default().bg(t.surface));
        let room = usize::from(area.width.saturating_sub(4));
        let note_cells = self.note.map_or(0, |note| width(note) + 2);
        if !self.title.is_empty() {
            let title = clip_end(self.title, room.saturating_sub(note_cells).max(1));
            block = block.title_top(Line::from(Span::styled(
                format!(" {title} "),
                Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            )));
        }
        if let Some(note) = self.note.filter(|note| !note.is_empty()) {
            let note = clip_end(note, room.saturating_sub(width(self.title) + 2).max(1));
            block = block.title_top(
                Line::from(Span::styled(
                    format!(" {note} "),
                    Style::default().fg(t.muted),
                ))
                .right_aligned(),
            );
        }
        let footer = hint_spans(self.hints, room, t);
        if !footer.is_empty() {
            block = block.title_bottom(Line::from(footer));
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);
        Rect {
            x: inner.x.saturating_add(1),
            width: inner.width.saturating_sub(2),
            ..inner
        }
    }
}

/// The only chip: ` key ` on the raised plane, accent bold.
pub(crate) fn keycap(key: &str, t: &Tokens) -> Span<'static> {
    Span::styled(
        format!(" {key} "),
        Style::default()
            .fg(t.accent)
            .bg(t.raised)
            .add_modifier(Modifier::BOLD),
    )
}

/// Keycap + description groups that fit in `cells`, whole groups only
/// (rule 10): `[ enter ] open   [ esc ] close`. Each group costs
/// `key + 2` for the cap, one space, the description, and two spaces
/// of gap before the next. Returns nothing when not even the first fits.
pub(crate) fn hint_spans(hints: &[Hint<'_>], cells: usize, t: &Tokens) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (index, hint) in hints.iter().enumerate() {
        let gap = if index == 0 { 1 } else { 2 };
        let cost = gap + width(hint.key) + 2 + 1 + width(hint.desc);
        if used + cost + 1 > cells {
            break;
        }
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(keycap(hint.key, t));
        spans.push(Span::styled(
            format!(" {}", hint.desc),
            Style::default().fg(t.muted),
        ));
        used += cost;
    }
    if !spans.is_empty() {
        spans.push(Span::raw(" "));
    }
    spans
}

/// Width from content (rule 9): capped at 60 % of the columns, never
/// narrower than 32 — unless the terminal itself is narrower.
pub(crate) fn card_width(content: usize, columns: u16) -> u16 {
    let cap = (u32::from(columns) * 60 / 100) as u16;
    let wanted = u16::try_from(content).unwrap_or(u16::MAX);
    wanted.min(cap).max(32).min(columns.saturating_sub(2))
}

/// The one selected-row idiom (rule 5): full-row raised background with
/// `▌` in accent at column 0; unselected rows get a blank marker cell.
pub(crate) fn row_marker(selected: bool, t: &Tokens) -> Span<'static> {
    if selected {
        Span::styled("▌", Style::default().fg(t.accent).bg(t.raised))
    } else {
        Span::raw(" ")
    }
}

/// Paint `line` across `width` cells with the selected-row background
/// when `selected`, padding the tail so the band covers the whole row.
pub(crate) fn band(
    mut line: Line<'static>,
    width: u16,
    selected: bool,
    t: &Tokens,
) -> Line<'static> {
    let used = line.width();
    let pad = usize::from(width).saturating_sub(used);
    if pad > 0 {
        line.spans.push(Span::raw(" ".repeat(pad)));
    }
    if selected {
        for span in &mut line.spans {
            span.style = span.style.bg(t.raised);
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::theme::ThemeId;

    #[test]
    fn hints_drop_whole_groups_never_mid_chord() {
        let t = Tokens::new(ThemeId::Hone);
        let hints = [
            Hint {
                key: "enter",
                desc: "open",
            },
            Hint {
                key: "ctrl-o",
                desc: "collect",
            },
            Hint {
                key: "esc",
                desc: "close",
            },
        ];
        let text = |cells: usize| {
            hint_spans(&hints, cells, &t)
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        assert_eq!(text(80), "  enter  open   ctrl-o  collect   esc  close ");
        assert_eq!(text(32), "  enter  open   ctrl-o  collect ");
        assert_eq!(text(31), "  enter  open ");
        assert_eq!(text(14), "  enter  open ");
        assert_eq!(text(10), "");
        for cells in 0..80 {
            assert!(width(&text(cells)) <= cells);
        }
    }

    #[test]
    fn card_width_follows_content_within_the_caps() {
        assert_eq!(card_width(10, 140), 32);
        assert_eq!(card_width(50, 140), 50);
        assert_eq!(card_width(500, 140), 84);
        assert_eq!(card_width(50, 40), 32);
        assert_eq!(card_width(50, 20), 18);
    }
}
