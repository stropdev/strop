//! Blame card (`Space g b`): the commit card for the cursor line.
//! Enter dives into the commit browser; anything else dismisses.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::Editor;

use super::{accent, base, muted, text};

pub fn render_blame_card(editor: &Editor, frame: &mut Frame) {
    if editor.input_owner() != crate::editor::InputOwner::BlameCard {
        return;
    }
    let Some(card) = editor.blame_card() else {
        return;
    };
    let area = frame.area();
    let width = ((u32::from(area.width) * 55 / 100) as u16)
        .max(46)
        .min(area.width.saturating_sub(4));
    // 0068 C4: fit the card to its content (a fixed 7 rows left dead
    // space under three lines), never a cut mid-word summary.
    let content_budget = usize::from(width.saturating_sub(5));
    let author_age_raw = format!("{} · {} ago", card.author, card.age);
    let author_age = super::clip::clip_end(&author_age_raw, content_budget);
    let summary = super::clip::clip_end(&card.summary, content_budget);
    let height = 5u16.min(area.height.saturating_sub(4)).max(3);
    let rect = Rect {
        x: (area.width - width) / 2,
        y: (area.height - height) / 3,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted()))
        .style(Style::default().bg(base()))
        .title(Span::styled(
            format!(" blame · line {} ", card.line),
            Style::default().fg(text()).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " enter dive · any key dismisses ",
            Style::default().fg(muted()),
        ));
    frame.render_widget(&block, rect);
    let inner = block.inner(rect);
    let inner = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", card.short_sha),
                Style::default()
                    .fg(base())
                    .bg(accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {author_age}"), Style::default().fg(muted())),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!(" {summary}"),
            Style::default().fg(text()),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
