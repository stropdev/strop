//! The command line (plan 0110 §3.1): while a `: / ? |` line is open it
//! is the bottom row — the modeline's own row. The mode chip keeps its
//! ten cells, the typed line takes the path's place, and the right side
//! says one thing, muted: an error (in error ink), the `:s` summary, the
//! `/` match count, or the command's usage. The menu hangs above the row
//! (`menu.rs`). Nothing here parses or ranks: it reads the engine.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::editor::{Editor, SubstitutePreviewState};

use super::clip::{clip_end, width};
use super::style::Tokens;

mod menu;
#[cfg(test)]
mod tests;

/// Cells between the chip and the line, and before the right side.
const GAP: usize = 2;

/// True while the command line owns the bottom row and the caret.
pub(crate) fn open(editor: &Editor) -> bool {
    !editor.picker_open() && editor.cmdline_sigil().is_some()
}

/// Draw the row (and the menu above it). Returns nothing when no line is
/// open; the modeline draws instead.
pub(super) fn render(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect) {
    if !open(editor) || area.width == 0 || area.height == 0 {
        return;
    }
    let row = Rect {
        y: area.bottom() - 1,
        height: 1,
        ..area
    };
    let cells = usize::from(row.width);
    let (chip, chip_color) = super::statusline::chip::chip(editor, t);
    let chip_cells = width(&chip).min(cells);
    let mut spans = vec![Span::styled(
        clip_end(&chip, chip_cells).into_owned(),
        Style::default()
            .fg(t.on_accent)
            .bg(chip_color)
            .add_modifier(Modifier::BOLD),
    )];
    let line = editor.cmdline();
    let text = line.text();
    let sigil_len = text.chars().next().map_or(0, char::len_utf8);
    let body = &text[sigil_len..];
    let caret = line.cursor().saturating_sub(sigil_len).min(body.len());
    let field_x = (chip_cells + GAP).min(cells);
    let field_cells = cells - field_x;
    // the right side gives way before the typed line is ever cut
    let (note, error) = right_side(editor);
    let typed = 1 + width(body) + 1;
    let note_cells = width(&note);
    let show_note = !note.is_empty() && typed + GAP + note_cells < field_cells;
    let field_width = if show_note {
        field_cells - GAP - note_cells - 1
    } else {
        field_cells
    };
    spans.push(Span::raw(" ".repeat(field_x - chip_cells)));
    let mut caret_x = None;
    if field_width > 0 {
        spans.push(Span::styled(
            text[..sigil_len].to_owned(),
            Style::default().fg(t.accent),
        ));
        let highlights = if matches!(line.sigil(), Some('/' | '?')) {
            strop_picker::query::highlight::buffer_search(body)
        } else {
            Vec::new()
        };
        let field = super::field::project(
            t,
            "",
            body,
            caret,
            true,
            u16::try_from(field_width - 1).unwrap_or(u16::MAX),
            &highlights,
        );
        caret_x = field
            .cursor
            .map(|column| row.x + u16::try_from(field_x + 1).unwrap_or(u16::MAX) + column);
        spans.extend(field.line.spans);
    }
    if show_note {
        let used: usize = spans.iter().map(Span::width).sum();
        let pad = cells.saturating_sub(used + note_cells + 1);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(
            note,
            Style::default().fg(if error { t.error } else { t.muted }),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.surface)),
        row,
    );
    menu::render(editor, t, frame, area, field_x + 1);
    if let Some(x) = caret_x.filter(|&x| x < row.right()) {
        super::frame_capture::place_cursor(frame, (x, row.y));
    }
}

/// What the right side says, and whether it is an error.
fn right_side(editor: &Editor) -> (String, bool) {
    if let Some(state) = editor.substitute_preview() {
        return match state {
            SubstitutePreviewState::Ready(preview) => (preview.summary(), false),
            SubstitutePreviewState::Resolving => ("resolving…".into(), false),
            SubstitutePreviewState::NoTarget => ("pattern not found".into(), false),
            SubstitutePreviewState::Incomplete(_) => (usage(editor), false),
            SubstitutePreviewState::Invalid(error) => (format!("invalid: {error}"), true),
            SubstitutePreviewState::Unavailable(error) => (error, true),
        };
    }
    match editor.cmdline_range() {
        Some(Err(error)) => return (error.to_owned(), true),
        Some(Ok((first, last))) => {
            let count = last + 1 - first;
            let lines = if first == last {
                format!("{}", first + 1)
            } else {
                format!("{}–{}", first + 1, last + 1)
            };
            return (
                format!(
                    "{lines} · {count} line{}",
                    if count == 1 { "" } else { "s" }
                ),
                false,
            );
        }
        _ => {}
    }
    if matches!(editor.cmdline_sigil(), Some('/' | '?')) {
        return match editor.current_search_query() {
            Ok(Some(query)) => match editor.search_summary(&query) {
                Some(Ok(summary)) => (
                    format!(
                        "{} match{}",
                        summary.count,
                        if summary.count == 1 { "" } else { "es" }
                    ),
                    false,
                ),
                Some(Err(error)) => (error.to_string(), true),
                None => ("searching…".into(), false),
            },
            Ok(None) => (String::new(), false),
            Err(error) => (error.to_string(), true),
        };
    }
    (usage(editor), false)
}

fn usage(editor: &Editor) -> String {
    editor.cmdline_usage().unwrap_or_default().to_owned()
}
