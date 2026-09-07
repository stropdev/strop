//! Commit-log and changed-file row decoration (0010 §5, 0032 §2):
//! rows are colored from typed data — lane-colored graph art, sha
//! accent, author/age metadata quiet, the subject and the filename
//! carrying the emphasis. The concatenated span content starts with
//! the buffer line's exact bytes; anything after that prefix is
//! virtual EOL content, so search, yank and the caret keep reading
//! the real buffer.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::editor::Surface;
use strop_git::memory::ChangedFile;

use super::super::text;
use super::super::{ACCENT, MUTED, TEXT};
use super::{ADD_FG, DEL_FG};

/// Restrained cursor-row band for log/file surfaces — a hair above
/// BASE, well under the selection color, so search/visual overlays
/// still read on top of it (they override the background per cell).
pub(crate) const CURSOR_ROW_BG: Color = Color::Rgb(0x1e, 0x20, 0x2b);

/// One decorated row of a log/files surface: styled spans whose
/// concatenation begins with the buffer line's exact bytes, plus an
/// optional full-row background (the cursor-row band).
pub(crate) struct RowDecoration {
    pub(crate) spans: Vec<Span<'static>>,
    pub(crate) row_bg: Option<Color>,
}

/// Decorate row `line_idx` of a CommitLog or ChangedFiles surface.
/// `cursor_row` adds the quiet current-row band. Returns None for
/// rows that render as normal text.
pub(crate) fn surface_list_row(
    surface: Option<&Surface>,
    line_idx: usize,
    width: usize,
    cursor_row: bool,
) -> Option<RowDecoration> {
    let row_bg = cursor_row.then_some(CURSOR_ROW_BG);
    match surface? {
        Surface::CommitLog { rows, .. } => {
            let row = rows.get(line_idx)?;
            Some(RowDecoration {
                spans: log_row_spans(&row.text),
                row_bg,
            })
        }
        Surface::ChangedFiles { sha, files, .. } => match line_idx {
            0 => Some(RowDecoration {
                spans: vec![
                    Span::styled("commit ", Style::default().fg(MUTED)),
                    Span::styled(
                        sha.chars().take(10).collect::<String>(),
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                ],
                row_bg,
            }),
            1 => Some(RowDecoration {
                spans: Vec::new(),
                row_bg,
            }),
            _ => {
                let file = files.get(line_idx - 2)?;
                Some(RowDecoration {
                    spans: file_row_spans(file, width),
                    row_bg,
                })
            }
        },
        _ => None,
    }
}

/// `* 51b63a8 t · 35 seconds ago · subject` → lane-colored graph
/// runes, sha accent bold, author and age muted, subject in text.
/// The subject is the THIRD ` · ` field, so a subject that itself
/// contains ` · ` stays one piece (splitn(3), never a full split).
fn log_row_spans(text: &str) -> Vec<Span<'static>> {
    let (graph, rest) = split_graph(text);
    let mut spans = graph_spans(graph);
    let sha_len = hex_prefix_len(rest);
    if sha_len == 0 {
        // graph-only art carries nothing after it; load/error rows
        // are ordinary text, not the metadata shape
        if !rest.is_empty() {
            spans.push(Span::styled(rest.to_string(), Style::default().fg(TEXT)));
        }
        return spans;
    }
    spans.push(Span::styled(
        rest[..sha_len].to_string(),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ));
    let remainder = &rest[sha_len..];
    let mut parts = remainder.splitn(3, " · ");
    if let (Some(author), Some(age), Some(subject)) = (parts.next(), parts.next(), parts.next()) {
        spans.push(quiet(author));
        spans.push(quiet(" · "));
        spans.push(quiet(age));
        spans.push(quiet(" · "));
        if !subject.is_empty() {
            spans.push(Span::styled(subject.to_string(), Style::default().fg(TEXT)));
        }
    } else if !remainder.is_empty() {
        spans.push(Span::styled(
            remainder.to_string(),
            Style::default().fg(TEXT),
        ));
    }
    spans
}

fn quiet(part: &str) -> Span<'static> {
    Span::styled(part.to_string(), Style::default().fg(MUTED))
}

/// Split graph art (ASCII lanes) from the rest at a byte-exact
/// boundary, so the sha that follows stays byte-aligned.
fn split_graph(text: &str) -> (&str, &str) {
    let end = text
        .char_indices()
        .take_while(|(_, c)| "*|/\\<>-_ ".contains(*c))
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    text.split_at(end)
}

