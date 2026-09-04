//! The command/search card (noice.nvim lineage): `:` and `/` float as a
//! top-center card instead of squatting in the statusline — what you're
//! typing deserves focus. Match count rides along on search.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use strop_grammar as grammar;

use crate::editor::Editor;

use super::{ACCENT, BASE, MUTED, TEXT};

pub fn render_cmd_card(editor: &Editor, frame: &mut Frame) {
    let pending = &editor.pending;
    let kind = if pending.starts_with(':') {
        ':'
    } else if pending.contains('/') && !pending.is_empty() {
        '/'
    } else {
        return;
    };
    if editor.picker_open() {
        return;
    }

    let area = frame.area();
    // ex completion rides along: candidates under the input (0003 §1)
    let candidates = if kind == ':' {
        editor.ex_candidates()
    } else {
        Vec::new()
    };
    let width = (area.width * 50 / 100).clamp(30, area.width.saturating_sub(4));
    let height = 3 + candidates.len().min(6) as u16;
    let card = Rect {
        x: (area.width - width) / 2,
        y: area.height / 6,
        width,
        height,
    };
    frame.render_widget(Clear, card);
    let title = if kind == ':' { " command " } else { " search " };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BASE))
        .title(Span::styled(
            title,
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(&block, card);
    let inner = block.inner(card);

    // body: the payload, not the prefix — ":w" shows "w", "d/foo" shows "foo"
    let body = match kind {
        ':' => pending.strip_prefix(':').unwrap_or(pending),
        '/' => pending.split_once('/').map(|(_, p)| p).unwrap_or(pending),
        _ => pending.as_str(),
    };
    let mut spans = vec![
        Span::styled(
            format!("{kind} "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(body.to_string(), Style::default().fg(TEXT)),
        Span::styled("▏", Style::default().fg(ACCENT)),
    ];

    // search rides with a live match count
    if kind == '/' {
        if let Some(idx) = pending.find('/') {
            let pat = &pending[idx + 1..];
            if !pat.is_empty() {
                let n = grammar::search_all(editor.buf(), pat).len();
                spans.push(Span::styled(
                    format!("   {n} match{}", if n == 1 { "" } else { "es" }),
                    Style::default().fg(MUTED),
                ));
            }
        }
    }

    let text_area = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(1),
        height: 1,
    };
    frame.render_widget(Paragraph::new(Line::from(spans)), text_area);

    // completion rows: first candidate accent (Tab cycles to it), the
    // rest muted with their doc strings
    let body_str = pending.strip_prefix(':').unwrap_or("");
    for (i, (name, doc)) in candidates.iter().take(6).enumerate() {
        let y = text_area.y + 1 + i as u16;
        let row = Rect {
            y,
            height: 1,
            ..text_area
        };
        let (name_fg, doc_fg) = if i == 0 {
            (ACCENT, TEXT)
        } else {
            (TEXT, MUTED)
        };
        let marker = if name == &body_str { "▌" } else { " " };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(ACCENT)),
                Span::styled(
                    format!("{name:<10}"),
                    Style::default().fg(name_fg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(doc.to_string(), Style::default().fg(doc_fg)),
            ])),
            row,
        );
    }

    // caret goes in the card, not the buffer
    let caret_x = text_area.x + 2 + body.chars().count() as u16;
    if caret_x < text_area.x + text_area.width {
        frame.set_cursor_position((caret_x, text_area.y));
    }
}
