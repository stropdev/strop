//! Diff surface rendering (0010 §4/§5): gutters, row backgrounds,
//! structural rows. All decoration comes from typed hunk data —
//! never from sniffing `+`/`-` in the text. Anatomy borrowed from
//! tuicr (`[sign][old][new][content]`, quiet bands for structural
//! rows), tuned to the strop palette. Log/file row decoration lives
//! in `diff/list`; the blame column and commit file sidebar in
//! `diff/margins` (0011/0032).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::editor::{DiffRow, Editor, Surface};
use strop_git::{DiffLine, LineOrigin};

use super::{ACCENT, MUTED, TEXT};

mod list;
mod margins;

pub(crate) use list::surface_list_row;
pub(crate) use margins::{blame_blank, blame_spans, Sidebar, BLAME_W};

pub(crate) const ADD_FG: Color = Color::Rgb(0xa9, 0xc4, 0x7c);
pub(crate) const DEL_FG: Color = Color::Rgb(0xe8, 0x67, 0x7a);
/// Quiet full-row backgrounds — a visible scan signal that doesn't
/// shout (tuicr's two-tier idea: quiet under content, loud markers).
const ADD_BG: Color = Color::Rgb(0x1b, 0x26, 0x20);
const DEL_BG: Color = Color::Rgb(0x2a, 0x1d, 0x20);
/// Structural rows (stats, hunk headers) sit on a band, not an accent.
const BAND_BG: Color = Color::Rgb(0x22, 0x24, 0x2e);

pub(crate) fn origin_fg(origin: LineOrigin) -> Color {
    match origin {
        LineOrigin::Addition => ADD_FG,
        LineOrigin::Deletion => DEL_FG,
        LineOrigin::Context => TEXT,
    }
}

/// Full-row background for add/del rows; context rows stay on BASE.
pub(crate) fn origin_bg(origin: LineOrigin) -> Option<Color> {
    match origin {
        LineOrigin::Addition => Some(ADD_BG),
        LineOrigin::Deletion => Some(DEL_BG),
        LineOrigin::Context => None,
    }
}

/// Brighter backgrounds for the intra-line changed spans (delta's
/// two-tier emphasis: row tint whispers, changed span speaks).
pub(crate) const ADD_STRONG_BG: Color = Color::Rgb(0x27, 0x3a, 0x30);
pub(crate) const DEL_STRONG_BG: Color = Color::Rgb(0x40, 0x2a, 0x2e);

/// Intra-line emphasis (delta-style): the byte range within this row's
/// text that actually changed, paired against the opposite-side row at
/// the same index in the hunk's del/add run. None for context rows and
/// unmatched rows (pure adds/deletes emphasize the whole line).
pub(crate) fn emphasis_span(surface: Option<&Surface>, row: usize) -> Option<(usize, usize)> {
    let Some(Surface::Diff { hunks, .. }) = surface else {
        return None;
    };
    if row == 0 {
        return None;
    }
    let mut row = row - 1;
    for hunk in hunks {
        if row == 0 {
            return None;
        }
        row -= 1;
        if row < hunk.lines.len() {
            return hunk_emphasis(&hunk.lines, row);
        }
        row -= hunk.lines.len();
    }
    None
}

/// Pair a row with its opposite-side counterpart in the hunk and return
/// THIS row's changed byte range.
fn hunk_emphasis(lines: &[DiffLine], idx: usize) -> Option<(usize, usize)> {
    let origin = lines[idx].origin;
    match origin {
        LineOrigin::Context => None,
        LineOrigin::Deletion => {
            // del-run start and the add-run right after it
            let mut run_start = idx;
            while run_start > 0 && lines[run_start - 1].origin == LineOrigin::Deletion {
                run_start -= 1;
            }
            let mut add_start = idx;
            while add_start < lines.len() && lines[add_start].origin == LineOrigin::Deletion {
                add_start += 1;
            }
            let k = idx - run_start;
            lines
                .get(add_start + k)
                .filter(|l| l.origin == LineOrigin::Addition)
                .map(|p| changed_range(&lines[idx].text_str(), &p.text_str()))
        }
        LineOrigin::Addition => {
            // add-run start and the del-run right before it
            let mut run_start = idx;
            while run_start > 0 && lines[run_start - 1].origin == LineOrigin::Addition {
                run_start -= 1;
            }
            let mut del_start = run_start;
            while del_start > 0 && lines[del_start - 1].origin == LineOrigin::Deletion {
                del_start -= 1;
            }
            if del_start == run_start {
                return None; // no paired deletions
            }
            let k = idx - run_start;
            lines
                .get(del_start + k)
                .filter(|l| l.origin == LineOrigin::Deletion)
                .map(|p| changed_range(&p.text_str(), &lines[idx].text_str()))
        }
    }
}

/// The changed middle of `a` vs `b` after trimming the common prefix
/// and suffix (byte offsets into `a`, char-boundary safe by
/// construction).
fn changed_range(a: &str, b: &str) -> (usize, usize) {
    let prefix: usize = a
        .chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = a
        .chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let end = a.len().saturating_sub(suffix).max(prefix);
    (prefix.min(end), end)
}