fn hex_prefix_len(rest: &str) -> usize {
    rest.char_indices()
        .take_while(|(_, c)| c.is_ascii_hexdigit())
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0)
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

/// Path left — directory context muted, filename emphasized — then
/// `+N -M` past at least one space. The stats are virtual EOL content
/// (the buffer row holds only the path), so the caret and the path
/// stay byte-aligned no matter how wide the stats run. Path labels and
/// stats are printable chrome, measured in display cells.
fn file_row_spans(file: &ChangedFile, width: usize) -> Vec<Span<'static>> {
    let display = strop_core::layout::printable_text(file.path.to_string_lossy());
    let path_cells = text::width(&display);
    let mut spans = match display.rfind('/') {
        Some(at) => vec![
            Span::styled(display[..=at].to_string(), Style::default().fg(MUTED)),
            Span::styled(display[at + 1..].to_string(), Style::default().fg(TEXT)),
        ],
        None => vec![Span::styled(
            display.into_owned(),
            Style::default().fg(TEXT),
        )],
    };
    let added = format!("+{} ", file.added);
    let deleted = format!("-{}", file.deleted);
    let pad = width
        .saturating_sub(path_cells + added.len() + deleted.len())
        .max(1);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(added, Style::default().fg(ADD_FG)));
    spans.push(Span::styled(deleted, Style::default().fg(DEL_FG)));
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn log_row_quiets_metadata_and_keeps_bytes() {
        let source = "* 51b63a8 t · 35 seconds ago · polish the modeline";
        let spans = log_row_spans(source);
        // the buffer line's exact bytes come first — caret/yank truth
        assert!(joined(&spans).starts_with(source));
        let sha = spans.iter().find(|s| s.content == "51b63a8").unwrap();
        assert_eq!(sha.style.fg, Some(ACCENT));
        assert!(sha.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "polish the modeline")
                .unwrap()
                .style
                .fg,
            Some(TEXT)
        );
        for muted in [" t", "35 seconds ago"] {
            assert_eq!(
                spans.iter().find(|s| s.content == muted).unwrap().style.fg,
                Some(MUTED),
                "{muted} must be quiet"
            );
        }
    }

    #[test]
    fn log_subject_keeps_embedded_separators() {
        let source = "* abc1234 a · 1s ago · fix · real · subject";
        let spans = log_row_spans(source);
        assert!(joined(&spans).starts_with(source));
        // the whole subject is ONE text span — the third field, not a
        // full split
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "fix · real · subject")
                .unwrap()
                .style
                .fg,
            Some(TEXT)
        );
    }

    #[test]
    fn graph_art_and_plain_rows_stay_unmuted() {
        let graph = log_row_spans("| |\\");
        assert_eq!(joined(&graph), "| |\\");
        assert!(graph.iter().all(|s| s.style.fg != Some(MUTED)));
        let plain = log_row_spans("loading log…");
        assert_eq!(joined(&plain), "loading log…");
        assert!(plain.iter().all(|s| s.style.fg == Some(TEXT)));
    }

    #[test]
    fn file_row_mutes_directory_and_emphasizes_filename() {
        let file = ChangedFile {
            path: "src/render/diff.rs".into(),
            added: 12,
            deleted: 3,
        };
        let spans = file_row_spans(&file, 40);
        let text = joined(&spans);
        assert!(text.starts_with("src/render/diff.rs"));
        assert!(text.ends_with("+12 -3"));
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "src/render/")
                .unwrap()
                .style
                .fg,
            Some(MUTED)
        );
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "diff.rs")
                .unwrap()
                .style
                .fg,
            Some(TEXT)
        );
    }

    #[test]
    fn file_row_padding_counts_display_cells() {
        // 界 is two cells: the path is 7 cells, the stats 5 — a char
        // count (5 for the path) would push the stats 2 cells left
        let file = ChangedFile {
            path: "界/x.rs".into(),
            added: 1,
            deleted: 0,
        };
        let spans = file_row_spans(&file, 40);
        let pad = spans
            .iter()
            .find(|s| !s.content.is_empty() && s.content.chars().all(|c| c == ' '))
            .unwrap();
        assert_eq!(text::width(&pad.content), 40 - (7 + 5));
    }
}
