//! The list surfaces' adapters (0010 §5, 0032 §2, drawn by 0100
//! §4.3–§4.4): the commit log (its rows in `diff/log`) and a commit's
//! files. The text begins with the buffer line's exact bytes; anything
//! after that prefix is render-only (a fact), so search, yank and the
//! caret keep reading the real buffer.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::editor::Surface;
use crate::render::block::gutter::{self, Sign};
use crate::render::block::{heading, Painted};
use strop_git::memory::ChangedFile;

use super::super::clip;
use super::super::style::Tokens;

/// The least width of a list block with facts (0100 §3.9).
const BLOCK_FLOOR: usize = 48;

/// What a list surface's rows are laid out against, measured once per
/// frame: the files block's width, the log's author column, the clock.
#[derive(Clone, Copy, Default)]
pub(crate) struct ListLayout {
    files_block: usize,
    author: usize,
    now: i64,
}

impl ListLayout {
    /// `pane` is the gutter and text cells; `now` the editor's clock in
    /// unix seconds.
    pub(crate) fn measure(surface: Option<&Surface>, pane: usize, now: i64) -> Self {
        match surface {
            Some(Surface::ChangedFiles { sha, files, .. }) => Self {
                files_block: files_block(files, sha, pane),
                ..Self::default()
            },
            Some(Surface::CommitLog { rows, .. }) => Self {
                author: super::log::author_cells(rows, pane),
                now,
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}

/// The width of the commit files' block, gutter included: the longest
/// row + 2 + the widest fact, at least 48 cells, at most the pane — so
/// the counts sit beside the names at any terminal width (F3). Measured
/// once per frame.
fn files_block(files: &[ChangedFile], sha: &str, pane: usize) -> usize {
    let mut widest = heading_words(sha).len() + 2 + 3 + files_fact_width(files);
    for file in files {
        let path = clip::width(&strop_core::layout::printable_text(
            file.path.to_string_lossy(),
        ));
        widest = widest.max(path + 2 + stats_text(file.added, file.deleted).len());
    }
    (strop_engine::editor::prepare::LIST_GUTTER + widest)
        .max(BLOCK_FLOOR)
        .min(pane)
}

fn heading_words(sha: &str) -> String {
    format!("commit {}", sha.get(..10).unwrap_or(sha))
}

fn stats_text(added: usize, deleted: usize) -> String {
    format!("+{added} -{deleted}")
}

fn totals(files: &[ChangedFile]) -> (usize, usize) {
    files
        .iter()
        .fold((0, 0), |(a, d), f| (a + f.added, d + f.deleted))
}

fn files_count(files: &[ChangedFile]) -> String {
    match files.len() {
        1 => "1 file".to_owned(),
        n => format!("{n} files"),
    }
}

fn files_fact_width(files: &[ChangedFile]) -> usize {
    let (added, deleted) = totals(files);
    files_count(files).len() + 2 + stats_text(added, deleted).len()
}

/// Paint row `line` of a commit log or a commit's files; `cursor` marks
/// the cursor row.
pub(crate) fn painted(
    t: &Tokens,
    surface: &Surface,
    line: usize,
    cursor: bool,
    layout: ListLayout,
    text_cells: usize,
) -> Option<Painted<'static>> {
    let sign = if cursor { Sign::Cursor } else { Sign::Blank };
    let mut painted = match surface {
        Surface::CommitLog { rows, columns, .. } => Painted {
            gutter: gutter::list(t, sign),
            text: super::log::row_spans(
                t,
                rows.get(line)?,
                *columns,
                layout.author,
                layout.now,
                text_cells,
            ),
            ..Default::default()
        },
        Surface::ChangedFiles { sha, files, .. } => {
            let text_block = layout
                .files_block
                .saturating_sub(strop_engine::editor::prepare::LIST_GUTTER)
                .min(text_cells);
            if line == 0 {
                let (added, deleted) = totals(files);
                let mut fact = heading::fact(t, files_count(files));
                fact.push(Span::raw("  "));
                fact.extend(heading::stats(t, added, deleted));
                let heading = heading::Heading {
                    gutter: strop_engine::editor::prepare::LIST_GUTTER,
                    ruled: true,
                    name: heading::Name::Words(vec![
                        Span::styled("commit ", Style::default().fg(t.muted)),
                        Span::styled(
                            sha.get(..10).unwrap_or(sha).to_owned(),
                            Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    marks: Vec::new(),
                    fact,
                    focus: heading::Focus::Here,
                };
                // the rule stays on the cursor row (see diff/status.rs)
                Painted::heading(t, &heading, text_block)
            } else {
                Painted {
                    gutter: gutter::list(t, sign),
                    text: file_row_spans(t, files.file_at_row(line)?, text_block),
                    ..Default::default()
                }
            }
        }
        Surface::Diff { .. } | Surface::Status { .. } => return None,
    };
    if cursor {
        painted.row_bg = Some(t.cursorline);
    }
    Some(painted)
}

/// Path left — directory secondary, filename text (the row idiom) — then
/// `+N -M` ending at the block's edge, at least two cells after the
/// path. The stats are render-only (the buffer row holds only the
/// path), so the caret and the path stay byte-aligned. A path too long
/// for its block yields its start, never the stats (0068 B4).
fn file_row_spans(t: &Tokens, file: &ChangedFile, cells: usize) -> Vec<Span<'static>> {
    let display = strop_core::layout::printable_text(file.path.to_string_lossy());
    let stats = stats_text(file.added, file.deleted);
    let room = cells.saturating_sub(stats.len() + 2);
    let display = if clip::width(&display) > room {
        clip::clip_start(&display, room).into_owned()
    } else {
        display.into_owned()
    };
    let path_cells = clip::width(&display);
    // the row idiom (0088 §2 "File names"): directory secondary, name text
    let mut spans =
        crate::render::file_name::spans(&display, crate::render::file_name::Role::Row, t).to_vec();
    let pad = cells.saturating_sub(path_cells + stats.len()).max(2);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(heading::stats(t, file.added, file.deleted));
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
    fn file_row_keeps_the_directory_secondary_and_the_filename_text() {
        let file = ChangedFile {
            path: "src/render/diff.rs".into(),
            added: 12,
            deleted: 3,
            ..Default::default()
        };
        let t = tokens();
        let spans = file_row_spans(&t, &file, 40);
        let text = joined(&spans);
        assert!(text.starts_with("src/render/diff.rs"));
        assert!(text.ends_with("+12 -3"));
        assert_eq!(clip::width(&text), 40);
        let ink = |needle: &str| spans.iter().find(|s| s.content == needle).unwrap().style.fg;
        assert_eq!(ink("src/render/"), Some(t.secondary));
        assert_eq!(ink("diff.rs"), Some(t.text));
    }

    #[test]
    fn file_row_padding_counts_display_cells() {
        // 界 is two cells: the path is 7 cells, the stats 5 — a char
        // count (5 for the path) would push the stats 2 cells left
        let file = ChangedFile {
            path: "界/x.rs".into(),
            added: 1,
            deleted: 0,
            ..Default::default()
        };
        let spans = file_row_spans(&tokens(), &file, 40);
        assert_eq!(clip::width(&joined(&spans)), 40);
    }

    /// F3: the block takes its width from its content, between the
    /// 48-cell floor and the pane.
    #[test]
    fn the_files_block_is_as_wide_as_its_content() {
        let file = |path: &str| ChangedFile {
            path: path.into(),
            added: 2,
            deleted: 0,
            ..Default::default()
        };
        let short = [file("a.rs"), file("src/lib.rs")];
        assert_eq!(files_block(&short, "19b4e9aec5", 200), 48);
        let long = [file(&"x".repeat(90))];
        assert_eq!(files_block(&long, "19b4e9aec5", 200), 2 + 90 + 2 + 5);
        assert_eq!(files_block(&long, "19b4e9aec5", 80), 80);
    }
}
