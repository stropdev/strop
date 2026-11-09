//! The echo row (0088 §2): one row above the statusline, present only
//! while a message is live, so feedback can never hide an error in a
//! shared slot (0087 U1). Precedence: the response to the last keypress
//! first — `Editor::message` is cleared on every key, so when it is set
//! it is fresh by construction — then retained feedback by severity
//! (failure > warning > info), then I/O activity. Errors stay until the
//! next keypress; the engine does not yet date its records, so retained
//! feedback stays until it is dismissed or resolved (`Space a`).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use strop_engine::editor::feedback::FeedbackSeverity;

use crate::editor::Editor;

use super::card::{hint_spans, Hint};
use super::clip::{clip_end, width};
use super::style::Tokens;

struct Echo {
    glyph: &'static str,
    color: Color,
    text: String,
    hint: Option<Hint<'static>>,
}

fn gather(editor: &Editor, t: &Tokens) -> Option<Echo> {
    if !editor.message().is_empty() {
        return Some(Echo {
            glyph: "›",
            color: t.text,
            text: editor.message().to_owned(),
            hint: None,
        });
    }
    if let Some(record) = editor.presented_feedback() {
        let (glyph, color) = match record.severity() {
            FeedbackSeverity::Failure => ("✗", t.error),
            FeedbackSeverity::Warning => ("▲", t.warning),
            FeedbackSeverity::Info => ("●", t.info),
        };
        return Some(Echo {
            glyph,
            color,
            text: record.summary().to_owned(),
            hint: Some(if editor.presented_feedback_has_actions() {
                Hint {
                    key: "space a",
                    desc: "actions",
                }
            } else {
                Hint {
                    key: ":explain",
                    desc: "why",
                }
            }),
        });
    }
    editor.io_status().map(|status| Echo {
        glyph: "…",
        color: t.muted,
        text: status.to_owned(),
        hint: None,
    })
}

/// Whether the row will paint this frame (cards above the statusline
/// make room for it).
pub(super) fn visible(editor: &Editor) -> bool {
    // the picker owns the frame, and in terminal input the child owns
    // every pane row — its own last line must stay visible
    !editor.picker_open()
        && !editor.terminal_input_active()
        && (!editor.message().is_empty()
            || editor.presented_feedback().is_some()
            || editor.io_status().is_some())
}

/// Paint the row above the statusline. The row is an overlay on the
/// last pane row, so it steps up one row rather than cover the cursor
/// line: what you are looking at stays visible while a message is live.
pub(super) fn render(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect, pane_area: Rect) {
    if area.width < 8 || area.height < 4 || editor.panes().is_empty() || !visible(editor) {
        return;
    }
    let Some(echo) = gather(editor, t) else {
        return;
    };
    let mut y = area.bottom() - 2;
    let caret = super::buffer::caret_position(
        editor,
        pane_area,
        editor.current(),
        editor.head(),
        editor.view_top(),
        editor.view().hscroll,
    );
    if caret.is_some_and(|(_, row)| row == y) {
        y -= 1;
    }
    let cells = usize::from(area.width);
    let text = strop_core::layout::printable_text(echo.text.replace('\n', " · "));
    let hint = echo
        .hint
        .map(|hint| hint_spans(&[hint], cells / 2, t))
        .unwrap_or_default();
    let hint_w: usize = hint.iter().map(|span| width(&span.content)).sum();
    let body_budget = cells.saturating_sub(3 + hint_w);
    let body = clip_end(&text, body_budget);
    let pad = cells.saturating_sub(3 + width(&body) + hint_w);
    let mut spans = vec![
        Span::styled(
            format!(" {} ", echo.glyph),
            Style::default().fg(echo.color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(body.into_owned(), Style::default().fg(echo.color)),
        Span::raw(" ".repeat(pad)),
    ];
    spans.extend(hint);
    let rect = Rect {
        y,
        height: 1,
        ..area
    };
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.surface)),
        rect,
    );
}
