//! Diff surface rendering (0010 §4/§5): gutters, row backgrounds,
//! structural rows. All decoration comes from typed hunk data —
//! never from sniffing `+`/`-` in the text. Anatomy borrowed from
//! tuicr (`[sign][old][new][content]`, quiet bands for structural
//! rows), tuned to the strop palette. Log/file row decoration lives
//! in `diff/list`; the blame column and commit file sidebar in
//! `diff/margins` (0011/0032).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::editor::{DiffRow, Surface};
use strop_git::{DiffLine, LineOrigin};

use super::style::Tokens;

mod list;
mod margins;
mod status;

pub(crate) use list::{surface_list_row, RowDecoration};
pub(crate) use margins::{blame_blank, blame_spans, sidebar_row_spans};

pub(crate) fn origin_fg(t: &Tokens, origin: LineOrigin) -> Color {
    match origin {
        LineOrigin::Addition => t.diff_add,
        LineOrigin::Deletion => t.diff_del,
        LineOrigin::Context => t.text,
    }
}

/// Quiet full-row backgrounds for add/del rows — a visible scan signal
/// that doesn't shout (tuicr's two-tier idea: quiet under content, loud
/// markers); context rows stay on the base plane.
pub(crate) fn origin_bg(t: &Tokens, origin: LineOrigin) -> Option<Color> {
    match origin {
        LineOrigin::Addition => Some(t.diff_add_bg),
        LineOrigin::Deletion => Some(t.diff_del_bg),
        LineOrigin::Context => None,
    }
}

/// Intra-line emphasis (delta-style): the byte range within this row's
/// text that actually changed, paired against the opposite-side row at
/// the same index in the hunk's del/add run. None for context rows and
/// unmatched rows (pure adds/deletes emphasize the whole line).
pub(crate) fn emphasis_span(surface: Option<&Surface>, row: usize) -> Option<(usize, usize)> {
    let Some(Surface::Diff { hunks, .. }) = surface else {
        return None;
    };
    hunks.emphasis(row)
}

/// The diff gutter for one content row: origin marker + both sides'
/// numbers, right-aligned, absent side blank (never `0`). The cursor
/// row's numbers light up like the standard gutter's do.
pub(crate) fn diff_gutter(
    t: &Tokens,
    line: &DiffLine,
    is_cursor_row: bool,
    digits: usize,
) -> Vec<Span<'static>> {
    // the sign column keeps one vocabulary (rule 5): `▎` in the origin
    // colour; the cursor row lights its numbers in the accent instead
    let (marker, color) = match line.origin {
        LineOrigin::Addition => ("▎", t.diff_add),
        LineOrigin::Deletion => ("▎", t.diff_del),
        LineOrigin::Context => (" ", t.muted),
    };
    let number = |n: Option<usize>| {
        let style = if is_cursor_row {
            Style::default().fg(t.accent)
        } else {
            Style::default().fg(t.muted)
        };
        let text = match n {
            Some(n) => format!("{:>width$}", n, width = digits),
            None => " ".repeat(digits),
        };
        Span::styled(text, style)
    };
    vec![
        Span::styled(
            marker,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        number(line.old_lineno),
        Span::styled(" ", Style::default()),
        number(line.new_lineno),
        Span::styled(" ", Style::default()),
    ]
}

/// Stats and hunk-header rows: a quiet band, label/stats left, nothing
/// loud (0010 §4). No width padding here — the row is ordinary content
/// that scrolls horizontally and gets its band from RowStyle.row_bg;
/// the caller prepends the fixed number-gutter cells.
pub(crate) fn structural_row(t: &Tokens, surface: &Surface, row: usize) -> Line<'static> {
    match (surface, row) {
        (Surface::Diff { hunks, .. }, 0) => Line::from(vec![
            Span::styled(
                hunks.label().to_owned(),
                Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" +{} ", hunks.added()),
                Style::default().fg(t.diff_add).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("-{}", hunks.deleted()),
                Style::default().fg(t.diff_del).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(t.surface)),
        (Surface::Diff { .. }, _) => surface
            .diff_row(row)
            .and_then(|row| match row {
                DiffRow::HunkHeader(hunk) => Some(hunk_header_spans(t, hunk)),
                _ => None,
            })
            .map(|spans| Line::from(spans).style(Style::default().bg(t.surface)))
            .unwrap_or_default(),
        _ => Line::default(),
    }
}

/// The hunk-header band: `@@` quiet, the old side in the deletion
/// color, the new side in the addition color — the range reads
/// structurally without shouting. The spans concatenate to exactly
/// `Hunk::header()`, the string the buffer row holds.
fn hunk_header_spans(t: &Tokens, hunk: &strop_git::Hunk) -> Vec<Span<'static>> {
    vec![
        Span::styled("@@ ", Style::default().fg(t.muted)),
        Span::styled(
            format!("-{},{}", hunk.old_start, hunk.old_count),
            Style::default().fg(t.diff_del),
        ),
        Span::styled(" ", Style::default().fg(t.muted)),
        Span::styled(
            format!("+{},{}", hunk.new_start, hunk.new_count),
            Style::default().fg(t.diff_add),
        ),
        Span::styled(" @@", Style::default().fg(t.muted)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_header_spans_keep_the_buffer_rows_bytes() {
        let hunk = strop_git::Hunk {
            kind: strop_git::HunkKind::Change,
            new_start: 41,
            new_count: 7,
            old_start: 40,
            old_count: 6,
            lines: Vec::new(),
        };
        let t = Tokens::new(strop_core::theme::ThemeId::Hone);
        let spans = hunk_header_spans(&t, &hunk);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        // the buffer row IS Hunk::header(): byte equality keeps the
        // caret, search and yank aligned with what is on screen
        assert_eq!(joined, hunk.header());
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "-40,6")
                .unwrap()
                .style
                .fg,
            Some(t.diff_del)
        );
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "+41,7")
                .unwrap()
                .style
                .fg,
            Some(t.diff_add)
        );
    }
}
