//! The document lifecycle (0014 wave 2): the Document struct, the
//! arena accessors, open/close/scratch, MRU. One document owns its text,
//! highlighter, and git surface — no parallel vectors.

use strop_core::Buffer;
use strop_syntax::Highlighter;

use super::{Editor, Surface};

/// One document: the text buffer plus everything that used to live in
/// parallel vectors keyed by buffer index (0014 wave 2). One struct,
/// one arena — the alignment invariant is the type system now.
pub struct Document {
    pub buf: Buffer,
    /// None: unsupported extension.
    pub highlighter: Option<Highlighter>,
    /// Git memory surface attached to this document (0010).
    pub surface: Option<Surface>,
}

impl Document {
    pub fn new(buf: Buffer) -> Self {
        let highlighter = buf.path.as_deref().and_then(Highlighter::for_path);
        Self {
            buf,
            highlighter,
            surface: None,
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

    pub fn cur_mut(&mut self) -> &mut Document {
        self.docs
            .get_mut(self.current())
            .expect("invariant: current document is live")
    }

    pub fn buf(&self) -> &Buffer {
        &self.cur().buf
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        &mut self.cur_mut().buf
    }

    /// One document by id — stale ids panic: an id outliving its
    /// document is a bug, and the generation check is what keeps it
    /// from silently resolving to the wrong one (0014 wave 2).
    pub fn doc(&self, id: strop_core::id::DocumentId) -> &Document {
        self.docs.get(id).expect("stale document id")
    }

    pub fn doc_mut(&mut self, id: strop_core::id::DocumentId) -> &mut Document {
        self.docs.get_mut(id).expect("stale document id")
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
    /// replaced by the first real thing you open — it never lingers as
    /// an extra :q with the welcome card on it.
    pub(crate) fn drop_stale_scratch(&mut self) {
        if self.docs.len() == 1 {
            let b = &self.cur().buf;
            if b.path.is_none() && !b.dirty && b.len_bytes() == 0 && b.name.is_none() {
                self.docs.clear();
                self.mru.clear();
            }
        }
    }

    /// Open without switching (splits): the document exists, the active
    /// view stays. Returns the id.
    pub fn open_document(&mut self, path: &str) -> std::io::Result<strop_core::id::DocumentId> {
        self.drop_stale_scratch();
        let canon = std::path::Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| self.cwd.join(path));
        if let Some((id, _)) = self.docs.iter().find(|(_, d)| {
            d.buf
                .path
                .as_deref()
                .and_then(|p| std::path::Path::new(p).canonicalize().ok())
                == Some(canon.clone())
        }) {
            return Ok(id);
        }
        let buf = Buffer::open(path)?;
        let id = self.docs.insert(Document::new(buf));
        if self.docs.len() == 1 {
            self.mru.clear();
        }
        self.generation += 1;
        self.mru.push(id);
        Ok(id)
    }

    /// The active document (derived: the active view's document).
    #[inline]
    pub fn current(&self) -> strop_core::id::DocumentId {
        self.view().doc
    }

    /// Switch the active view to a document.
    pub fn switch_to(&mut self, id: strop_core::id::DocumentId) {
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

    /// Open a file into a new document and switch to it (`:e`).
    pub fn open_buffer(&mut self, path: &str) -> std::io::Result<()> {
        self.drop_stale_scratch();
        self.push_jump(); // leaving a buffer is a jumplist entry (vim)
                          // vim semantics: :e on an open file switches to its buffer
        let canon = std::path::Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| self.cwd.join(path));
        let existing = self
            .docs
            .iter()
            .find(|(_, d)| {
                d.buf
                    .path
                    .as_deref()
                    .and_then(|p| std::path::Path::new(p).canonicalize().ok())
                    == Some(canon.clone())
            })
            .map(|(id, _)| id);
        if let Some(id) = existing {
            self.switch_to(id);
            return Ok(());
        }
        let buf = Buffer::open(path)?;
        let id = self.docs.insert(Document::new(buf));
        if self.docs.len() == 1 {
            // the scratch was dropped under us
            self.mru.clear();
        }
        self.generation += 1; // document set changed: old jobs are stale (0011 §2)
        self.switch_to(id);
        self.set_head(0);
        self.view_mut().view_top = 0;
        self.discover_git();
        self.lsp_maybe_attach();
        Ok(())
    }

    /// Close the current document; quits when the last one closes.
    /// Returns false when unsaved changes block the close. Generational
    /// ids mean no reindexing anywhere (0014 wave 2).
    pub fn close_buffer(&mut self, force: bool) -> bool {
        if self.buf().dirty && !force {
            self.message = "unsaved changes — :q! to force".into();
            return false;
        }
        let closed = self.current();
        let closed_surface = self.docs.remove(closed).and_then(|d| d.surface);
        if self.docs.is_empty() {
            crate::session::save(self);
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
            self.switch_to(next);
            self.set_head(0);
            self.view_mut().view_top = 0;
            // a closing surface hands the cursor and view back to the
            // document it opened from — by id, no index math (0011 §1)
            if let Some(surface) = closed_surface {
                if let Some(ret) = surface.return_point() {
                    if self.docs.get(ret.buffer).is_some() {
                        if ret.buffer != self.current() {
                            self.view_mut().doc = ret.buffer;
                            self.touch_mru(ret.buffer);
                        }
                        self.set_head(ret.cursor.min(self.buf().len_bytes()));
                        self.view_mut().view_top = ret.view_top;
                    }
                }
            }
        }
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
}
