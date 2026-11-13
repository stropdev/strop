//! The commit log's adapter (0100 §4.3 step B): typed rows drawn as
//! aligned columns. The buffer row is `graph sha  (refs) subject`, padded
//! by the engine so the shas and subjects line up; the author and the
//! age are the row's fact, right-aligned and render-only (§3.8), the age
//! counted from the commit time against the editor's clock.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use strop_engine::editor::LogColumns;
use strop_git::log::{LogCommit, LogRow};

use super::super::clip;
use super::super::style::Tokens;

/// The widest the author column grows; longer names end in `…`.
const AUTHOR_MAX: usize = 16;
/// Under this many columns the author leaves the row (0100 §3.9).
const AUTHOR_PANE: usize = 60;
/// The age column: `59s`, `23h`, `11mo` right-aligned.
const AGE_CELLS: usize = 4;

/// The author column's width this frame: the widest author among the
/// rows, at most [`AUTHOR_MAX`]; none on a pane under [`AUTHOR_PANE`].
pub(crate) fn author_cells(rows: &[LogRow], pane: usize) -> usize {
    if pane < AUTHOR_PANE {
        return 0;
    }
    rows.iter()
        .filter_map(|row| row.commit.as_ref())
        .map(|commit| clip::width(&commit.author))
        .max()
        .unwrap_or(0)
        .min(AUTHOR_MAX)
}

/// An age in one unit, as the blame column words it: `1s 5m 3h 2d 4w
/// 6mo 1y`. A time in the future reads as `0s`.
pub(crate) fn short_age(seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let seconds = seconds.max(0);
    match seconds {
        s if s < MINUTE => format!("{s}s"),
        s if s < HOUR => format!("{}m", s / MINUTE),
        s if s < DAY => format!("{}h", s / HOUR),
        s if s < 7 * DAY => format!("{}d", s / DAY),
        s if s < 30 * DAY => format!("{}w", s / (7 * DAY)),
        s if s < 365 * DAY => format!("{}mo", s / (30 * DAY)),
        s => format!("{}y", s / (365 * DAY)),
    }
}

/// One log row's text spans: the buffer row's own bytes, styled — lane-
/// coloured graph, the sha grey, refs bold with the branch `HEAD` points
/// to in the accent, the subject in text — then, for a commit, the
/// author and age at the right edge of `cells`.
pub(crate) fn row_spans(
    t: &Tokens,
    row: &LogRow,
    columns: LogColumns,
    author: usize,
    now: i64,
    cells: usize,
) -> Vec<Span<'static>> {
    let mut lead = graph_spans(t, &row.graph);
    let Some(commit) = &row.commit else {
        return crate::render::block::heading::clip_spans(lead, cells);
    };
    pad(&mut lead, columns.graph.saturating_sub(row.graph.len()));
    lead.push(Span::styled(
        commit.short.clone(),
        Style::default().fg(t.secondary),
    ));
    pad(
        &mut lead,
        columns.sha.saturating_sub(commit.short.len()) + 2,
    );
    lead.extend(refs_spans(t, commit));
    lead.push(Span::styled(
        commit.subject.clone(),
        Style::default().fg(t.text),
    ));
    crate::render::block::fact_row(lead, fact(t, commit, author, now), cells)
}

fn pad(spans: &mut Vec<Span<'static>>, cells: usize) {
    if cells > 0 {
        spans.push(Span::raw(" ".repeat(cells)));
    }
}

/// `(HEAD -> main, tag: v1) `: bold `fg.secondary`, the checked-out
/// branch in the accent — the log's one accent (0100 L4).
fn refs_spans(t: &Tokens, commit: &LogCommit) -> Vec<Span<'static>> {
    if commit.refs.is_empty() {
        return Vec::new();
    }
    let quiet = Style::default()
        .fg(t.secondary)
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled("(", quiet)];
    for (at, name) in commit.refs.iter().enumerate() {
        if at > 0 {
            spans.push(Span::styled(", ", quiet));
        }
        match name.strip_prefix("HEAD -> ") {
            Some(branch) => {
                spans.push(Span::styled("HEAD -> ", quiet));
                spans.push(Span::styled(
                    branch.to_owned(),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ));
            }
            None => spans.push(Span::styled(name.clone(), quiet)),
        }
    }
    spans.push(Span::styled(") ", quiet));
    spans
}

/// The author, padded to its column, two cells, the age right-aligned.
fn fact(t: &Tokens, commit: &LogCommit, author: usize, now: i64) -> Vec<Span<'static>> {
    let quiet = Style::default().fg(t.muted);
    let mut spans = Vec::new();
    if author > 0 {
        let name = clip::clip_end(&commit.author, author);
        let cells = clip::width(&name);
        spans.push(Span::styled(name.into_owned(), quiet));
        pad(&mut spans, author - cells + 2);
    }
    spans.push(Span::styled(
        format!("{:>AGE_CELLS$}", short_age(now.saturating_sub(commit.time))),
        quiet,
    ));
    spans
}

