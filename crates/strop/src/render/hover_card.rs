//! Hover card (`Space k`): the server's hover text as a floating card.
//! Any key dismisses.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::editor::Editor;

use super::{base, muted};

pub fn render_hover_card(editor: &Editor, frame: &mut Frame) {
    if editor.input_owner() != crate::editor::InputOwner::HoverCard {
        return;
    }
    let Some(text) = editor.hover_card() else {
        return;
    };
    let area = frame.area();
    let width = ((u32::from(area.width) * 60 / 100) as u16)
        .max(40)
        .min(area.width.saturating_sub(4));
    // 0068 C3: size from the WRAPPED row count — counting source lines
    // while Wrap is on silently cut wrapped content at the bottom border.
    let inner_width = width.saturating_sub(4).max(1);
    let lines = wrapped_rows(text, inner_width) as u16;
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
        .border_style(Style::default().fg(muted()))
        .style(Style::default().bg(base()))
        .title(Span::styled(
            " hover ",
            Style::default()
                .fg(crate::render::text())
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " enter full documentation · other key dismisses ",
            Style::default().fg(muted()),
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
        Paragraph::new(text)
            .style(Style::default().fg(crate::render::text()))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

/// Rows `text` occupies under ratatui's `Wrap { trim: false }` at
/// `width` cells: greedy fill, breaking at the last space when one
/// exists, hard-breaking words longer than the width — never an
/// undercount (an over-tall card clamps at the area bound; a cut
/// loses content).
fn wrapped_rows(text: &str, width: u16) -> usize {
    let width = usize::from(width).max(1);
    text.lines()
        .map(|line| {
            let mut rows = 1usize;
            let mut used = 0usize;
            let mut last_space = 0usize; // `used` before the last space
            for grapheme in unicode_segmentation::UnicodeSegmentation::graphemes(line, true) {
                let w = super::clip::width(grapheme);
                if used + w > width {
                    rows += 1;
                    if grapheme.trim().is_empty() {
                        // the space stays trailing on the previous row
                        used = 0;
                    } else if last_space > 0 {
                        // break at the space: the next row starts with
                        // the content after it plus this grapheme
                        used = used - last_space - 1 + w;
                        last_space = 0;
                    } else {
                        used = w;
                    }
                    while used > width {
                        rows += 1;
                        used -= width;
                    }
                    continue;
                }
                if grapheme.trim().is_empty() {
                    last_space = used;
                }
                used += w;
            }
            rows
        })
        .sum()
}

#[cfg(test)]
mod tests {
    /// The estimator never undercounts vs the real widget: render the
    /// text and compare against the last non-empty row in the buffer.
    #[test]
    fn wrapped_rows_never_undercounts_the_real_widget() {
        use ratatui::backend::TestBackend;
        use ratatui::widgets::{Paragraph, Wrap};
        use ratatui::Terminal;
        let samples = [
            "short",
            "a much longer hover line that must wrap at word boundaries instead of cutting mid-word",
            "an_extremely_long_identifier_without_any_spaces_that_forces_a_hard_break_somewhere",
            "mixed words an_extremely_long_identifier_token and more prose after it",
            "line one\n\nline three after a blank",
        ];
        for sample in samples {
            for width in [20u16, 37, 52] {
                let mut terminal = Terminal::new(TestBackend::new(width, 60)).unwrap();
                terminal
                    .draw(|f| {
                        f.render_widget(Paragraph::new(sample).wrap(Wrap { trim: false }), f.area())
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let actual = buffer
                    .content
                    .chunks(width as usize)
                    .take_while(|row| row.iter().any(|cell| cell.symbol() != " "))
                    .count();
                let estimate = super::wrapped_rows(sample, width);
                assert!(
                    estimate >= actual,
                    "{sample:?} at {width}: estimate {estimate} < actual {actual}"
                );
            }
        }
    }
}
