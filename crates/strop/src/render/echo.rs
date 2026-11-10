//! The echo row (0088 §2): one row above the statusline, present only
//! while a message is live, so feedback can never hide an error in a
//! shared slot (0087 U1). A long message wraps to at most three rows
//! (B5) and clips after that; `:messages` keeps every message whole.
//! Precedence: the response to the last keypress
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
use super::clip::{width, wrap};
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

/// The most rows a message wraps to before it clips (`:messages` keeps
/// the whole text).
const MAX_ROWS: usize = 3;

/// The echo laid out for one width: wrapped body rows and the hint that
/// rides the last row.
struct Layout {
    echo: Echo,
    rows: Vec<String>,
    hint: Vec<Span<'static>>,
    hint_w: usize,
}

fn layout(editor: &Editor, t: &Tokens, cells: usize) -> Option<Layout> {
    // the picker owns the frame, and in terminal input the child owns
    // every pane row — its own last line must stay visible
    if editor.picker_open() || editor.terminal_input_active() {
        return None;
    }
    let echo = gather(editor, t)?;
    let text = strop_core::layout::printable_text(echo.text.replace('\n', " · "));
    let hint = echo
        .hint
        .map(|hint| hint_spans(&[hint], cells / 2, t))
        .unwrap_or_default();
    let hint_w: usize = hint.iter().map(|span| width(&span.content)).sum();
    let rows = wrap(&text, cells.saturating_sub(3 + hint_w), MAX_ROWS);
    Some(Layout {
        echo,
        rows,
        hint,
        hint_w,
    })
}

/// How many rows the echo paints this frame (0 when no message is live):
/// cards above the statusline make room for them.
pub(super) fn rows(editor: &Editor, t: &Tokens, columns: u16) -> u16 {
    layout(editor, t, usize::from(columns)).map_or(0, |layout| layout.rows.len().max(1) as u16)
}

/// Paint the rows above the statusline. They overlay the last pane rows,
/// so when the cursor line falls under them they step up above it: what
/// you are looking at stays visible while a message is live.
pub(super) fn render(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect, pane_area: Rect) {
    if area.width < 8 || area.height < 4 || editor.panes().is_empty() {
        return;
    }
    let cells = usize::from(area.width);
    let Some(layout) = layout(editor, t, cells) else {
        return;
    };
    // never taller than the pane rows above the statusline, minus one
    // row for the cursor line to stay visible
    let fits = usize::from(area.height - 2);
    let count = layout.rows.len().clamp(1, fits) as u16;
    let mut bottom = area.bottom() - 2;
    let caret = super::buffer::caret_position(
        editor,
        pane_area,
        editor.current(),
        editor.head(),
        editor.view_top(),
        editor.view().hscroll,
    );
    if let Some((_, row)) = caret {
        if row <= bottom && row + count > bottom {
            bottom = row.saturating_sub(1).max(area.y + count - 1);
        }
    }
    let top = bottom + 1 - count;
    let style = Style::default().fg(layout.echo.color);
    for index in 0..count {
        let body = layout
            .rows
            .get(usize::from(index))
            .cloned()
            .unwrap_or_default();
        let last = index + 1 == count;
        let lead = if index == 0 {
            format!(" {} ", layout.echo.glyph)
        } else {
            "   ".into()
        };
        let pad = cells.saturating_sub(3 + width(&body) + if last { layout.hint_w } else { 0 });
        let mut spans = vec![
            Span::styled(lead, style.add_modifier(Modifier::BOLD)),
            Span::styled(body, style),
            Span::raw(" ".repeat(pad)),
        ];
        if last {
            spans.extend(layout.hint.iter().cloned());
        }
        let rect = Rect {
            y: top + index,
            height: 1,
            ..area
        };
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(t.surface)),
            rect,
        );
    }
}
