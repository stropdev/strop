//! The document lifecycle (0014 wave 2): the Document struct, the
//! arena accessors, open/close/scratch, MRU. One document owns its text,
//! highlighter, and git surface — no parallel vectors.

use strop_core::Buffer;

mod indentation;
pub(crate) mod remote;
pub mod surfaces;
pub use indentation::{detect_indent, Detection, Indent, IndentOverride, IndentSource};
mod directory;
pub use directory::Directory;
pub use remote::RemoteDocument;
mod close;
mod snapshot;

pub use super::jumps::JumpRecord;
pub use surfaces::{DiffRow, DocumentSource, Surface};

use super::Editor;

/// One document: the text buffer plus everything that used to live in
/// parallel vectors keyed by buffer index (0014 wave 2). One struct,
/// one arena — the alignment invariant is the type system now.
pub struct Document {
    pub buf: Buffer,
    pub(crate) syntax_hint: Option<std::path::PathBuf>,
    /// Resolved at open/reload and after `:tab-size`/`:indent-style`
    /// (0051 R08): manual override → confident detection → config.
    pub indent: Indent,
    /// Explicit `:tab-size N` / `:indent-style …` choices; they survive
    /// reloads and config refreshes, and other buffers never touch them.
    pub indent_override: IndentOverride,
    /// What the last detection pass concluded (None when `indent_detect`
    /// is off) — `:explain` reports it with its confidence or reason.
    pub detection: Option<Detection>,
    /// What backs this document (0021 §4): the surface payload lives in
    /// the source variant; readonly derives from it at construction.
    pub source: DocumentSource,
    /// A notification hint observed the file change underneath a dirty
    /// (or vanished-from-under) buffer (0058 §2): the buffer is preserved
    /// untouched, the save path's disk-baseline check keeps its refusal,
    /// and only a fresh observation (guarded reload, save) clears this.
    pub external_change: bool,
    /// Loaded in the background by a collection build (0101 §6.1): the
    /// buffer revision at load. Cleared for good once the document is on
    /// screen after an event, or once it is edited; while it stands and
    /// nothing holds the document, the collection's close releases it.
    pub(crate) loan: Option<strop_core::id::BufferRevision>,
}

impl Document {
    pub fn label(&self, cwd: &std::path::Path) -> String {
        if let DocumentSource::Terminal(terminal) = &self.source {
            // `term://{cwd}//{N}:{program}` — the buffer carries its own
            // Neovim-style name; the launch identity is immutable.
            self.buf.name.clone().unwrap_or_else(|| {
                super::terminal::terminal_name(terminal.session, &terminal.launch, "sh")
            })
        } else if let Some(remote) = self.remote_metadata() {
            remote.file.to_string()
        } else {
            self.buf
                .path
                .as_ref()
                .map(|path| path.strip_prefix(cwd).unwrap_or(path).display().to_string())
                .or_else(|| self.buf.name.clone())
                .unwrap_or_else(|| "[scratch]".into())
        }
    }

    pub(crate) fn documentation(mut buffer: Buffer) -> Self {
        buffer.name = Some("documentation".into());
        let mut document = Self::output(buffer);
        document.syntax_hint = Some(std::path::PathBuf::from("documentation.md"));
        document
    }
    pub fn new(buf: Buffer) -> Self {
        let detection = Some(detect_indent(buf.text()));
        Self {
            buf,
            syntax_hint: None,
            indent: Indent::default(),
            indent_override: IndentOverride::default(),
            detection,
            source: DocumentSource::File,
            external_change: false,
            loan: None,
        }
    }

    /// A scratch document (no file).
    pub fn scratch(buf: Buffer) -> Self {
        let detection = Some(detect_indent(buf.text()));
        Self {
            buf,
            syntax_hint: None,
            indent: Indent::default(),
            indent_override: IndentOverride::default(),
            detection,
            source: DocumentSource::Scratch,
            external_change: false,
            loan: None,
        }
    }

