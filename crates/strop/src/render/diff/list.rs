//! The list surfaces' adapters (0010 §5, 0032 §2, drawn by 0100
//! §4.3–§4.4): the commit log (its rows in `diff/log`) and a commit's
//! view (`diff/files`). The text begins with the buffer line's exact
//! bytes; anything after that prefix is render-only (a fact), so search,
//! yank and the caret keep reading the real buffer.

use crate::editor::Surface;
use crate::render::block::gutter::{self, Sign};
use crate::render::block::Painted;

use super::super::style::Tokens;

/// What a list surface's rows are laid out against, measured once per
/// frame: the commit view's block, the log's author column, the clock.
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
            Some(Surface::ChangedFiles { files, .. }) => Self {
                files_block: super::files::block_cells(files, pane, now),
                now,
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

/// Paint row `line` of a commit log or a commit's view; `cursor` marks
/// the cursor row, which is `cursor_line`.
pub(crate) fn painted(
    t: &Tokens,
    surface: &Surface,
    line: usize,
    (cursor, cursor_line): (bool, usize),
    layout: ListLayout,
    text_cells: usize,
) -> Option<Painted<'static>> {
    match surface {
        Surface::CommitLog { rows, columns, .. } => Some(Painted {
            gutter: gutter::list(t, if cursor { Sign::Cursor } else { Sign::Blank }),
            text: super::log::row_spans(
                t,
                rows.get(line)?,
                *columns,
                layout.author,
                layout.now,
                text_cells,
            ),
            row_bg: cursor.then_some(t.cursorline),
            ..Default::default()
        }),
        Surface::ChangedFiles { files, .. } => super::files::painted(
            t,
            files,
            line,
            (cursor, cursor_line),
            (layout.files_block, text_cells),
            layout.now,
        ),
        Surface::Diff { .. } | Surface::Status { .. } => None,
    }
}
