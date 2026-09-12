//! Hover card (`Space k`): the server's hover text as a floating card.
//! Any key dismisses.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::editor::Editor;

use super::{ACCENT, BASE, MUTED, TEXT};

pub fn render_hover_card(editor: &Editor, frame: &mut Frame) {
    if editor.input_owner() != crate::editor::InputOwner::HoverCard {
        return;
    }
    let Some(text) = &editor.hover_card else {
        return;
    };
    let area = frame.area();
    let width = ((u32::from(area.width) * 60 / 100) as u16)
        .max(40)
        .min(area.width.saturating_sub(4));
    let lines = text.lines().take(usize::from(area.height)).count() as u16;
    let height = lines
        .saturating_add(2)
        .max(5)
        .min(area.height.saturating_sub(4));
    let card = Rect {
        x: (area.width - width) / 2,
        y: (area.height / 4).min(area.height.saturating_sub(height)),
        width,
        height,
    };
    frame.render_widget(Clear, card);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BASE))
        .title(Span::styled(
            " hover ",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " enter full documentation · other key dismisses ",
            Style::default().fg(MUTED),
        ));
    frame.render_widget(&block, card);
    let inner = block.inner(card);
    let inner = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };
    frame.render_widget(
        Paragraph::new(text.clone())
            .style(Style::default().fg(TEXT))
            .wrap(Wrap { trim: false }),
        inner,
    );
}