    /// A git-memory surface: job-owned content, readonly derived from
    /// the source — not set by hand (0021 §4).
    pub fn surface(mut buf: Buffer, surface: Surface, context: strop_git::GitContext) -> Self {
        buf.set_readonly(strop_core::ReadonlyReason::GitSurface);
        Self {
            buf,
            syntax_hint: None,
            indent: Indent::default(),
            source: DocumentSource::Surface(Box::new(surfaces::GitSurface {
                context,
                content: surface,
            })),
            external_change: false,
            loan: None,
            indent_override: IndentOverride::default(),
            detection: None,
        }
    }

    /// Named virtual content (`:!cmd` output, help, the undo browser):
    /// readonly derived from the source. Temporary surfaces opened
    /// through [`Editor::open_temporary_output`] also carry the return
    /// point `:q` restores (0051 §7 R07).
    pub fn output(mut buf: Buffer) -> Self {
        buf.set_readonly(strop_core::ReadonlyReason::Output);
        Self {
            buf,
            syntax_hint: None,
            indent: Indent::default(),
            indent_override: IndentOverride::default(),
            detection: None,
            source: DocumentSource::Output { return_to: None },
            external_change: false,
            loan: None,
        }
    }

    /// Syntax identity is data; parsers live on the display-analysis worker.
    pub fn syntax_path(&self) -> Option<&std::path::Path> {
        if let Some(path) = &self.syntax_hint {
            return Some(path);
        }
        match &self.source {
            DocumentSource::Remote(source) => Some(source.file.path()),
            DocumentSource::Surface(source) => match &source.content {
                Surface::Diff {
                    commit: Some(commit),
                    ..
                } => Some(&commit.current),
                Surface::Diff { hunks, .. } => Some(std::path::Path::new(hunks.label())),
                _ => None,
            },
            DocumentSource::File | DocumentSource::Scratch => self.buf.path.as_deref(),
            DocumentSource::Directory(_)
            | DocumentSource::Output { .. }
            | DocumentSource::Terminal(_) => None,
        }
    }

    pub fn matches_target(&self, target: &crate::files::FileTarget) -> bool {
        match (&self.source, target) {
            (DocumentSource::Remote(source), crate::files::FileTarget::Remote(location)) => {
                location.absolute_file() == Some(&source.file)
            }
            (DocumentSource::Directory(source), target) => {
                target.matches_location(&source.location)
            }
            (DocumentSource::File, crate::files::FileTarget::Local(path)) => {
                self.buf.path.as_ref() == Some(path)
                    || self.buf.file_identity() == Some(path.as_path())
            }
            _ => false,
        }
    }

    pub(crate) fn file_target(&self, cwd: &std::path::Path) -> Option<crate::files::FileTarget> {
        use crate::files::FileTarget;
        match &self.source {
            DocumentSource::Remote(source) => Some(FileTarget::Remote(source.file.clone().into())),
            DocumentSource::Directory(source) => FileTarget::from_location(&source.location).ok(),
            _ => self
                .buf
                .file_identity()
                .or(self.buf.path.as_deref())
                .map(|path| FileTarget::Local(cwd.join(path))),
        }
    }

    /// The surface payload, when this document is one.
    pub fn surface_payload(&self) -> Option<&Surface> {
        match &self.source {
            DocumentSource::Surface(s) => Some(&s.content),
            _ => None,
        }
    }

    /// Mutable surface payload, when this document is one.
    pub fn surface_payload_mut(&mut self) -> Option<&mut Surface> {
        match &mut self.source {
            DocumentSource::Surface(s) => Some(&mut s.content),
            _ => None,
        }
    }

    pub(crate) fn git_context(&self) -> Option<&strop_git::GitContext> {
        match &self.source {
            DocumentSource::Surface(surface) => Some(&surface.context),
            _ => None,
        }
    }
}

impl Editor {
    /// Mark a document most-recently-used. Engine-internal (0056 AR02):
    /// MRU order follows admitted navigation, never a frontend write.
    pub(crate) fn touch_mru(&mut self, i: strop_core::id::DocumentId) {
        self.mru.retain(|&x| x != i);
        self.mru.insert(0, i);
    }

    /// The current document. Invariant: the editor always has one live
    /// document while it runs (closing the last one sets should_quit).
    pub fn cur(&self) -> &Document {
        self.docs
            .get(self.current())
            .expect("invariant: current document is live")
    }

