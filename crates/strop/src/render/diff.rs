//! Diff surface rendering (0010 §4/§5): gutters, row backgrounds,
//! structural rows. All decoration comes from typed hunk data — never
//! from sniffing `+`/`-` in the text. Anatomy borrowed from tuicr
//! (`[sign][old][new][content]`, quiet bands for structural rows),
//! tuned to the strop palette.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::editor::Surface;
use strop_git::{DiffLine, LineOrigin};

use super::{ACCENT, MUTED, TEXT};

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

/// What row `row` of a Diff surface is. Row 0 is the stats line; then
/// per hunk a header row followed by its content rows.
pub(crate) enum DiffRow<'a> {
    Stats,
    HunkHeader,
    Line(&'a DiffLine),
}

pub(crate) fn diff_row<'a>(surface: Option<&'a Surface>, row: usize) -> Option<DiffRow<'a>> {
    let Some(Surface::Diff { hunks, .. }) = surface else {
        return None;
    };
    if row == 0 {
        return Some(DiffRow::Stats);
    }
    let mut row = row - 1;
    for hunk in hunks {
        if row == 0 {
            return Some(DiffRow::HunkHeader);
        }
        row -= 1;
        if row < hunk.lines.len() {
            return Some(DiffRow::Line(&hunk.lines[row]));
        }
        row -= hunk.lines.len();
    }
    None
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
    let (marker, color) = match line.origin {
        LineOrigin::Addition => ("▎", ADD_FG),
        LineOrigin::Deletion => ("▎", DEL_FG),
        LineOrigin::Context => (" ", MUTED),
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

/// Stats and hunk-header rows: a quiet band across the full width —
/// label/stats left, nothing loud (0010 §4). `width` pads the band
/// past the text so it reads as a full row.
pub(crate) fn structural_row(surface: &Surface, row: usize, width: u16) -> Line<'static> {
    let mut line = structural_row_inner(surface, row);
    let used: usize = line.spans.iter().map(|s| s.content.len()).sum();
    let pad = (width as usize).saturating_sub(used + 1);
    if pad > 0 {
        line.spans
            .push(Span::styled(" ".repeat(pad), Style::default().bg(BAND_BG)));
    }
    line
}

fn structural_row_inner(surface: &Surface, row: usize) -> Line<'static> {
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
                format!(" {label}"),
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
        (Surface::Diff { hunks, .. }, _) => match hunk_header_at(hunks, row) {
            Some(header) => Line::from(Span::styled(
                format!(" {header}"),
                Style::default().fg(MUTED),
            ))
            .style(Style::default().bg(BAND_BG)),
            None => Line::default(),
        },
        _ => Line::default(),
    }
}

/// The hunk whose header sits at `row` (row 1 + hunk offsets).
fn hunk_header_at(hunks: &[strop_git::Hunk], row: usize) -> Option<String> {
    let mut row = row.checked_sub(1)?;
    for hunk in hunks {
        if row == 0 {
            return Some(hunk.header());
        }
        row -= 1;
        if row < hunk.lines.len() {
            return None;
        }
        row -= hunk.lines.len();
    }
    None
}

/// Commit-log and changed-files rows, decorated from their typed data
/// (0010 §5): graph runes dim, sha accent; paths with right-aligned
/// colored stats. Returns None for rows that render as normal text.
pub(crate) fn surface_content_spans(
    surface: Option<&Surface>,
    line_idx: usize,
    width: u16,
) -> Option<Vec<Span<'static>>> {
    match surface? {
        Surface::CommitLog { rows, .. } => {
            let row = rows.get(line_idx)?;
            Some(log_row_spans(&row.text))
        }
        Surface::ChangedFiles { sha, files, .. } => match line_idx {
            0 => Some(vec![
                Span::styled("commit ", Style::default().fg(MUTED)),
                Span::styled(
                    sha.chars().take(10).collect::<String>(),
                    Style::default().fg(ACCENT),
                ),
            ]),
            1 => Some(vec![]),
            _ => {
                let file = files.get(line_idx - 2)?;
                Some(file_row_spans(
                    &file.path.display().to_string(),
                    file.added,
                    file.deleted,
                    width,
                ))
            }
        },
        _ => None,
    }
}

/// `* 51b63a8 t · 35 seconds ago · subject` → graph dim, sha accent
/// bold, `·` separators dim, the rest text.
fn log_row_spans(text: &str) -> Vec<Span<'static>> {
    let graph_len = text
        .chars()
        .take_while(|c| "*|/\\<>- ".contains(*c))
        .count();
    let rest = &text[graph_len..];
    let sha_len = rest
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .map(|c| c.len_utf8())
        .sum::<usize>();
    let mut spans = vec![Span::styled(
        text[..graph_len].to_string(),
        Style::default().fg(MUTED),
    )];
    if sha_len > 0 {
        spans.push(Span::styled(
            rest[..sha_len].to_string(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    }
    for (i, part) in rest[sha_len..].split(" · ").enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(MUTED)));
        }
        if !part.is_empty() {
            spans.push(Span::styled(part.to_string(), Style::default().fg(TEXT)));
        }
    }
    spans
}

/// path left, ` +N -M` right-aligned to the row width.
fn file_row_spans(path: &str, added: usize, deleted: usize, width: u16) -> Vec<Span<'static>> {
    let stats_len = added.to_string().len() + deleted.to_string().len() + 4;
    let gutter = gutter_width(None);
    let room = (width as usize).saturating_sub(gutter + 1 + stats_len);
    let shown: String = path.chars().take(room).collect();
    let pad = room.saturating_sub(path.chars().count());
    vec![
        Span::styled(format!(" {shown}"), Style::default().fg(TEXT)),
        Span::styled(" ".repeat(pad + 1), Style::default()),
        Span::styled(
            format!("+{added} "),
            Style::default().fg(ADD_FG).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("-{deleted}"),
            Style::default().fg(DEL_FG).add_modifier(Modifier::BOLD),
        ),
    ]
}
