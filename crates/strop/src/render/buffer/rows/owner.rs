//! Who owns a pane's row (0100 §6.1). A buffer is exactly one of a file,
//! the help buffer, a directory listing, the review buffer, a collection
//! or a git surface — what its source is decides, not the order of a
//! chain of checks. The owner is computed as data, then painted into one
//! `Painted` row the pane composes overlays over.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::{collection, commit_message, directory, PaneView};
use crate::editor::{CollectionRowInfo, Directory, Editor, ReviewRow, StatusView, Surface};
use crate::render::block::gutter;
use crate::render::block::{diff_line, Painted};
use crate::render::diff;
use crate::render::style::Tokens;
use strop_engine::editor::prepare::Gutter;

/// A block surface's row (0100 §3.1): drawn by its surface's adapter.
pub(super) enum Block<'a> {
    Collection(&'a CollectionRowInfo),
    Diff(&'a Surface),
    Status(&'a StatusView),
    /// The commit log and a commit's files.
    List(&'a Surface),
}

pub(super) enum Owner<'a> {
    /// A file's line: sign, line number, the cursor line. A commit
    /// message adds its three inks (0100 §4.6).
    File {
        commit_message: bool,
    },
    /// The help buffer's house-style decoration (render/help.rs).
    Help,
    /// A directory listing's row.
    Directory(&'a Directory),
    /// A row of the review buffer (0094).
    Review(ReviewRow),
    Block(Block<'a>),
}

/// What a pane is, read once per frame.
pub(super) struct Pane<'a> {
    pub gutter: Gutter,
    pub surface: Option<&'a Surface>,
    pub directory: Option<&'a Directory>,
    pub help: bool,
    pub commit_message: bool,
}

impl<'a> Pane<'a> {
    /// The owner of line `line`; `info` is the collection's row facts.
    pub(super) fn owner(
        &self,
        editor: &Editor,
        doc: strop_core::id::DocumentId,
        line: usize,
        info: Option<&'a CollectionRowInfo>,
    ) -> Owner<'a> {
        let file = Owner::File {
            commit_message: self.commit_message,
        };
        match (self.gutter, self.surface) {
            (Gutter::Source, _) => info.map_or(file, |info| Owner::Block(Block::Collection(info))),
            (Gutter::Pair, Some(surface)) => Owner::Block(Block::Diff(surface)),
            (Gutter::List, Some(Surface::Status { view, .. })) => Owner::Block(Block::Status(view)),
            (Gutter::List, Some(surface)) => Owner::Block(Block::List(surface)),
            (Gutter::Pair | Gutter::List, None) => file,
            (Gutter::Lines, _) => match (self.directory, self.help) {
                (Some(directory), _) => Owner::Directory(directory),
                (None, true) => Owner::Help,
                (None, false) => editor.review_row(doc, line).map_or(file, Owner::Review),
            },
        }
    }
}

/// One row's inputs.
pub(super) struct Row<'t> {
    pub line: usize,
    pub text: ropey::RopeSlice<'t>,
    /// The pane's cursor is on this row and the pane shows overlays.
    pub cursor: bool,
    /// The line the pane's cursor is on.
    pub cursor_line: usize,
}

/// The widths a row is laid out in: the number gutter, the text column
/// and a list surface's layout.
#[derive(Clone, Copy)]
pub(super) struct Cells {
    pub gutter: usize,
    pub text: usize,
    pub list: diff::ListLayout,
}

impl<'a> Owner<'a> {
    pub(super) fn paint(
        &self,
        editor: &Editor,
        t: &Tokens,
        view: &PaneView,
        row: &Row<'_>,
        cells: Cells,
    ) -> Painted<'a> {
        let line = row.line;
        match self {
            Owner::Block(Block::Collection(info)) => collection::painted(
                editor,
                t,
                view,
                info,
                line,
                row.text,
                sign(editor, t, view, line),
                (cells.gutter, cells.text),
            ),
            Owner::Block(Block::Diff(surface)) => {
                diff::painted(t, surface, line, row.cursor, cells.gutter, cells.text)
                    .unwrap_or_default()
            }
            Owner::Block(Block::Status(status)) => {
                diff::status_painted(t, status, line, row.cursor_line, row.cursor, cells.text)
                    .unwrap_or_else(|| list_blank(t, row.cursor))
            }
            Owner::Block(Block::List(surface)) => {
                diff::list_painted(t, surface, line, row.cursor, cells.list, cells.text)
                    .unwrap_or_else(|| list_blank(t, row.cursor))
            }
            Owner::Directory(directory) => {
                let mut painted =
                    directory::row(t, directory, line, row.text, row.cursor).unwrap_or_default();
                painted.gutter = numbered(editor, t, view, line, cells.gutter);
                painted
            }
            Owner::Help => Painted {
                gutter: numbered(editor, t, view, line, cells.gutter),
                text: crate::render::help::row_spans(t, &row.text.to_string(), cells.text as u16),
                ..Default::default()
            },
            Owner::Review(review) => {
                let (row_fg, row_bg) = review_inks(t, *review);
                Painted {
                    gutter: numbered(editor, t, view, line, cells.gutter),
                    row_fg,
                    row_bg: if row.cursor {
                        Some(t.cursorline)
                    } else {
                        row_bg
                    },
                    ..Default::default()
                }
            }
            Owner::File { commit_message } => Painted {
                gutter: numbered(editor, t, view, line, cells.gutter),
                text: commit_message
                    .then(|| commit_message::spans(t, line, &row.text.to_string()))
                    .flatten()
                    .unwrap_or_default(),
                // the cursor line is painted in file buffers (0088 rule 1);
                // selection, search and flash still override per cell
                row_bg: row.cursor.then_some(t.cursorline),
                ..Default::default()
            },
        }
    }

    /// The band under a directory row or the cursor line covers the
    /// gutter too, so the row reads as one surface; help keeps its own.
    pub(super) fn band_covers_gutter(&self, cursor: bool) -> bool {
        match self {
            Owner::Directory(_) => true,
            Owner::Help => false,
            _ => cursor,
        }
    }
}

