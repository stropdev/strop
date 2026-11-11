//! Status-buffer row decoration (0096 §2.1): one accent (the branch),
//! greys for structure, file names in the row idiom, and the diff
//! surface's own add/delete colours for inline diff rows. Every row is
//! styled from its typed `StatusRow`; the spans concatenate to exactly
//! the buffer line, so search, yank and the caret read real text.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::editor::{StatusRow, StatusView};
use strop_git::Change;

use super::super::style::Tokens;
use super::{hunk_header_spans, origin_bg, origin_fg, RowDecoration};

/// Decorate line `line` of a status buffer. `None` renders as plain
/// text (the "loading" row before the first snapshot lands).
pub(crate) fn status_row(
    t: &Tokens,
    view: &StatusView,
    line: usize,
    cursor_row: bool,
) -> Option<RowDecoration> {
    let cursor_bg = cursor_row.then_some(t.cursorline);
    let quiet = Style::default().fg(t.muted);
    let plain = |spans: Vec<Span<'static>>| RowDecoration {
        spans,
        row_bg: cursor_bg,
    };
    Some(match view.row(line)? {
        StatusRow::Blank => plain(Vec::new()),
        StatusRow::Head => plain(vec![
            Span::styled("Head: ", quiet),
            Span::styled(
                view.tree.head.clone().unwrap_or("(no branch)".into()),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
        ]),
        StatusRow::Tracking => {
            let tracking = view.tree.tracking.as_ref()?;
            let mut spans = vec![
                Span::styled("Upstream: ", quiet),
                Span::styled(tracking.upstream.clone(), Style::default().fg(t.secondary)),
            ];
            let mut rest = String::new();
            if tracking.ahead > 0 {
                rest.push_str(&format!(", ahead {}", tracking.ahead));
            }
            if tracking.behind > 0 {
                rest.push_str(&format!(", behind {}", tracking.behind));
            }
            if !rest.is_empty() {
                spans.push(Span::styled(rest, quiet));
            }
            plain(spans)
        }
        StatusRow::Clean => plain(vec![Span::styled(
            "Nothing to commit, working tree clean",
            quiet,
        )]),
        StatusRow::Section(section) => plain(vec![
            Span::styled(
                section.title(),
                Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" ({})", view.count(section)), quiet),
        ]),
        StatusRow::File(index) => {
            let entry = view.entry(index)?;
            let letter = match entry.change {
                Change::Added => t.diff_add,
                Change::Deleted => t.diff_del,
                Change::Conflicted => t.warning,
                Change::Modified | Change::TypeChanged | Change::Untracked => t.muted,
            };
            let mut spans = vec![Span::styled(
                format!("{} ", entry.change.letter()),
                Style::default().fg(letter),
            )];
            // the row idiom (0088 §2 "File names"): directory secondary, name text
            spans.extend(crate::render::file_name::spans(
                &entry.rel.to_string_lossy(),
                crate::render::file_name::Role::Row,
                t,
            ));
            plain(spans)
        }
        StatusRow::Note(index) => {
            let binary = matches!(
                view.delta(view.entry(index)?),
                Some(strop_git::FileDelta::Binary)
            );
            plain(vec![Span::styled(
                if binary {
                    "  binary file"
                } else {
                    "  no line changes"
                },
                quiet,
            )])
        }
        StatusRow::HunkHeader { entry, hunk } => RowDecoration {
            spans: hunk_header_spans(t, view.hunk(entry, hunk)?),
            row_bg: cursor_bg.or(Some(t.surface)),
        },
        StatusRow::Line { entry, hunk, line } => {
            let line = view.diff_line(entry, hunk, line)?;
            let prefix = match line.origin {
                strop_git::LineOrigin::Addition => '+',
                strop_git::LineOrigin::Deletion => '-',
                strop_git::LineOrigin::Context => ' ',
            };
            RowDecoration {
                spans: vec![Span::styled(
                    format!("{prefix}{}", line.text_str()),
                    Style::default().fg(origin_fg(t, line.origin)),
                )],
                row_bg: cursor_bg.or(origin_bg(t, line.origin)),
            }
        }
    })
}
