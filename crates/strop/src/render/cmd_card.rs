//! The command/search card (noice.nvim lineage): `:` and `/` float as a
//! top-center card instead of squatting in the statusline — what you're
//! typing deserves focus. Match count rides along on search.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::Editor;

use super::{accent, base, muted, text};

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
    let pending = editor.pending().text();

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
        .border_style(Style::default().fg(muted()))
        .style(Style::default().bg(base()))
        .title(Span::styled(
            title,
            Style::default().fg(text()).add_modifier(Modifier::BOLD),
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
        .pending()
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

    // search rides with a live match count — right-aligned inside the
    // field's width instead of appended past it (0068 C2: inline
    // append truncated hard)
    if matches!(kind, '/' | '?') {
        let label = match editor.current_search_query() {
            Ok(Some(query)) => match editor.search_summary(&query) {
                Some(Ok(summary)) => format!(
                    "{} match{}",
                    summary.count,
                    if summary.count == 1 { "" } else { "es" }
                ),
                Some(Err(error)) => error.to_string(),
                None => "searching…".into(),
            },
            Ok(None) => "0 matches".into(),
            Err(error) => error.to_string(),
        };
        let label = super::clip::clip_end(&label, usize::from(text_area.width) / 3);
        let label_w = super::clip::width(&label);
        let used: usize = spans.iter().map(|span| span.width()).sum();
        let pad = usize::from(text_area.width).saturating_sub(used + label_w + 1);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(
            label.into_owned(),
            Style::default().fg(muted()),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), text_area);

    // completion rows: first candidate accent (Tab cycles to it), the
    // rest muted with their doc strings. The name column sizes to the
    // shown candidates (0068 C2: a fixed 10 overflowed into the doc
    // column), names and docs ellipsize — nothing touches the border.
    let body_str = pending.strip_prefix(':').unwrap_or("");
    let shown: Vec<_> = candidates.iter().take(6).collect();
    let name_w = shown
        .iter()
        .map(|(name, _)| super::clip::width(name))
        .max()
        .unwrap_or(4)
        .clamp(4, 16);
    for (i, (name, doc)) in shown.iter().enumerate() {
        let y = text_area.y + 1 + i as u16;
        let row = Rect {
            y,
            height: 1,
            ..text_area
        };
        let (name_fg, doc_fg) = if i == 0 {
            (accent(), text())
        } else {
            (text(), muted())
        };
        let marker = if name == &body_str { "▌" } else { " " };
        let doc_budget = usize::from(text_area.width).saturating_sub(1 + name_w + 2 + 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(accent())),
                Span::styled(
                    format!("{:<name_w$}  ", super::clip::clip_end(name, name_w)),
                    Style::default().fg(name_fg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    super::clip::clip_end(doc, doc_budget).into_owned(),
                    Style::default().fg(doc_fg),
                ),
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
