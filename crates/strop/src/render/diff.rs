//! Diff surface rendering (0010 §4/§5, drawn by 0100 §4.5): one block
//! per file — the file's ruled heading with `+N -M` as its fact, each
//! hunk header a break, the lines with both sides' numbers, the quiet
//! tint and intra-line emphasis. All decoration comes from typed hunk
//! data — never from sniffing `+`/`-` in the text. Log/file row
//! decoration lives in `diff/list`, the status buffer's in `diff/status`;
//! the blame column and commit file sidebar in `diff/margins`
//! (0011/0032).

use ratatui::style::Style;

use crate::editor::{DiffRow, Surface};
use crate::render::block::gutter::{self};
use crate::render::block::{diff_line, heading, Painted};
use crate::render::file_name::Label;

use super::style::Tokens;

mod files;
mod list;
mod log;
mod margins;
mod status;

pub(crate) use list::{painted as list_painted, ListLayout};
pub(crate) use margins::{blame_blank, blame_spans, sidebar_row_spans};
pub(crate) use status::painted as status_painted;

/// The diff view's adapter: row `row` of a diff surface as a block row.
/// The cursor row lights its numbers and lies on the cursor line, except
/// over a changed line, which keeps its tint.
pub(crate) fn painted<'a>(
    t: &Tokens,
    surface: &'a Surface,
    row: usize,
    cursor: bool,
    gutter_cells: usize,
    text_cells: usize,
) -> Option<Painted<'a>> {
    let Surface::Diff { hunks, commit, .. } = surface else {
        return None;
    };
    let mut painted = match surface.diff_row(row)? {
        DiffRow::Heading => {
            let label = Label::parse(hunks.label());
            // a binary file says so (the commit's list knows it); any
            // other delta without line changes — a mode, a pure rename —
            // says that instead of `+0 -0`
            let binary = commit.as_ref().is_some_and(|commit| {
                commit
                    .files
                    .file_at_path(&commit.current)
                    .is_some_and(|file| file.binary)
            });
            let quiet = |words: &'static str| {
                vec![ratatui::text::Span::styled(
                    words,
                    Style::default().fg(t.muted),
                )]
            };
            let fact = if binary {
                quiet("binary")
            } else if hunks.is_empty() {
                quiet("no line changes")
            } else {
                heading::stats(t, hunks.added(), hunks.deleted())
            };
            let heading = heading::Heading {
                gutter: gutter_cells,
                ruled: true,
                name: heading::Name::Path(&label),
                marks: Vec::new(),
                fact,
                focus: heading::Focus::Here,
            };
            Painted::heading(t, &heading, text_cells)
        }
        DiffRow::HunkHeader(hunk) => Painted::band(
            t,
            gutter::jump(t, gutter_cells, &gutter::pair_jump(gutter_cells)),
            diff_line::hunk_header(t, hunk),
            text_cells,
        ),
        DiffRow::Line(line) => Painted {
            gutter: gutter::pair(t, line, cursor, gutter_cells),
            row_bg: diff_line::origin_bg(t, line.origin),
            line: Some((line, hunks.emphasis(row))),
            ..Default::default()
        },
    };
    if cursor
        && painted
            .line
            .is_none_or(|(line, _)| line.origin == strop_git::LineOrigin::Context)
    {
        painted.row_bg = Some(t.cursorline);
    }
    Some(painted)
}