/// A list surface's row with nothing to draw (the loading row).
fn list_blank(t: &Tokens, cursor: bool) -> Painted<'static> {
    Painted {
        gutter: gutter::list(
            t,
            if cursor {
                gutter::Sign::Cursor
            } else {
                gutter::Sign::Blank
            },
        ),
        row_bg: cursor.then_some(t.cursorline),
        ..Default::default()
    }
}

fn review_inks(t: &Tokens, row: ReviewRow) -> (Option<Color>, Option<Color>) {
    use strop_git::LineOrigin;
    match row {
        ReviewRow::Heading | ReviewRow::File => (Some(t.accent), None),
        ReviewRow::Hunk => (Some(t.muted), Some(t.surface)),
        ReviewRow::Removed => (
            Some(t.diff_del),
            diff_line::origin_bg(t, LineOrigin::Deletion),
        ),
        ReviewRow::Added => (
            Some(t.diff_add),
            diff_line::origin_bg(t, LineOrigin::Addition),
        ),
        ReviewRow::Warning => (Some(t.warning), None),
        _ => (None, None),
    }
}

/// The lines gutter: the sign column, then the line number.
fn numbered(
    editor: &Editor,
    t: &Tokens,
    view: &PaneView,
    line: usize,
    width: usize,
) -> Vec<Span<'static>> {
    let cursor = editor.doc(view.doc).buf.line_of(view.cursor) == line;
    gutter::numbered(
        t,
        sign(editor, t, view, line),
        Some(line + 1),
        cursor,
        width,
    )
}

/// The sign column: diagnostics win over git signs (merged gutter,
/// 0009), and only the pane's own buffer shows them.
fn sign(editor: &Editor, t: &Tokens, view: &PaneView, line: usize) -> Span<'static> {
    let (mark, color) = mark(editor, t, view, line);
    Span::styled(
        mark,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn mark(editor: &Editor, t: &Tokens, view: &PaneView, line_idx: usize) -> (&'static str, Color) {
    if let Some(directory) = editor
        .doc(view.doc)
        .directory_metadata_ref()
        .filter(|source| source.draft.is_none())
    {
        if let Some(entry) = directory.entry(strop_core::id::LineIndex::new(line_idx)) {
            if directory.marked.contains_key(&entry.name) {
                return ("●", t.accent);
            }
            if entry.error.is_some() {
                return ("!", t.diff_del);
            }
        }
        if view.overlays && editor.doc(view.doc).buf.line_of(view.cursor) == line_idx {
            return ("▌", t.accent);
        }
    }
    if let Some(sev) = editor.diag_severity_at(view.doc, line_idx + 1) {
        // severity dot (VSCode/gitui lesson: color reads faster than
        // letters) — the cursor line's EOL note carries the words
        return ("●", t.severity(sev));
    }
    // git signs: + add, ~ change, - deletion below (only for the
    // working buffer — surfaces have no path, so no leak)
    if view.doc == editor.current() {
        // the four states in one column: unstaged sign wins; staged-only
        // lines get the committed-adjacent tint (0014 wave 4)
        if editor.sign_at(line_idx + 1).is_none() && editor.sign_at_staged(line_idx + 1) {
            return ("▎", t.dim(t.diff_add));
        }
        match editor.sign_at(line_idx + 1) {
            Some('+') => return ("▎", t.diff_add),
            Some('~') => return ("▎", t.diff_change),
            Some('-') => return ("▎", t.diff_del),
            _ => {}
        }
    }
    (" ", t.muted)
}