/// Lane-coloured graph art: each two-column lane cycles the palette, the
/// commit node `*` is bold in its lane's colour (gitui/lazygit lesson —
/// lane colour is how the eye tracks a branch through merges). The
/// accent is not a lane: it has one job, where you are (0088 rule 2).
fn graph_spans(t: &Tokens, graph: &str) -> Vec<Span<'static>> {
    let lanes: [Color; 6] = [
        t.syn.string,
        t.syn.function,
        t.syn.keyword,
        t.syn.r#type,
        t.warning,
        t.syn.number,
    ];
    graph
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if c == ' ' {
                return Span::raw(" ");
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

#[cfg(test)]
mod tests {
    use super::*;
    use strop_engine::editor::git_memory::log_text;

    fn joined(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn tokens() -> Tokens {
        Tokens::new(strop_core::theme::ThemeId::Hone)
    }

    fn commit(graph: &str, refs: &[&str], subject: &str, author: &str, time: i64) -> LogRow {
        LogRow {
            graph: graph.into(),
            commit: Some(LogCommit {
                sha: "51b63a8000".into(),
                short: "51b63a8".into(),
                refs: refs.iter().map(|r| r.to_string()).collect(),
                subject: subject.into(),
                author: author.into(),
                time,
            }),
        }
    }

    const NOW: i64 = 1_000_000;

    /// The painted row begins with the buffer row's exact bytes (caret
    /// and yank truth), and the author and age end at the last cell.
    #[test]
    fn the_row_is_its_text_then_author_and_age() {
        let t = tokens();
        let row = commit(
            "| * ",
            &["HEAD -> main"],
            "polish",
            "Ada Example",
            NOW - 120,
        );
        let columns = LogColumns { graph: 4, sha: 7 };
        let spans = row_spans(&t, &row, columns, 11, NOW, 70);
        let text = joined(&spans);
        assert!(
            text.starts_with(&log_text::row_text(&row, columns)),
            "{text}"
        );
        assert!(text.ends_with("Ada Example    2m"), "{text}");
        assert_eq!(clip::width(&text), 70);
        let ink = |needle: &str| spans.iter().find(|s| s.content == needle).unwrap().style;
        assert_eq!(ink("51b63a8").fg, Some(t.secondary), "the sha is grey");
        assert_eq!(ink("main").fg, Some(t.accent), "the branch HEAD is on");
        assert_eq!(ink("HEAD -> ").fg, Some(t.secondary));
        assert_eq!(ink("polish").fg, Some(t.text));
        assert_eq!(ink("Ada Example").fg, Some(t.muted));
        // one accent: the graph lanes never take it
        assert!(spans
            .iter()
            .filter(|s| "|*/\\".contains(s.content.as_ref()))
            .all(|s| s.style.fg != Some(t.accent)));
    }

    /// A subject too long for the pane is cut with `…` before the fact.
    #[test]
    fn a_long_subject_is_cut_before_the_fact() {
        let t = tokens();
        let row = commit("* ", &[], &"word ".repeat(30), "Ada", NOW - 3 * 86_400);
        let columns = LogColumns { graph: 2, sha: 7 };
        let text = joined(&row_spans(&t, &row, columns, 3, NOW, 48));
        assert_eq!(clip::width(&text), 48);
        assert!(text.ends_with("…  Ada    3d"), "{text}");
    }

    /// Without an author column the age alone stays.
    #[test]
    fn no_author_column_keeps_the_age() {
        let t = tokens();
        let row = commit("* ", &[], "fix", "Ada", NOW - 5);
        let text = joined(&row_spans(
            &t,
            &row,
            LogColumns { graph: 2, sha: 7 },
            0,
            NOW,
            40,
        ));
        assert!(text.ends_with("  5s"), "{text}");
        assert!(!text.contains("Ada"));
    }

    #[test]
    fn the_author_leaves_a_narrow_pane() {
        let rows = vec![commit("* ", &[], "s", "A very long author name indeed", 0)];
        assert_eq!(author_cells(&rows, 59), 0);
        assert_eq!(author_cells(&rows, 60), AUTHOR_MAX);
    }

    #[test]
    fn ages_are_one_unit() {
        let day = 86_400;
        for (seconds, age) in [
            (-5, "0s"),
            (1, "1s"),
            (300, "5m"),
            (3 * 3600, "3h"),
            (2 * day, "2d"),
            (29 * day, "4w"),
            (180 * day, "6mo"),
            (364 * day, "12mo"),
            (400 * day, "1y"),
        ] {
            assert_eq!(short_age(seconds), age, "{seconds}");
            assert!(age.len() <= AGE_CELLS);
        }
    }

    #[test]
    fn connector_rows_are_their_graph() {
        let t = tokens();
        let row = LogRow {
            graph: "| |\\".into(),
            commit: None,
        };
        let spans = row_spans(&t, &row, LogColumns { graph: 4, sha: 7 }, 3, NOW, 40);
        assert_eq!(joined(&spans), "| |\\");
        assert!(spans.iter().all(|s| s.style.fg != Some(t.muted)));
    }
}
