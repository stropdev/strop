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

use super::{accent, muted, text};

mod list;
mod margins;

pub(crate) use list::{cursor_row_bg, surface_list_row, RowDecoration};
pub(crate) use margins::{blame_blank, blame_spans, sidebar_row_spans};

pub(crate) fn add_fg() -> Color {
    crate::render::seed(strop_core::theme::current().class_string)
}
pub(crate) fn del_fg() -> Color {
    crate::render::seed(strop_core::theme::current().diag_error)
}
/// Quiet full-row backgrounds — a visible scan signal that doesn't
/// shout (tuicr's two-tier idea: quiet under content, loud markers).
fn add_bg() -> Color {
    crate::render::seed(strop_core::theme::current().diff_add_bg)
}
fn del_bg() -> Color {
    crate::render::seed(strop_core::theme::current().diff_del_bg)
}
/// Structural rows (stats, hunk headers) sit on a band, not an accent.
pub(crate) fn band_bg() -> Color {
    crate::render::seed(strop_core::theme::current().surface)
}

pub(crate) fn origin_fg(origin: LineOrigin) -> Color {
    match origin {
        LineOrigin::Addition => add_fg(),
        LineOrigin::Deletion => del_fg(),
        LineOrigin::Context => text(),
    }
}

/// Full-row background for add/del rows; context rows stay on base().
pub(crate) fn origin_bg(origin: LineOrigin) -> Option<Color> {
    match origin {
        LineOrigin::Addition => Some(add_bg()),
        LineOrigin::Deletion => Some(del_bg()),
        LineOrigin::Context => None,
    }
}

/// Brighter backgrounds for the intra-line changed spans (delta's
/// two-tier emphasis: row tint whispers, changed span speaks).
pub(crate) fn add_strong_bg() -> Color {
    crate::render::seed(strop_core::theme::current().diff_add_strong_bg)
}
pub(crate) fn del_strong_bg() -> Color {
    crate::render::seed(strop_core::theme::current().diff_del_strong_bg)
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
    line: &DiffLine,
    is_cursor_row: bool,
    digits: usize,
) -> Vec<Span<'static>> {
    // rootle's triangle: the cursor row's marker points at you
    let (marker, color) = match (line.origin, is_cursor_row) {
        (LineOrigin::Addition, true) => ("▸", add_fg()),
        (LineOrigin::Deletion, true) => ("▸", del_fg()),
        (LineOrigin::Context, true) => ("▸", accent()),
        (LineOrigin::Addition, false) => ("▎", add_fg()),
        (LineOrigin::Deletion, false) => ("▎", del_fg()),
        (LineOrigin::Context, false) => (" ", muted()),
    };
    let number = |n: Option<usize>| {
        let style = if is_cursor_row {
            Style::default().fg(accent())
        } else {
            Style::default().fg(muted())
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
pub(crate) fn structural_row(surface: &Surface, row: usize) -> Line<'static> {
    match (surface, row) {
        (Surface::Diff { hunks, .. }, 0) => Line::from(vec![
            Span::styled(
                hunks.label().to_owned(),
                Style::default().fg(text()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" +{} ", hunks.added()),
                Style::default().fg(add_fg()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("-{}", hunks.deleted()),
                Style::default().fg(del_fg()).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(band_bg())),
        (Surface::Diff { .. }, _) => surface
            .diff_row(row)
            .and_then(|row| match row {
                DiffRow::HunkHeader(hunk) => Some(hunk_header_spans(hunk)),
                _ => None,
            })
            .map(|spans| Line::from(spans).style(Style::default().bg(band_bg())))
            .unwrap_or_default(),
        _ => Line::default(),
    }
}

/// The hunk-header band: `@@` quiet, the old side in the deletion
/// color, the new side in the addition color — the range reads
/// structurally without shouting. The spans concatenate to exactly
/// `Hunk::header()`, the string the buffer row holds.
fn hunk_header_spans(hunk: &strop_git::Hunk) -> Vec<Span<'static>> {
    vec![
        Span::styled("@@ ", Style::default().fg(muted())),
        Span::styled(
            format!("-{},{}", hunk.old_start, hunk.old_count),
            Style::default().fg(del_fg()),
        ),
        Span::styled(" ", Style::default().fg(muted())),
        Span::styled(
            format!("+{},{}", hunk.new_start, hunk.new_count),
            Style::default().fg(add_fg()),
        ),
        Span::styled(" @@", Style::default().fg(muted())),
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
        let spans = hunk_header_spans(&hunk);
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
            Some(del_fg())
        );
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "+41,7")
                .unwrap()
                .style
                .fg,
            Some(add_fg())
        );
    }
}
