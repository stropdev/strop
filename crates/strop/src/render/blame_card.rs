//! Blame card (`Space g B`): the commit card for the cursor line.
//! Enter dives into the commit browser; anything else dismisses.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::editor::Editor;

use super::card::{card_width, Card, Hint};
use super::style::Tokens;

const HINTS: &[Hint<'static>] = &[
    Hint {
        key: "enter",
        desc: "dive",
    },
    Hint {
        key: "any key",
        desc: "dismiss",
    },
];

pub fn render_blame_card(editor: &Editor, t: &Tokens, frame: &mut Frame) {
    if editor.input_owner() != crate::editor::InputOwner::BlameCard {
        return;
    }
    let Some(card) = editor.blame_card() else {
        return;
    };
    let area = frame.area();
    let author_age_raw = format!("{} · {} ago", card.author, card.age);
    let content =
        super::clip::width(&card.summary).max(super::clip::width(&author_age_raw) + 10) + 6;
    let width = card_width(content, area.width);
    // 0068 C4: fit the card to its content (a fixed 7 rows left dead
    // space under three lines), never a cut mid-word summary.
    let content_budget = usize::from(width.saturating_sub(5));
    let author_age = super::clip::clip_end(&author_age_raw, content_budget);
    let summary = super::clip::clip_end(&card.summary, content_budget);
    let height = 5u16.min(area.height.saturating_sub(4)).max(3);
    let rect = Rect {
        x: (area.width - width) / 2,
        y: (area.height - height) / 3,
        width,
        height,
    };
    let title = format!("blame · line {}", card.line);
    let inner = Card::new(&title).hints(HINTS).render(frame, rect, t);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", card.short_sha),
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {author_age}"), Style::default().fg(t.muted)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            summary.into_owned(),
            Style::default().fg(t.text),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
