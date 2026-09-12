//! The command/search card (noice.nvim lineage): `:` and `/` float as a
//! top-center card instead of squatting in the statusline — what you're
//! typing deserves focus. Match count rides along on search.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::Editor;

use super::{ACCENT, BASE, MUTED, TEXT};

pub fn render_cmd_card(editor: &Editor, frame: &mut Frame) {
    // `?` is a first-class search sigil here (the old detection missed
    // it: backward searches never got the card)
    let Some(kind) = editor.pending_sigil() else {
        return;
    };
    if !matches!(kind, ':' | '/' | '?' | '|') {
        return;
    }
    if editor.picker_open() {
        return;
    }
    let pending = editor.pending.text();

    let area = frame.area();
    // ex completion rides along: candidates under the input (0003 §1)
    let candidates = if kind == ':' {
        editor.ex_candidates()
    } else {
        Vec::new()
    };
    let width = ((u32::from(area.width) * 50 / 100) as u16)
        .max(30)
        .min(area.width.saturating_sub(4));
    let height = (3 + candidates.len().min(6) as u16).min(area.height.saturating_sub(1));
    let card = Rect {
        x: (area.width.saturating_sub(width)) / 2,
        y: 1,
        width,
        height,
    };
    frame.render_widget(Clear, card);
    let title = match kind {
        ':' => " command ",
        '|' => " pipe ",
        _ => " search ",
    };
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

    // body: the payload, not the prefix — ":w" shows "w", "/foo" shows "foo"
    let body = pending.strip_prefix(kind).unwrap_or(pending);
    let text_area = Rect {
        x: inner.x.saturating_add(1),
        y: inner.y,
        width: inner.width.saturating_sub(1),
        height: inner.height.min(1),
    };
    let caret_byte = editor
        .pending
        .cursor()
        .saturating_sub(kind.len_utf8())
        .min(body.len());
    let field = super::field::project(
        &kind.to_string(),
        body,
        caret_byte,
        true,
        text_area.width,
        &[],
    );
    let caret = field.cursor;
    let mut spans = field.line.spans;

    // search rides with a live match count
    if matches!(kind, '/' | '?') {
        let label = match editor.current_search_query() {
            Ok(Some(query)) => match editor.search_summary(&query) {
                Some(Ok(summary)) => format!(
                    "   {} match{}",
                    summary.count,
                    if summary.count == 1 { "" } else { "es" }
                ),
                Some(Err(error)) => format!("   {error}"),
                None => "   searching…".into(),
            },
            Ok(None) => "   0 matches".into(),
            Err(error) => format!("   {error}"),
        };
        spans.push(Span::styled(label, Style::default().fg(MUTED)));
    }

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

    if let Some(column) = caret {
        if column < text_area.width && text_area.height != 0 {
            crate::render::frame_capture::place_cursor(frame, (text_area.x + column, text_area.y));
        }
    }
}
