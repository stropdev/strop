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

use super::super::clip;
use super::super::style::Tokens;

/// One decorated row of a log/files surface: styled spans whose
/// concatenation begins with the buffer line's exact bytes, plus an
/// optional full-row background (the cursor-row band).
pub(crate) struct RowDecoration {
    pub(crate) spans: Vec<Span<'static>>,
    pub(crate) row_bg: Option<Color>,
}

/// Decorate row `line_idx` of a CommitLog, ChangedFiles or Status surface.
/// `cursor_row` adds the quiet current-row band. Returns None for
/// rows that render as normal text.
pub(crate) fn surface_list_row(
    t: &Tokens,
    surface: Option<&Surface>,
    line_idx: usize,
    width: usize,
    cursor_row: bool,
) -> Option<RowDecoration> {
    // the cursor row rides the cursorline plane, under the selection and
    // search overlays (they override the background per cell)
    let row_bg = cursor_row.then_some(t.cursorline);
    match surface? {
        Surface::CommitLog { rows, .. } => {
            let row = rows.get(line_idx)?;
            Some(RowDecoration {
                spans: log_row_spans(t, &row.text),
                row_bg,
            })
        }
        Surface::ChangedFiles { sha, files, .. } => match line_idx {
            0 => Some(RowDecoration {
                spans: vec![
                    Span::styled("commit ", Style::default().fg(t.muted)),
                    Span::styled(
                        sha.chars().take(10).collect::<String>(),
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
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
                    spans: file_row_spans(t, file, width),
                    row_bg,
                })
            }
        },
        Surface::Status { .. } => None,
        Surface::Diff { .. } => None,
    }
}

/// `* 51b63a8 t · 35 seconds ago · subject` → lane-colored graph
/// runes, sha accent bold, author and age muted, subject in text.
/// The subject is the THIRD ` · ` field, so a subject that itself
/// contains ` · ` stays one piece (splitn(3), never a full split).
fn log_row_spans(t: &Tokens, text: &str) -> Vec<Span<'static>> {
    let (graph, rest) = split_graph(text);
    let mut spans = graph_spans(t, graph);
    let sha_len = hex_prefix_len(rest);
    if sha_len == 0 {
        // graph-only art carries nothing after it; load/error rows
        // are ordinary text, not the metadata shape
        if !rest.is_empty() {
            spans.push(Span::styled(rest.to_string(), Style::default().fg(t.text)));
        }
        return spans;
    }
    spans.push(Span::styled(
        rest[..sha_len].to_string(),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ));
    let remainder = &rest[sha_len..];
    let mut parts = remainder.splitn(3, " · ");
    let quiet = |part: &str| Span::styled(part.to_string(), Style::default().fg(t.muted));
    if let (Some(author), Some(age), Some(subject)) = (parts.next(), parts.next(), parts.next()) {
        spans.push(quiet(author));
        spans.push(quiet(" · "));
        spans.push(quiet(age));
        spans.push(quiet(" · "));
        if !subject.is_empty() {
            spans.push(Span::styled(
                subject.to_string(),
                Style::default().fg(t.text),
            ));
        }
    } else if !remainder.is_empty() {
        spans.push(Span::styled(
            remainder.to_string(),
            Style::default().fg(t.text),
        ));
    }
    spans
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
fn graph_spans(t: &Tokens, prefix: &str) -> Vec<Span<'static>> {
    let lanes: [Color; 6] = [
        t.accent,
        t.syn.string,
        t.syn.function,
        t.syn.keyword,
        t.syn.r#type,
        t.warning,
    ];
    prefix
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if c == ' ' {
                return Span::styled(" ", Style::default());
            }
            let color = lanes[(i / 2) % lanes.len()];
            let style = if c == '*' {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            Span::styled(c.to_string(), style)
        })
        .collect()
}

/// Path left — directory secondary, filename text (the row idiom) — then
/// `+N -M` past at least one space. The stats are virtual EOL content
/// (the buffer row holds only the path), so the caret and the path
/// stay byte-aligned no matter how wide the stats run. Path labels and
/// stats are printable chrome, measured in display cells.
fn file_row_spans(t: &Tokens, file: &ChangedFile, width: usize) -> Vec<Span<'static>> {
    let display = strop_core::layout::printable_text(file.path.to_string_lossy());
    let added = format!("+{} ", file.added);
    let deleted = format!("-{}", file.deleted);
    // 0068 B4: a long path yields its prefix (never the stats) — the
    // filename end survives, the `+N -M` pair always shows.
    let stats_w = added.len() + deleted.len() + 1;
    let display = if clip::width(&display) > width.saturating_sub(stats_w) {
        clip::clip_start(&display, width.saturating_sub(stats_w)).into_owned()
    } else {
        display.to_string()
    };
    let path_cells = clip::width(&display);
    // the row idiom (0088 §2 "File names"): directory secondary, name text
    let mut spans =
        crate::render::file_name::spans(&display, crate::render::file_name::Role::Row, t).to_vec();
    let pad = width
        .saturating_sub(path_cells + added.len() + deleted.len())
        .max(1);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(added, Style::default().fg(t.diff_add)));
    spans.push(Span::styled(deleted, Style::default().fg(t.diff_del)));
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn tokens() -> Tokens {
        Tokens::new(strop_core::theme::ThemeId::Hone)
    }

    #[test]
    fn log_row_quiets_metadata_and_keeps_bytes() {
        let source = "* 51b63a8 t · 35 seconds ago · polish the modeline";
        let t = tokens();
        let spans = log_row_spans(&t, source);
        // the buffer line's exact bytes come first — caret/yank truth
        assert!(joined(&spans).starts_with(source));
        let sha = spans.iter().find(|s| s.content == "51b63a8").unwrap();
        assert_eq!(sha.style.fg, Some(t.accent));
        assert!(sha.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "polish the modeline")
                .unwrap()
                .style
                .fg,
            Some(t.text)
        );
        for muted in [" t", "35 seconds ago"] {
            assert_eq!(
                spans.iter().find(|s| s.content == muted).unwrap().style.fg,
                Some(t.muted),
                "{muted} must be quiet"
            );
        }
    }

    #[test]
    fn log_subject_keeps_embedded_separators() {
        let source = "* abc1234 a · 1s ago · fix · real · subject";
        let t = tokens();
        let spans = log_row_spans(&t, source);
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
            Some(t.text)
        );
    }

    #[test]
    fn graph_art_and_plain_rows_stay_unmuted() {
        let t = tokens();
        let graph = log_row_spans(&t, "| |\\");
        assert_eq!(joined(&graph), "| |\\");
        assert!(graph.iter().all(|s| s.style.fg != Some(t.muted)));
        let plain = log_row_spans(&t, "loading log…");
        assert_eq!(joined(&plain), "loading log…");
        assert!(plain.iter().all(|s| s.style.fg == Some(t.text)));
    }

    #[test]
    fn file_row_keeps_the_directory_secondary_and_the_filename_text() {
        let file = ChangedFile {
            path: "src/render/diff.rs".into(),
            added: 12,
            deleted: 3,
        };
        let t = tokens();
        let spans = file_row_spans(&t, &file, 40);
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
            Some(t.secondary)
        );
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "diff.rs")
                .unwrap()
                .style
                .fg,
            Some(t.text)
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
        let spans = file_row_spans(&tokens(), &file, 40);
        let pad = spans
            .iter()
            .find(|s| !s.content.is_empty() && s.content.chars().all(|c| c == ' '))
            .unwrap();
        assert_eq!(clip::width(&pad.content), 40 - (7 + 5));
    }
}
