//! The document lifecycle (0014 wave 2): the Document struct, the
//! arena accessors, open/close/scratch, MRU. One document owns its text,
//! highlighter, and git surface — no parallel vectors.

use strop_core::Buffer;

pub(crate) mod remote;
pub mod surfaces;
pub use remote::{RemoteDirectory, RemoteDocument};

pub use surfaces::{DiffRow, DocumentSource, ReturnPoint, Surface};

use super::Editor;

/// One document: the text buffer plus everything that used to live in
/// parallel vectors keyed by buffer index (0014 wave 2). One struct,
/// one arena — the alignment invariant is the type system now.
pub struct Document {
    pub buf: Buffer,
    /// What backs this document (0021 §4): the surface payload lives in
    /// the source variant; readonly derives from it at construction.
    pub source: DocumentSource,
}

impl Document {
    pub fn new(buf: Buffer) -> Self {
        Self {
            buf,
            source: DocumentSource::File,
        }
    }

    /// A scratch document (no file).
    pub fn scratch(buf: Buffer) -> Self {
        Self {
            buf,
            source: DocumentSource::Scratch,
        }
    }

    /// A git-memory surface: job-owned content, readonly derived from
    /// the source — not set by hand (0021 §4).
    pub fn surface(mut buf: Buffer, surface: Surface, context: strop_git::GitContext) -> Self {
        buf.readonly = true;
        Self {
            buf,
            source: DocumentSource::Surface(Box::new(surfaces::GitSurface {
                context,
                content: surface,
            })),
        }
    }

    /// Named virtual content (`:!cmd` output, help, the undo browser):
    /// readonly derived from the source.
    pub fn output(mut buf: Buffer) -> Self {
        buf.readonly = true;
        Self {
            buf,
            source: DocumentSource::Output,
        }
    }

