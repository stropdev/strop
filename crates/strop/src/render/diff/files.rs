//! A commit's view (0100 §4.4): the commit's heading with its author and
//! age, the subject bold and the body, then the `Files` heading and one
//! row per file — its status letter in the change's ink, the path in the
//! list idiom, `R old → new` for a rename, and `+N -M` or `binary` at the
//! block's edge. The two headings and the file rows share one block as
//! wide as its content (§3.9, F3).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use strop_engine::editor::prepare::LIST_GUTTER;
use strop_engine::editor::{FilesRow, PreparedFiles};
use strop_git::memory::{ChangedFile, FileStatus};

use super::super::clip;
use super::super::style::Tokens;
use crate::render::block::gutter::{self, Sign};
use crate::render::block::{self, heading, Painted};
use crate::render::file_name::{self, Role};

/// The least width of a list block with facts (0100 §3.9).
const BLOCK_FLOOR: usize = 48;

/// The block's width, gutter included: its widest row (a heading's words
/// and least leader, or a file row) + 2 + its fact, at least 48 cells, at
/// most the pane — so the counts sit beside the names at any width.
pub(crate) fn block_cells(files: &PreparedFiles, pane: usize, now: i64) -> usize {
    let commit = heading_words(files.sha()).len() + 2 + 3 + clip::width(&commit_fact(files, now));
    let list = "Files".len() + 2 + 3 + files_fact_width(files);
    let mut widest = commit.max(list);
    for file in files.iter() {
        let row = clip::width(&strop_engine::editor::git_memory::file_row_text(file));
        widest = widest.max(row + 2 + file_fact_text(file).len());
    }
    (LIST_GUTTER + widest).max(BLOCK_FLOOR).min(pane)
}

/// Paint row `line` of a commit's view. `cursor_line` says which heading
/// is the one the cursor is under; `block` is [`block_cells`].
pub(crate) fn painted(
    t: &Tokens,
    files: &PreparedFiles,
    line: usize,
    (cursor, cursor_line): (bool, usize),
    (block, text_cells): (usize, usize),
    now: i64,
) -> Option<Painted<'static>> {
    let sign = if cursor { Sign::Cursor } else { Sign::Blank };
    let text_block = block.saturating_sub(LIST_GUTTER).min(text_cells);
    let in_files = matches!(
        files.row(cursor_line),
        Some(FilesRow::Files | FilesRow::File(_))
    );
    let focus = |here: bool| {
        if here {
            heading::Focus::Here
        } else {
            heading::Focus::Elsewhere
        }
    };
    let list = |text: Vec<Span<'static>>, cut: bool| Painted {
        gutter: gutter::list(t, sign),
        text,
        cut,
        ..Default::default()
    };
    let bold = Style::default().fg(t.text).add_modifier(Modifier::BOLD);
    let painted = match files.row(line)? {
        FilesRow::Commit => {
            let sha = files.sha();
            let heading = heading::Heading {
                gutter: LIST_GUTTER,
                rule: heading::Rule::Light,
                name: heading::Name::Words(vec![
                    Span::styled("commit ", Style::default().fg(t.muted)),
                    Span::styled(sha.get(..10).unwrap_or(sha).to_owned(), bold),
                ]),
                marks: Vec::new(),
                fact: heading::fact(t, commit_fact(files, now)),
                focus: focus(!in_files),
            };
            Painted::heading(t, &heading, text_block)
        }
        FilesRow::Subject => list(
            vec![Span::styled(
                strop_core::layout::printable_text(files.summary().subject.as_str()).into_owned(),
                bold,
            )],
            true,
        ),
        // a body line is the buffer's own text, drawn in `fg.text`
        FilesRow::Body => Painted {
            row_fg: Some(t.text),
            ..list(Vec::new(), true)
        },
        FilesRow::Blank => list(Vec::new(), false),
        FilesRow::Files => {
            let (added, deleted) = totals(files);
            let mut fact = heading::fact(t, files_count(files));
            fact.push(Span::raw("  "));
            fact.extend(heading::stats(t, added, deleted));
            let heading = heading::Heading {
                gutter: LIST_GUTTER,
                rule: heading::Rule::Light,
                name: heading::Name::Words(vec![Span::styled("Files", bold)]),
                marks: Vec::new(),
                fact,
                focus: focus(in_files),
            };
            Painted::heading(t, &heading, text_block)
        }
        FilesRow::File(index) => list(file_row_spans(t, files.get(index)?, text_block), false),
    };
    let mut painted = painted;
    // the rule stays on a heading under the cursor (see diff/status.rs)
    if cursor {
        painted.row_bg = Some(t.cursorline);
    }
    Some(painted)
}

fn heading_words(sha: &str) -> String {
    format!("commit {}", sha.get(..10).unwrap_or(sha))
}

/// `Ada Example · 3 days ago`.
fn commit_fact(files: &PreparedFiles, now: i64) -> String {
    let summary = files.summary();
    format!(
        "{} · {}",
        summary.author,
        super::log::age_words(now.saturating_sub(summary.time))
    )
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
    files_count(files).len() + 2 + format!("+{added} -{deleted}").len()
}

/// A file's fact: `binary`, or its line counts.
fn file_fact_text(file: &ChangedFile) -> String {
    if file.binary {
        "binary".to_owned()
    } else {
        format!("+{} -{}", file.added, file.deleted)
    }
}

