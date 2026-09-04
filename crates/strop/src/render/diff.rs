//! Diff surface rendering (0010 §4/§5): gutters, row backgrounds,
//! structural rows — plus the 0011 left-margin columns (blame gutter,
//! commit file sidebar). All decoration comes from typed hunk data —
//! never from sniffing `+`/`-` in the text. Anatomy borrowed from
//! tuicr (`[sign][old][new][content]`, quiet bands for structural
//! rows), tuned to the strop palette.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::editor::{Editor, Surface};
use strop_git::memory::ChangedFile;
use strop_git::{DiffLine, LineOrigin};

use super::{ACCENT, MUTED, SELECT_BG, TEXT};

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
                .map(|p| changed_range(&lines[idx].text, &p.text))
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
                .map(|p| changed_range(&p.text, &lines[idx].text))
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

/// `* 51b63a8 t · 35 seconds ago · subject` → lane-colored graph runes,
/// sha accent bold, `·` separators dim, the rest text.
fn log_row_spans(text: &str) -> Vec<Span<'static>> {
    let graph_len = text
        .chars()
        .take_while(|c| "*|/\\<>-_ ".contains(*c))
        .count();
    let rest = &text[graph_len..];
    let sha_len = rest
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .map(|c| c.len_utf8())
        .sum::<usize>();
    let mut spans = graph_spans(&text[..graph_len]);

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

/// Lane-colored graph art: each two-column lane cycles the palette, the
/// commit node `*` is bold in its lane's color (gitui/lazygit lesson —
/// lane color is how the eye tracks a branch through merges).
fn graph_spans(prefix: &str) -> Vec<Span<'static>> {
    const LANES: [Color; 6] = [
        ACCENT,                       // amber
        Color::Rgb(0x9e, 0xce, 0x6a), // green
        Color::Rgb(0x7a, 0xa2, 0xf7), // blue
        Color::Rgb(0xbb, 0x9a, 0xf7), // purple
        Color::Rgb(0x7d, 0xcf, 0xff), // cyan
        Color::Rgb(0xe0, 0xaf, 0x68), // yellow
    ];
    prefix
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if c == ' ' {
                return Span::styled(" ", Style::default());
            }
            let color = LANES[(i / 2) % LANES.len()];
            let style = if c == '*' {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            Span::styled(c.to_string(), style)
        })
        .collect()
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

// ---- left-margin columns (0011) -----------------------------------------

/// Blame gutter width: `sha˟7 author˟9 age˟3` + separators.
pub(crate) const BLAME_W: usize = 22;
/// Pane-divider color — the sidebar's rule matches it.
const RULE: Color = Color::Rgb(0x3a, 0x3d, 0x4d);
/// Younger than this counts as "recent" → accent (0011 §3).
const RECENT_SECS: i64 = 30 * 86400;

/// The blame cell for one buffer line: `sha7 author9 age3`, muted for
/// old commits, accent for recent ones and uncommitted lines
/// (`0000000 you now`).
pub(crate) fn blame_spans(line: &strop_git::memory::BlameLine) -> Span<'static> {
    let uncommitted = line.is_uncommitted();
    let recent = line.ts > 0 && unix_now() - line.ts < RECENT_SECS;
    let fg = if uncommitted || recent { ACCENT } else { MUTED };
    let sha: String = if uncommitted {
        "0".repeat(7)
    } else {
        line.sha.chars().take(7).collect()
    };
    let author = ellipsize(&line.author, 9);
    let age: String = line.age.chars().take(3).collect();
    Span::styled(
        format!("{sha} {author:<9} {age:>3} "),
        Style::default().fg(fg),
    )
}

/// A blank blame cell (filler rows past the buffer end).
pub(crate) fn blame_blank() -> Span<'static> {
    Span::styled(" ".repeat(BLAME_W), Style::default())
}
/// Sidebar width fits the commit's longest path (clamped 12–24) — a
/// two-file commit shouldn't pay a 28-column pane.
pub(crate) fn sidebar_width(files: &[ChangedFile]) -> usize {
    let longest = files
        .iter()
        .map(|f| f.path.display().to_string().chars().count())
        .max()
        .unwrap_or(0);
    (longest + 2).clamp(12, 24)
}

/// One sidebar row: the commit's changed files, current one marked `▌`
/// (or `▸` when the sidebar has Tab focus — tuicr's rule) on the
/// selection background, plus the dividing rule — accent when focused.
/// Rows past the file list stay blank so the column reads as one surface.
pub(crate) fn sidebar_spans(
    files: &[ChangedFile],
    current: &str,
    row: usize,
    focused: bool,
) -> Vec<Span<'static>> {
    let w = sidebar_width(files);
    let cell = match files.get(row) {
        Some(f) => {
            let path = f.path.display().to_string();
            if path == current {
                let shown = ellipsize(&path, w - 2);
                let pad = w - 1 - shown.chars().count();
                vec![
                    Span::styled(
                        format!("{}{shown}", if focused { "▸" } else { "▌" }),
                        Style::default().fg(ACCENT).bg(SELECT_BG),
                    ),
                    Span::styled(" ".repeat(pad), Style::default().bg(SELECT_BG)),
                ]
            } else {
                let shown = ellipsize(&path, w - 1);
                let pad = w - 1 - shown.chars().count();
                vec![
                    Span::styled(format!(" {shown}"), Style::default().fg(TEXT)),
                    Span::styled(" ".repeat(pad), Style::default()),
                ]
            }
        }
        None => vec![Span::styled(" ".repeat(w), Style::default())],
    };
    let mut spans = cell;
    spans.push(Span::styled(
        "│",
        Style::default().fg(if focused { ACCENT } else { RULE }),
    ));
    spans
}

/// Total left inset before a pane's content: file sidebar + blame
/// column + the surface's number gutter. Cursor placement and the
/// inactive-pane caret both derive from here — one composition, no
/// per-surface drift (0011 §3/§4).
pub(crate) fn left_inset(editor: &Editor, buffer: strop_core::id::DocumentId) -> usize {
    let surface = editor.docs.get(buffer).and_then(|d| d.surface.as_ref());
    let mut inset = gutter_width(surface);
    if editor.blame_gutter_for(buffer).is_some() {
        inset += BLAME_W;
    }
    if let Some(Surface::Diff {
        commit: Some(cf), ..
    }) = surface
    {
        inset += sidebar_width(&cf.files) + 1;
    }
    inset
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `s` clipped to `n` chars with a trailing `…` when it had more.
fn ellipsize(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n - 1).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_lanes_get_distinct_colors() {
        // a two-lane merge row: `*` in lane 0, `|` in lane 1
        let spans = log_row_spans("* | 3a9eeec t · 1s ago · merge");
        let star = spans.iter().find(|s| s.content == "*").unwrap();
        let bar = spans.iter().find(|s| s.content == "|").unwrap();
        assert_ne!(star.style.fg, bar.style.fg, "lanes must differ");
        assert_eq!(star.style.fg, Some(ACCENT));
        assert!(spans
            .iter()
            .any(|s| s.content == "3a9eeec" && s.style.add_modifier.contains(Modifier::BOLD)));
    }

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
        use strop_git::{DiffLine, LineOrigin};
        let line = |origin, old, new, text: &str| DiffLine {
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
}
