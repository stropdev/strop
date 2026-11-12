//! Diff surface rendering (0010 §4/§5): gutters, row backgrounds,
//! structural rows. All decoration comes from typed hunk data —
//! never from sniffing `+`/`-` in the text. Anatomy borrowed from
//! tuicr (`[sign][old][new][content]`, quiet bands for structural
//! rows), tuned to the strop palette. Log/file row decoration lives
//! in `diff/list`; the blame column and commit file sidebar in
//! `diff/margins` (0011/0032).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::editor::{DiffRow, Surface};

use super::style::Tokens;

mod list;
mod margins;
mod status;

pub(crate) use list::{surface_list_row, RowDecoration};
pub(crate) use margins::{blame_blank, blame_spans, sidebar_row_spans};
pub(crate) use status::painted as status_painted;

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
                DiffRow::HunkHeader(hunk) => Some(super::block::diff_line::hunk_header(t, hunk)),
                _ => None,
            })
            .map(|spans| Line::from(spans).style(Style::default().bg(t.surface)))
            .unwrap_or_default(),
        _ => Line::default(),
    }
}