/// Gutter width for a surface's buffer: the diff gutter widens to fit
/// both sides' numbers (min 3 digits each); everything else is the
/// standard sign+number gutter.
pub(crate) fn gutter_width(surface: Option<&Surface>) -> usize {
    let Some(Surface::Diff { hunks, .. }) = surface else {
        return super::buffer::GUTTER as usize;
    };
    let max_lineno = hunks
        .iter()
        .flat_map(|h| &h.lines)
        .flat_map(|l| [l.old_lineno, l.new_lineno])
        .flatten()
        .max()
        .unwrap_or(0);
    let digits = max_lineno.to_string().len().max(3);
    // sign(1) + old(digits) + space + new(digits) + space
    1 + digits + 1 + digits + 1
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
        (LineOrigin::Addition, true) => ("▸", ADD_FG),
        (LineOrigin::Deletion, true) => ("▸", DEL_FG),
        (LineOrigin::Context, true) => ("▸", ACCENT),
        (LineOrigin::Addition, false) => ("▎", ADD_FG),
        (LineOrigin::Deletion, false) => ("▎", DEL_FG),
        (LineOrigin::Context, false) => (" ", MUTED),
    };
    let number = |n: Option<usize>| {
        let style = if is_cursor_row {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(MUTED)
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
        (
            Surface::Diff {
                label,
                added,
                deleted,
                ..
            },
            0,
        ) => Line::from(vec![
            Span::styled(
                label.clone(),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" +{added} "),
                Style::default().fg(ADD_FG).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("-{deleted}"),
                Style::default().fg(DEL_FG).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(BAND_BG)),
        (Surface::Diff { .. }, _) => surface
            .diff_row(row)
            .and_then(|row| match row {
                DiffRow::HunkHeader(hunk) => Some(hunk_header_spans(hunk)),
                _ => None,
            })
            .map(|spans| Line::from(spans).style(Style::default().bg(BAND_BG)))
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
        Span::styled("@@ ", Style::default().fg(MUTED)),
        Span::styled(
            format!("-{},{}", hunk.old_start, hunk.old_count),
            Style::default().fg(DEL_FG),
        ),
        Span::styled(" ", Style::default().fg(MUTED)),
        Span::styled(
            format!("+{},{}", hunk.new_start, hunk.new_count),
            Style::default().fg(ADD_FG),
        ),
        Span::styled(" @@", Style::default().fg(MUTED)),
    ]
}

/// The number gutter for a pane's buffer: diff surfaces keep their
/// two-sided gutter; ordinary buffers size to the largest line number
/// (a fixed 5-cell gutter misaligned the caret past line 999).
pub(crate) fn number_gutter_width(editor: &Editor, doc: strop_core::id::DocumentId) -> usize {
    let buffer = editor.doc(doc);
    let surface = buffer.surface_payload();
    if matches!(surface, Some(Surface::Diff { .. })) {
        return gutter_width(surface);
    }
    let number = buffer.buf.last_content_line() + 1;
    let digits = number.ilog10() as usize + 1;
    2 + digits.max(3)
}

/// Total left inset before a pane's content: file sidebar + blame
/// column + the surface's number gutter. Cursor placement and the
/// inactive-pane caret both derive from here — one composition, no
/// per-surface drift (0011 §3/§4). The sidebar contributes exactly
/// what its emission draws (`Sidebar::outer_width`), so the caret and
/// the tree can never disagree (0032 §3).
pub(crate) fn left_inset(editor: &Editor, buffer: strop_core::id::DocumentId) -> usize {
    let surface = editor.docs.get(buffer).and_then(|d| d.surface_payload());
    let mut inset = number_gutter_width(editor, buffer);
    if editor.blame_gutter_for(buffer).is_some() {
        inset += BLAME_W;
    }
    if let Some(Surface::Diff {
        commit: Some(cf), ..
    }) = surface
    {
        inset += Sidebar::measured_width(&cf.files);
    }
    inset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emphasis_trims_shared_affixes() {
        // delta-style: only the middle changed
        assert_eq!(
            changed_range("let x = hone(a);", "let x = hone(b, c);"),
            (13, 14) // only "a" vs "b, c" differs
        );
        // whole line changed
        assert_eq!(changed_range("aaa", "bbb"), (0, 3));
        // identical → empty range
        assert_eq!(changed_range("same", "same"), (4, 4));
    }

    #[test]
    fn hunk_pairs_deletions_with_additions() {
        let line = |origin, old, new, text: &str| DiffLine {
            has_newline: true,
            origin,
            old_lineno: old,
            new_lineno: new,
            text: text.into(),
        };
        let lines = vec![
            line(LineOrigin::Context, Some(1), Some(1), "fn f() {"),
            line(LineOrigin::Deletion, Some(2), None, "    hone(a);"),
            line(LineOrigin::Deletion, Some(3), None, "    gone();"),
            line(LineOrigin::Addition, None, Some(2), "    hone(b, c);"),
            line(LineOrigin::Context, Some(4), Some(3), "}"),
        ];
        // first deletion pairs with the lone addition
        assert_eq!(
            hunk_emphasis(&lines, 1),
            Some(changed_range("    hone(a);", "    hone(b, c);"))
        );
        // second deletion has no pair
        assert_eq!(hunk_emphasis(&lines, 2), None);
        // the addition sees the same middle from its own side
        assert_eq!(
            hunk_emphasis(&lines, 3),
            Some(changed_range("    hone(a);", "    hone(b, c);"))
        );
        // context never emphasizes
        assert_eq!(hunk_emphasis(&lines, 0), None);
    }

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
            Some(DEL_FG)
        );
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "+41,7")
                .unwrap()
                .style
                .fg,
            Some(ADD_FG)
        );
    }
}
