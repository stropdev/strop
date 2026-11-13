//! The status buffer's adapter (0096 §2.1, drawn by 0100 §4.2): its typed
//! `StatusRow`s become block rows. Sections are ruled headings with their
//! count as the fact; file rows carry the change letter in its ink, the
//! path in the list idiom and the entry's `+N -M` as their fact; an
//! expanded file's rows carry the rail and draw their diff as the diff
//! view does. The text spans are the buffer line's own bytes, so
//! search, yank and the caret read real text.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::editor::{StatusRow, StatusView};
use crate::render::block::gutter::{self, Sign};
use crate::render::block::{self, diff_line, heading, Painted};
use strop_git::Change;

use super::super::style::Tokens;

/// The section the cursor is in, by the nearest section heading above it.
fn section_of(view: &StatusView, line: usize) -> Option<strop_git::Section> {
    view.rows
        .get(..=line.min(view.rows.len().saturating_sub(1)))?
        .iter()
        .rev()
        .find_map(|row| match row {
            StatusRow::Section(section) => Some(*section),
            _ => None,
        })
}

/// Paint line `line` of a status buffer. `None` renders as plain text
/// (the "loading" row before the first snapshot lands).
pub(crate) fn painted<'a>(
    t: &Tokens,
    view: &'a StatusView,
    line: usize,
    cursor_line: usize,
    cursor: bool,
    text_cells: usize,
) -> Option<Painted<'a>> {
    let row = view.row(line)?;
    let quiet = Style::default().fg(t.muted);
    // rows nested under an expanded file carry the rail; the cursor's
    // `▌` takes that cell on its own row
    let sign = if cursor {
        Sign::Cursor
    } else if !matches!(row, StatusRow::File(_)) && row.entry().is_some() {
        Sign::Rail
    } else {
        Sign::Blank
    };
    let list = |text: Vec<Span<'static>>| Painted {
        gutter: gutter::list(t, sign),
        text,
        ..Default::default()
    };
    let mut painted = match row {
        StatusRow::Blank => list(Vec::new()),
        StatusRow::Head => list(vec![
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
            list(spans)
        }
        StatusRow::Clean => list(vec![Span::styled(
            "Nothing to commit, working tree clean",
            quiet,
        )]),
        StatusRow::Section(section) => {
            let here = section_of(view, cursor_line) == Some(section);
            let heading = heading::Heading {
                gutter: strop_engine::editor::prepare::LIST_GUTTER,
                rule: heading::Rule::Light,
                name: heading::Name::Words(vec![Span::styled(
                    section.title(),
                    Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                )]),
                marks: Vec::new(),
                fact: heading::fact(t, view.count(section).to_string()),
                focus: if here {
                    heading::Focus::Here
                } else {
                    heading::Focus::Elsewhere
                },
            };
            // a heading keeps its rule on the cursor row: the band says
            // where the cursor is, the rule where the block starts
            Painted::heading(t, &heading, text_cells)
        }
        StatusRow::File(index) => {
            let entry = view.entry(index)?;
            let letter = match entry.change {
                Change::Added => t.diff_add,
                Change::Deleted => t.diff_del,
                Change::Modified | Change::TypeChanged => t.diff_change,
                Change::Conflicted => t.warning,
                Change::Untracked => t.muted,
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
            // the entry's `+N -M` (or `binary`) at the right edge, once
            // the counts follow the snapshot (0101 §11)
            let fact = match view.stat(index) {
                Some(stat) if stat.binary => vec![Span::styled("binary", quiet)],
                Some(stat) => heading::stats(t, stat.added, stat.deleted),
                None => Vec::new(),
            };
            list(block::fact_row(spans, fact, text_cells))
        }
        StatusRow::Note(index) => {
            let binary = matches!(
                view.delta(view.entry(index)?),
                Some(strop_git::FileDelta::Binary)
            );
            list(vec![Span::styled(
                if binary {
                    "  binary file"
                } else {
                    "  no line changes"
                },
                quiet.add_modifier(Modifier::ITALIC),
            )])
        }
        StatusRow::HunkHeader { entry, hunk } => Painted::band(
            t,
            gutter::list(t, sign),
            diff_line::hunk_header(t, view.hunk(entry, hunk)?),
            text_cells,
        ),
        StatusRow::Line { entry, hunk, line } => {
            let diff = view.diff_line(entry, hunk, line)?;
            // the emphasis range is in the text's bytes; the row leads
            // with its one-byte prefix
            let emphasis = view
                .emphasis(entry, hunk, line)
                .map(|(start, end)| (start + 1, end + 1));
            Painted {
                gutter: gutter::list(t, sign),
                text: diff_line::prefixed(t, diff),
                row_bg: diff_line::origin_bg(t, diff.origin),
                line: Some((diff, emphasis)),
                ..Default::default()
            }
        }
    };
    if cursor {
        painted.row_bg = Some(t.cursorline);
    }
    Some(painted)
}