fn file_fact(t: &Tokens, file: &ChangedFile) -> Vec<Span<'static>> {
    if file.binary {
        vec![Span::styled("binary", Style::default().fg(t.muted))]
    } else {
        heading::stats(t, file.added, file.deleted)
    }
}

/// The status letter's ink, as the status buffer inks its letters.
fn letter_ink(t: &Tokens, status: FileStatus) -> Color {
    match status {
        FileStatus::Added => t.diff_add,
        FileStatus::Deleted => t.diff_del,
        FileStatus::Unmerged => t.warning,
        FileStatus::Other => t.muted,
        FileStatus::Modified
        | FileStatus::Renamed
        | FileStatus::Copied
        | FileStatus::TypeChanged => t.diff_change,
    }
}

/// The letter, then the path in the row idiom (directory secondary, name
/// text), `old → new` for a rename or copy; the fact at the block's edge.
/// A path too long for its block yields its start, never the fact (0068
/// B4); a rename too long is cut at its end.
fn file_row_spans(t: &Tokens, file: &ChangedFile, cells: usize) -> Vec<Span<'static>> {
    let fact = file_fact(t, file);
    let mut lead = vec![Span::styled(
        format!("{} ", file.status.letter()),
        Style::default().fg(letter_ink(t, file.status)),
    )];
    let path = strop_core::layout::printable_text(file.path.to_string_lossy());
    match &file.old_path {
        Some(old) => {
            let old = strop_core::layout::printable_text(old.to_string_lossy());
            lead.extend(file_name::spans(&old, Role::Row, t));
            lead.push(Span::styled(" → ", Style::default().fg(t.muted)));
            lead.extend(file_name::spans(&path, Role::Row, t));
        }
        None => {
            let room = cells.saturating_sub(2 + heading::spans_width(&fact) + 2);
            let path = if clip::width(&path) > room {
                clip::clip_start(&path, room).into_owned()
            } else {
                path.into_owned()
            };
            lead.extend(file_name::spans(&path, Role::Row, t));
        }
    }
    block::fact_row(lead, fact, cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_engine::editor::git_memory::file_row_text;
    use strop_git::memory::CommitSummary;

    fn joined(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn tokens() -> Tokens {
        Tokens::new(strop_core::theme::ThemeId::Hone)
    }

    fn file(path: &str, added: usize) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            added,
            ..Default::default()
        }
    }

    #[test]
    fn a_file_row_is_its_letter_and_path_then_its_counts() {
        let t = tokens();
        let file = ChangedFile {
            deleted: 3,
            ..file("src/render/diff.rs", 12)
        };
        let spans = file_row_spans(&t, &file, 40);
        let text = joined(&spans);
        assert!(text.starts_with(&file_row_text(&file)), "{text}");
        assert!(text.ends_with("+12 -3"));
        assert_eq!(clip::width(&text), 40);
        let ink = |needle: &str| spans.iter().find(|s| s.content == needle).unwrap().style.fg;
        assert_eq!(ink("M "), Some(t.diff_change));
        assert_eq!(ink("src/render/"), Some(t.secondary));
        assert_eq!(ink("diff.rs"), Some(t.text));
    }

    /// A rename reads `R old → new`; a binary file says so.
    #[test]
    fn renames_and_binaries_say_what_they_are() {
        let t = tokens();
        let renamed = ChangedFile {
            status: FileStatus::Renamed,
            old_path: Some("docs/old.md".into()),
            ..file("docs/new.md", 0)
        };
        let text = joined(&file_row_spans(&t, &renamed, 48));
        assert!(text.starts_with("R docs/old.md → docs/new.md"), "{text}");
        assert!(text.ends_with("+0 -0"), "{text}");
        let binary = ChangedFile {
            binary: true,
            ..file("logo.bin", 0)
        };
        let spans = file_row_spans(&t, &binary, 48);
        assert!(joined(&spans).ends_with("  binary"));
        assert_eq!(spans.last().unwrap().style.fg, Some(t.muted));
    }

    #[test]
    fn padding_counts_display_cells() {
        // 界 is two cells: a char count would push the stats left
        let spans = file_row_spans(&tokens(), &file("界/x.rs", 1), 40);
        assert_eq!(clip::width(&joined(&spans)), 40);
    }

    /// A path too long for its block keeps its end and its counts.
    #[test]
    fn a_long_path_yields_its_start() {
        let text = joined(&file_row_spans(&tokens(), &file(&"x".repeat(90), 1), 40));
        assert_eq!(clip::width(&text), 40);
        assert!(text.starts_with("M …x"), "{text}");
        assert!(text.ends_with("x  +1 -0"), "{text}");
    }

    /// F3: the block takes its width from its content, between the
    /// 48-cell floor and the pane.
    #[test]
    fn the_block_is_as_wide_as_its_content() {
        let commit = |files| {
            PreparedFiles::new(
                CommitSummary {
                    sha: "19b4e9aec5".into(),
                    author: "Ada".into(),
                    ..Default::default()
                },
                files,
            )
        };
        let short = commit(vec![file("a.rs", 2), file("src/lib.rs", 2)]);
        assert_eq!(block_cells(&short, 200, 0), 48);
        let long = commit(vec![file(&"x".repeat(90), 2)]);
        assert_eq!(block_cells(&long, 200, 0), 2 + 2 + 90 + 2 + 5);
        assert_eq!(block_cells(&long, 80, 0), 80);
    }
}
