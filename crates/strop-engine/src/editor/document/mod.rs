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
mod snapshot;
pub(crate) use snapshot::last_position;

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
}

impl Document {
    pub fn label(&self, cwd: &std::path::Path) -> String {
        if let Some(remote) = self.remote_metadata() {
            remote.file.to_string()
        } else if let DocumentSource::Container { container, path } = &self.source {
            strop_workspace::ResourceLocation {
                filesystem: strop_workspace::Filesystem::Container(container.clone()),
                path: path.clone(),
            }
            .label()
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
        }
    }

    /// A git-memory surface: job-owned content, readonly derived from
    /// the source — not set by hand (0021 §4).
    pub fn surface(mut buf: Buffer, surface: Surface, context: strop_git::GitContext) -> Self {
        buf.readonly = true;
        Self {
            buf,
            syntax_hint: None,
            indent: Indent::default(),
            source: DocumentSource::Surface(Box::new(surfaces::GitSurface {
                context,
                content: surface,
            })),
            indent_override: IndentOverride::default(),
            detection: None,
        }
    }

    /// Named virtual content (`:!cmd` output, help, the undo browser):
    /// readonly derived from the source. Temporary surfaces opened
    /// through [`Editor::open_temporary_output`] also carry the return
    /// point `:q` restores (0051 §7 R07).
    pub fn output(mut buf: Buffer) -> Self {
        buf.readonly = true;
        Self {
            buf,
            syntax_hint: None,
            indent: Indent::default(),
            indent_override: IndentOverride::default(),
            detection: None,
            source: DocumentSource::Output { return_to: None },
        }
    }

    /// A file read from a container (0037 DC1a/b): readonly derived from
    /// the source; the path names container bytes, never a local file.
    pub fn container_file(
        mut buf: Buffer,
        container: strop_workspace::ContainerId,
        path: std::path::PathBuf,
    ) -> Self {
        buf.readonly = true;
        let detection = Some(detect_indent(buf.text()));
        Self {
            buf,
            syntax_hint: None,
            indent: Indent::default(),
            indent_override: IndentOverride::default(),
            detection,
            source: DocumentSource::Container { container, path },
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
            DocumentSource::Container { path, .. } => Some(path),
            DocumentSource::Directory(_) | DocumentSource::Output { .. } => None,
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
            (
                DocumentSource::Container { container, path },
                crate::files::FileTarget::Container {
                    container: expected,
                    path: requested,
                },
            ) => container == expected && path == requested,
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
            DocumentSource::Container { container, path } => Some(FileTarget::Container {
                container: container.clone(),
                path: path.clone(),
            }),
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

    pub fn buf_mut(&mut self) -> super::transact::BufferEdit<'_> {
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn first_doc(&self) -> strop_core::id::DocumentId {
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
        if self.current() != id {
            self.remember_directory_view();
        }
        self.cancel_pending();
        self.cancel_open(strop_core::worker::CancelReason::Superseded);
        self.focus_epoch += 1;
        if self.current() != id {
            let view = self.view_mut();
            view.sels = strop_core::selection::SelectionSet::default();
            view.view_top = 0;
            view.hscroll = strop_core::id::DisplayColumn::new(0);
            view.desired_column = None;
        }
        self.view_mut().doc = id;
        self.touch_mru(id);
    }

    /// Open a temporary readonly output surface — help, explain, the
    /// undo browser, a review or save report (0051 §7 R07): the jump is
    /// recorded AND the new document carries the exact origin view as
    /// its navigation record, so ctrl-o AND `:q` hand back the caret,
    /// viewport and horizontal origin the user came from. A stale
    /// scratch origin dies with its buffer; close_buffer skips dead
    /// return points on its own.
    pub(crate) fn open_temporary_output(
        &mut self,
        buf: strop_core::Buffer,
    ) -> strop_core::id::DocumentId {
        self.push_jump();
        let mut document = Document::output(buf);
        document.set_return_point(self.jump_record());
        let id = self.docs.insert(document);
        self.drop_stale_scratch(id);
        self.switch_to(id);
        self.set_head(0);
        id
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
        self.remember_directory_view();
        if !force && self.docs.len() == 1 && self.filesystem.unconfirmed() > 0 {
            self.message =
                "filesystem outcomes are unconfirmed; :fs verify before quitting, or :q! to force"
                    .into();
            return false;
        }
        // A collection's dirty bit is presentation state (0049 §5): the
        // sources hold the real unsaved edits — the view always closes.
        let view_only = self.collections.contains_key(&self.current());
        if self.buf().dirty && !force && !view_only {
            self.message = "unsaved changes — :q! to force".into();
            return false;
        }
        self.cancel_pending();
        self.cancel_open(strop_core::worker::CancelReason::OwnerClosed);
        if self.docs.len() == 1 {
            self.request_session_save();
        }
        let closed = self.current();
        let mut affected_collections = Vec::new();
        for (id, collection) in &mut self.collections {
            let before = collection.excerpts.len();
            collection
                .excerpts
                .retain(|excerpt| excerpt.source != closed);
            collection
                .pending_commit
                .retain(|(source, _)| *source != closed);
            if collection.excerpts.len() != before {
                collection.match_count = collection
                    .excerpts
                    .iter()
                    .map(|excerpt| excerpt.matches.len().max(excerpt.hit_anchors.len()))
                    .sum();
                affected_collections.push(*id);
            }
        }
        self.revoke_remote_write(closed);
        self.analysis
            .forget(super::analysis::AnalysisTarget::Document(closed));
        self.stop_remote_follow(closed);
        self.cancel_directory_filter(closed);
        self.lsp_close_document(closed);
        self.shell_document_closed(closed);
        self.revoke_git_requests_for(closed);
        self.blame_gutters.remove(&closed);
        self.collections.remove(&closed);
        self.review.forget(closed);
        self.filesystem_forget_view(closed);
        let return_to = self
            .docs
            .remove(closed)
            .and_then(|document| document.return_point().cloned());
        if self.docs.is_empty() {
            self.collection_build = None;
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
            if let Some(ret) = return_to.filter(|ret| self.docs.get(ret.document).is_some()) {
                self.jump_to(ret);
            } else if self.doc(next).directory_metadata_ref().is_some() {
                self.restore_directory_view(next);
            }
        }
        for collection in affected_collections {
            self.collection_render_view(collection);
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
        if self.ctrl_c_armed || (!self.any_dirty() && self.filesystem.unconfirmed() == 0) {
            return true;
        }
        self.ctrl_c_armed = true;
        self.message = if self.filesystem.unconfirmed() > 0 {
            "filesystem outcomes are unconfirmed — ctrl-c again to force-quit".into()
        } else {
            "unsaved changes — ctrl-c again to force-quit".into()
        };
        false
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
