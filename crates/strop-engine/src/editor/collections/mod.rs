//! Source-backed collections (0094): the editable multibuffer and the quickfix
//! list. An edit inside an excerpt writes through to its source (`write.rs`);
//! a source edit made anywhere else is projected into every view of it
//! (`updates.rs`). Card borders, gap rows and the title are not text.
//! `Ctrl-O` in a results list makes one (`build.rs`); its hits are stepped
//! and opened from `quickfix.rs` (0097).

mod build;
mod context;
mod journal;
pub(crate) mod loans;
mod navigation;
mod projection;
mod quickfix;
#[cfg(test)]
mod tests;
mod updates;
mod view_positions;
mod write;

pub use write::ExcerptRefusal;

use super::document::Document;
use super::Editor;
use strop_core::id::{BufferRevision, DocumentId};

/// What a collection view row IS (0049 §6): the renderer styles chrome
/// from this — never by parsing row text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionRow {
    Title,
    /// A file card's top border; carries the excerpt index it precedes.
    CardTop(usize),
    /// An omitted-lines gap between two excerpts of one file.
    Gap,
    /// A file card's bottom border.
    CardBottom,
    Body,
}

/// One excerpt: a whole-line span of a source document, remapped through
/// the source's change journal like any other saved anchor.
#[derive(Debug, Clone)]
pub(crate) struct Excerpt {
    pub source: DocumentId,
    /// Source byte span (whole lines), remapped on every source mutation.
    pub start: usize,
    pub end: usize,
    /// Context radius around the query's source hits.
    pub context: usize,
    /// Original hit line anchors, remapped with source edits.
    pub hit_anchors: Vec<usize>,
    /// Header line index in the view (the title is line 0).
    pub view_line: usize,
    /// Source line count as rendered.
    pub view_lines: usize,
    /// Projected source body byte span in the view, excluding synthetic newline.
    pub view_start: usize,
    pub view_end: usize,
    /// Query hit spans in SOURCE bytes within this excerpt (the picker's
    /// match evidence paints in the view, 0050 §7).
    pub matches: Vec<(usize, usize)>,
}

#[derive(Debug, Clone)]
pub(crate) struct Collection {
    pub title: String,
    pub excerpts: Vec<Excerpt>,
    /// Hits the list had that could not be shown: their file changed since
    /// the search, or could not be opened.
    pub skipped: usize,
    /// The buffer revision at last sync — the cheap no-change check that
    /// keeps motions from materializing rope text on the input path.
    pub revision: BufferRevision,
    /// The last view change the excerpt geometry accounts for. Past it the
    /// journal holds edits `write.rs` has still to place in the sources.
    pub mapped: BufferRevision,
    /// A write did not reach its source, or left a body its source does not
    /// spell: the view is rendered again when its lease settles.
    pub rerender: bool,
    /// A body may carry one line more than its source (`write.rs`).
    pub recheck: bool,
    /// The running command's first edit was refused: so is the rest of it.
    pub voided: bool,
    /// The running command has changed the view.
    pub touched: bool,
    /// `u` (true) or `Ctrl-R` (false) is moving the view's history.
    pub history_move: Option<bool>,
    /// The current hit (0097 §3): where the cursor last was among the
    /// hits, in the collection or in a file.
    pub cursor: Option<usize>,
    /// Row roles parallel to the view text (0049 §6), rebuilt at render.
    pub rows: Vec<CollectionRow>,
}

/// A source location plus the exact search witness when this came from Search.
#[derive(Debug)]
pub(crate) struct CollectionHit {
    location: strop_workspace::ResourceLocation,
    line: usize,
    span: Option<(usize, usize)>,
    witness: Option<super::picker::ReplacementHit>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SourceHit {
    line: usize,
    span: Option<(usize, usize)>,
}

/// An in-flight collection build: hits plus the count of background
/// source loads still outstanding (0044 v2 async source loading).
#[derive(Debug)]

pub(crate) struct CollectionBuild {
    pub title: String,
    /// (path, line, match col+len in source bytes when known)
    pub hits: Vec<CollectionHit>,
    pub waiting: usize,
    pub owner: strop_core::worker::WorkerId,
    pub origin: DocumentId,
    pub revision: BufferRevision,
    pub focus_on_ready: bool,
    /// The hit under the picker's bar, as (file, line, column): where the
    /// cursor lands.
    pub picked: Option<(strop_workspace::ResourceLocation, usize, Option<usize>)>,
}

/// `1 hit`, `4 hits`: counts are spelled, never `hit(s)` (0097 §3.4).
pub(crate) fn counted(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        _ => format!("{count} {noun}s"),
    }
}

/// `4 hits in 2 files`.
pub(crate) fn hits_in_files(hits: usize, files: usize) -> String {
    format!("{} in {}", counted(hits, "hit"), counted(files, "file"))
}

/// What the renderer needs per collection row (0049 §6).
pub struct CollectionRowInfo {
    pub kind: CollectionRow,
    /// Body rows: (source document, source byte start, source byte end).
    pub source: Option<(strop_core::id::DocumentId, usize, usize)>,
    /// Query hit spans within the row, in SOURCE bytes (0050 §7).
    pub source_matches: Vec<(usize, usize)>,
    /// The caret sits inside this row's card (focus chrome).
    pub card_active: bool,
    pub source_dirty: bool,
    pub source_readonly: bool,
}
