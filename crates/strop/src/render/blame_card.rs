//! Blame card (`Space g b`): the commit card for the cursor line.
//! Enter dives into the commit browser; anything else dismisses.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::Editor;

use super::{ACCENT, BASE, MUTED, TEXT};

pub fn render_blame_card(editor: &Editor, frame: &mut Frame) {
    let Some(card) = &editor.blame_card else {
        return;
    };
    let area = frame.area();
    let width = (area.width * 55 / 100).clamp(46, area.width.saturating_sub(4));
    let height = 7u16.min(area.height.saturating_sub(4));
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
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BASE))
        .title(Span::styled(
            format!(" blame · line {} ", card.line),
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " enter dive · any key dismisses ",
            Style::default().fg(MUTED),
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
                    .fg(BASE)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} · {} ago", card.author, card.age),
                Style::default().fg(MUTED),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!(" {}", card.summary),
            Style::default().fg(TEXT),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