    pub(crate) fn cur_mut(&mut self) -> super::transact::DocumentEdit<'_> {
        self.doc_mut(self.current())
    }

    pub fn buf(&self) -> &Buffer {
        &self.cur().buf
    }

    /// The transaction gateway for the current document (0056 AR02):
    /// engine-internal — the frontend never mutates documents directly;
    /// tests use [`Editor::fixture_buf_mut`].
    pub(crate) fn buf_mut(&mut self) -> super::transact::BufferEdit<'_> {
        super::transact::BufferEdit::new(self.cur_mut())
    }

    /// Fixture hook (tests only): the same transaction gateway the
    /// engine uses, so fixture edits keep the sync-on-drop semantics.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture_buf_mut(&mut self) -> super::transact::BufferEdit<'_> {
        self.buf_mut()
    }

    /// One document by id, for ids the engine itself hands out and keeps
    /// live: pane/window bindings ([`Editor::view`], [`Editor::panes`],
    /// prepared-view windows) are rebound before their document closes,
    /// so a stale id here is an engine bug, not user input — that
    /// invariant is what the panic reports (0014 wave 2). Ids the
    /// frontend holds across turns (a remembered row, a cached source)
    /// can legitimately outlive their document: look those up through
    /// [`Editor::document`] and handle `None`.
    pub fn doc(&self, id: strop_core::id::DocumentId) -> &Document {
        self.docs.get(id).expect("stale document id")
    }

    /// Readonly document lookup (0056 AR02): None for a stale id —
    /// the frontend's tolerated form of a closed document.
    pub fn document(&self, id: strop_core::id::DocumentId) -> Option<&Document> {
        self.docs.get(id)
    }

    /// Any live documents at all (quit-loop condition).
    pub fn has_documents(&self) -> bool {
        !self.docs.is_empty()
    }

    /// Admit an already-constructed document (0056 AR02/AR13): checked
    /// identity allocation is the whole operation — failure changes
    /// nothing, so the caller decides how to surface it.
    pub fn admit_document(
        &mut self,
        document: Document,
    ) -> Result<strop_core::id::DocumentId, strop_core::id::ArenaExhausted> {
        self.docs.try_insert(document)
    }

    pub(crate) fn doc_mut(
        &mut self,
        id: strop_core::id::DocumentId,
    ) -> super::transact::DocumentEdit<'_> {
        super::transact::DocumentEdit::new(self, id)
    }

    /// Tests: the first live document's id (the "buffers[0]" of the
    /// index era).
    #[cfg(any(test, feature = "test-support"))]
    pub fn first_doc(&self) -> strop_core::id::DocumentId {
        self.docs
            .iter()
            .next()
            .map(|(id, _)| id)
            .expect("test document")
    }

    /// The active document (derived: the active view's document).
    #[inline]
    pub fn current(&self) -> strop_core::id::DocumentId {
        self.view().doc
    }

    /// Switch the active view to a document.
    pub fn switch_to(&mut self, id: strop_core::id::DocumentId) {
        let switching = self.current() != id;
        if switching {
            self.remember_directory_view();
            self.remember_terminal_view();
        }
        self.cancel_pending();
        self.cancel_open(strop_core::worker::CancelReason::Superseded);
        self.focus_epoch += 1;
        if switching {
            let view = self.view_mut();
            view.sels = strop_core::selection::SelectionSet::default();
            view.view_top = 0;
            view.hscroll = strop_core::id::DisplayColumn::new(0);
            view.desired_column = None;
            view.terminal_input = false;
        }
        self.view_mut().doc = id;
        if switching {
            self.restore_terminal_view(id);
            self.note_file_visit(id);
        }
        self.touch_mru(id);
    }

    /// Count a visit toward this project's frecent files (0093 §3.4): a
    /// local file under the working directory, by its relative path.
    pub(crate) fn note_file_visit(&mut self, id: strop_core::id::DocumentId) {
        let Some(document) = self.docs.get(id) else {
            return;
        };
        if document.remote_metadata().is_some() {
            return;
        }
        let Some(relative) = document
            .buf
            .path
            .as_ref()
            .and_then(|path| path.strip_prefix(&self.cwd).ok())
        else {
            return;
        };
        let relative = strop_picker::display_path(relative).into_owned();
        self.picker_ranking.recent.visit(&relative);
    }

    /// Open a temporary readonly output surface — help, explain, the
    /// undo browser, a review or save report (0051 §7 R07): the jump is
    /// recorded AND the new document carries the exact origin view as
    /// its navigation record, so ctrl-o AND `:q` hand back the caret,
    /// viewport and horizontal origin the user came from. A stale
    /// scratch origin dies with its buffer; close_buffer skips dead
    /// return points on its own. None on identity exhaustion (0056
    /// AR13): no jump recorded, no view switched, message set.
    pub(crate) fn open_temporary_output(
        &mut self,
        buf: strop_core::Buffer,
    ) -> Option<strop_core::id::DocumentId> {
        let mut document = Document::output(buf);
        document.set_return_point(self.jump_record());
        let Ok(id) = self.docs.try_insert(document) else {
            self.message = "document identity space exhausted".into();
            return None;
        };
        self.push_jump();
        // A temporary surface returns to where it came from: the pristine
        // scratch stays, or closing help on a fresh launch quits.
        self.switch_to(id);
        self.set_head(0);
        Some(id)
    }

    /// The active view's selections.
    #[inline]
    pub fn sels(&self) -> &strop_core::selection::SelectionSet {
        &self.view().sels
    }

    /// Selection mutation is grammar-owned (0056 AR02): multicursor
    /// commands go through the engine, never a frontend write.
    #[inline]
    pub(crate) fn sels_mut(&mut self) -> &mut strop_core::selection::SelectionSet {
        &mut self.view_mut().sels
    }

    /// Fixture hook (tests only): plant extra carets without driving
    /// the multicursor grammar.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture_sels_mut(&mut self) -> &mut strop_core::selection::SelectionSet {
        self.sels_mut()
    }

    /// The active pane's text-area height in rows (render-loop fed).
    pub fn view_rows(&self) -> usize {
        self.view_rows
    }

    pub fn view_top(&self) -> usize {
        self.view().view_top
    }

    /// `:bd` / `:bd!`: delete the document, not just a pane. A running
    /// terminal refuses without `!`; with it the child is stopped and the
    /// buffer goes (Neovim's E89 and `:bd!`, 0090 §4).
    pub(crate) fn delete_buffer(&mut self, force: bool) {
        let document = self.current();
        if self
            .terminal_phase(document)
            .is_some_and(strop_terminal::model::Phase::live)
        {
            if !force {
                self.message = "terminal is running — :bd! stops it".into();
                return;
            }
            self.kill_terminal_document(document);
        }
        self.close_buffer(force);
    }

    /// Any path-backed or scratch document holding unsaved content.
    pub fn any_dirty(&self) -> bool {
        self.docs.iter().any(|(_, d)| d.buf.is_dirty())
    }

    /// Fixture convenience exercises the production request and completion path.
    #[cfg(any(test, feature = "test-support"))]
    pub fn open_fixture(
        &mut self,
        path: &std::path::Path,
    ) -> Result<strop_core::id::DocumentId, String> {
        self.request_open(
            path.to_owned(),
            super::io::OpenIntent::Switch { readonly: false },
        );
        self.wait_io()?;
        Ok(self.current())
    }
}

#[cfg(test)]
mod pathbuf_tests {
    //! 0026: non-UTF-8 filenames (linux names are bytes) work end to end.
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn non_utf8_filename_opens_highlights_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(std::ffi::OsStr::from_bytes(b"\xff\xfe.rs"));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        let id = e.open_fixture(&path).expect("opens by bytes");
        e.switch_to(id);
        // extension detection works through the OsStr, not a lossy str
        assert!(e.analysis_fixture().spans.iter().any(|span| span.start == 0
            && span.end == 2
            && span.class == strop_syntax::Class::Keyword));
        e.feed_text("dd"); // delete the line
        e.feed_text(":w\r");
        e.wait_io().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }
}