    /// Syntax identity is data; parsers live on the display-analysis worker.
    pub(crate) fn syntax_path(&self) -> Option<&std::path::Path> {
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
            DocumentSource::RemoteDirectory(_) | DocumentSource::Output => None,
        }
    }

    pub fn matches_target(&self, target: &crate::files::FileTarget) -> bool {
        match (&self.source, target) {
            (DocumentSource::Remote(source), crate::files::FileTarget::Remote(location)) => {
                location.absolute_file() == Some(&source.file)
            }
            (
                DocumentSource::RemoteDirectory(source),
                crate::files::FileTarget::Remote(location),
            ) => location.absolute_file() == Some(&source.directory),
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
            DocumentSource::RemoteDirectory(source) => {
                Some(FileTarget::Remote(source.directory.clone().into()))
            }
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
    /// Mark a document most-recently-used.
    pub fn touch_mru(&mut self, i: strop_core::id::DocumentId) {
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

    pub(crate) fn buf_mut(&mut self) -> super::transact::BufferEdit<'_> {
        super::transact::BufferEdit::new(self.cur_mut())
    }

    /// One document by id — stale ids panic: an id outliving its
    /// document is a bug, and the generation check is what keeps it
    /// from silently resolving to the wrong one (0014 wave 2).
    pub fn doc(&self, id: strop_core::id::DocumentId) -> &Document {
        self.docs.get(id).expect("stale document id")
    }

    pub(crate) fn doc_mut(
        &mut self,
        id: strop_core::id::DocumentId,
    ) -> super::transact::DocumentEdit<'_> {
        super::transact::DocumentEdit::new(self, id)
    }

    /// Tests: the first live document's id (the "buffers[0]" of the
    /// index era).
    #[cfg(test)]
    pub(crate) fn first_doc(&self) -> strop_core::id::DocumentId {
        self.docs
            .iter()
            .next()
            .map(|(id, _)| id)
            .expect("test document")
    }

    /// vim's [No Name] rule: the untouched initial scratch buffer is
    /// replaced by the first real thing you open. `replacement` is the
    /// document taking over (0023 §1): panes on the scratch rebind to
    /// it FIRST — the drop used to strand them (the :vs crash probe).
    pub(crate) fn drop_stale_scratch(&mut self, replacement: strop_core::id::DocumentId) {
        // find the pristine scratch (pathless, untouched) wherever it is
        // — the replacement exists by now, so len() is no longer the
        // signal (0023: it must fire AFTER the insert so panes can rebind)
        let scratch = self.docs.iter().find_map(|(id, d)| {
            let b = &d.buf;
            (b.path.is_none()
                && !b.dirty
                && b.len_bytes() == 0
                && b.name.is_none()
                && id != replacement)
                .then_some(id)
        });
        let Some(scratch) = scratch else {
            return;
        };
        if self
            .pending
            .prompt()
            .is_some_and(|prompt| prompt.origin().pane.doc == scratch)
        {
            self.cancel_pending();
        }
        for pane in &mut self.panes {
            if pane.doc == scratch {
                pane.doc = replacement;
            }
        }
        self.lsp_close_document(scratch);
        self.docs.remove(scratch);
        self.mru.retain(|&x| x != scratch);
        if self.view().doc == scratch {
            self.view_mut().doc = replacement;
        }
    }

    /// The active document (derived: the active view's document).
    #[inline]
    pub fn current(&self) -> strop_core::id::DocumentId {
        self.view().doc
    }

    /// Switch the active view to a document.
    pub fn switch_to(&mut self, id: strop_core::id::DocumentId) {
        self.cancel_pending();
        self.cancel_open(strop_core::worker::CancelReason::Superseded);
        self.focus_epoch += 1;
        self.view_mut().doc = id;
        self.touch_mru(id);
    }

    /// The active view's selections.
    #[inline]
    pub fn sels(&self) -> &strop_core::selection::SelectionSet {
        &self.view().sels
    }

    #[inline]
    pub fn sels_mut(&mut self) -> &mut strop_core::selection::SelectionSet {
        &mut self.view_mut().sels
    }

    /// The active view's scroll offset.
    #[inline]
    /// The active pane's text-area height in rows (render-loop fed).
    pub fn view_rows(&self) -> usize {
        self.view_rows
    }

    pub fn view_top(&self) -> usize {
        self.view().view_top
    }

    /// Close the current document; quits when the last one closes.
    /// Returns false when unsaved changes block the close. Generational
    /// ids mean no reindexing anywhere (0014 wave 2).
    pub fn close_buffer(&mut self, force: bool) -> bool {
        if self.buf().dirty && !force {
            self.message = "unsaved changes — :q! to force".into();
            return false;
        }
        self.cancel_pending();
        self.cancel_open(strop_core::worker::CancelReason::OwnerClosed);
        if self.docs.len() == 1 {
            self.request_session_save();
        }
        let closed = self.current();
        self.revoke_remote_write(closed);
        self.analysis
            .forget(super::analysis::AnalysisTarget::Document(closed));
        self.stop_remote_follow(closed);
        self.cancel_remote_filter(closed);
        self.lsp_close_document(closed);
        self.shell_document_closed(closed);
        self.revoke_git_requests_for(closed);
        self.blame_gutters.remove(&closed);
        self.collections.remove(&closed);
        let return_to = self
            .docs
            .remove(closed)
            .and_then(|document| document.return_point().cloned());
        if self.docs.is_empty() {
            self.panes.clear();
            self.active_pane = 0;
            self.should_quit = true;
        } else {
            self.mru.retain(|&x| x != closed);
            self.generation += 1; // document set changed: old jobs are stale (0011 §2)
            let next = self.mru.first().copied().unwrap_or_else(|| {
                self.docs
                    .iter()
                    .next()
                    .map(|(id, _)| id)
                    .expect("docs non-empty")
            });
            for pane in &mut self.panes {
                if pane.doc == closed {
                    pane.doc = next;
                    pane.sels = Default::default();
                    pane.view_top = 0;
                    pane.hscroll = strop_core::id::DisplayColumn::new(0);
                    pane.desired_column = None;
                }
            }
            self.switch_to(next);
            self.set_head(0);
            self.view_mut().view_top = 0;
            self.view_mut().hscroll = strop_core::id::DisplayColumn::new(0);
            // a closing surface hands the cursor and view back to the
            // document it opened from — by id, no index math (0011 §1)
            if let Some(ret) = return_to {
                if self.docs.get(ret.buffer).is_some() {
                    if ret.buffer != self.current() {
                        self.view_mut().doc = ret.buffer;
                        self.touch_mru(ret.buffer);
                    }
                    self.set_head(ret.cursor.min(self.buf().len_bytes()));
                    self.view_mut().view_top = ret.view_top;
                    self.view_mut().hscroll = ret.hscroll;
                }
            }
        }
        self.lsp_retire_remote_servers();
        true
    }
    /// Any path-backed or scratch document holding unsaved content.
    pub fn any_dirty(&self) -> bool {
        self.docs.iter().any(|(_, d)| d.buf.dirty)
    }

    /// ctrl-c's quit intent (0015): warn once when dirty work exists,
    /// force on the second press. Returns true when the app may exit.
    pub fn ctrl_c_quit(&mut self) -> bool {
        if self.ctrl_c_armed || !self.any_dirty() {
            return true;
        }
        self.ctrl_c_armed = true;
        self.message = "unsaved changes — ctrl-c again to force-quit".into();
        false
    }

    /// Fixture convenience exercises the production request and completion path.
    #[cfg(test)]
    pub(crate) fn open_fixture(
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
