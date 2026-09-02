//! Hunk preview card (`Space g p`): the hunk's diff as a floating card.
//! House style: centered card, hint in the bottom border, +/- colors.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::Editor;

use super::{ACCENT, BASE, MUTED, TEXT};

const GREEN: Color = Color::Rgb(0xa9, 0xc4, 0x7c);
const RED: Color = Color::Rgb(0xe8, 0x67, 0x7a);

pub fn render_hunk_card(editor: &Editor, frame: &mut Frame) {
    let Some(hunk) = &editor.hunk_preview else {
        return;
    };
    let area = frame.area();
    let width = (area.width * 60 / 100).clamp(40, area.width.saturating_sub(4));
    let height = ((hunk.lines.len() as u16) + 2).clamp(6, area.height.saturating_sub(4));
    let card = Rect {
        x: (area.width - width) / 2,
        y: (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, card);
    let title = format!(
        " hunk @@ -{},{} +{},{} @@ ",
        hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BASE))
        .title(Span::styled(
            title,
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " any key dismisses · g u undo · g s stage ",
            Style::default().fg(MUTED),
        ));
    let inner = block.inner(card);
    let inner = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };
    frame.render_widget(&block, card);

    let lines: Vec<Line> = hunk
        .lines
        .iter()
        .map(|l| {
            let style = match l.as_bytes().first() {
                Some(b'+') => Style::default().fg(GREEN),
                Some(b'-') => Style::default().fg(RED),
                _ => Style::default().fg(MUTED),
            };
            Line::from(Span::styled(l.clone(), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
