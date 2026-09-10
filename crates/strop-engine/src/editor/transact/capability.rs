//! Editor-owned mutation leases publish the core journal even on early return.
use super::super::{Document, Editor};
use std::ops::{Deref, DerefMut};
use strop_core::id::{ByteOffset, DocumentId};
use strop_core::{Buffer, Range};

pub struct DocumentEdit<'a> {
    editor: &'a mut Editor,
    document: DocumentId,
    map_active: bool,
}
impl<'a> DocumentEdit<'a> {
    pub(crate) fn new(editor: &'a mut Editor, document: DocumentId) -> Self {
        assert!(
            editor.docs.get(document).is_some(),
            "mutation lease requires a live document"
        );
        Self {
            editor,
            document,
            map_active: true,
        }
    }

    /// A replacement snapshot has its own position correspondence, not the
    /// ordinary "deleted bytes collapse to the edit start" rule. Publish the
    /// same journal once, with that mapping for every view and saved anchor.
    pub(crate) fn replace_snapshot(
        mut self,
        rope: ropey::Rope,
        position: impl Fn(usize) -> usize,
    ) -> Result<(), strop_core::EditError> {
        debug_assert!(self.buf.changes().is_empty());
        self.buf.system_edit().replace_rope(rope)?;
        self.editor
            .sync_document_positions(self.document, true, |offset, _| position(offset));
        Ok(())
    }
}
impl Deref for DocumentEdit<'_> {
    type Target = Document;
    fn deref(&self) -> &Document {
        match self.editor.docs.get(self.document) {
            Some(document) => document,
            None => unreachable!("exclusive mutation lease preserves its document"),
        }
    }
}
impl DerefMut for DocumentEdit<'_> {
    fn deref_mut(&mut self) -> &mut Document {
        match self.editor.docs.get_mut(self.document) {
            Some(document) => document,
            None => unreachable!("exclusive mutation lease preserves its document"),
        }
    }
}
impl Drop for DocumentEdit<'_> {
    fn drop(&mut self) {
        self.editor.sync_document(self.document, self.map_active);
    }
}

pub struct BufferEdit<'a>(DocumentEdit<'a>);
impl<'a> BufferEdit<'a> {
    pub(crate) fn new(mut document: DocumentEdit<'a>) -> Self {
        // Input commands place their own active cursors; document transactions
        // and worker publications instead remap every view automatically.
        document.map_active = false;
        Self(document)
    }
    pub fn insert(&mut self, at: impl Into<ByteOffset>, text: &str) -> bool {
        match self.0.buf.edit().insert(at, text) {
            Ok(()) => true,
            Err(error) => {
                self.0.editor.message = format!("edit failed: {error}");
                false
            }
        }
    }
    pub fn delete(&mut self, range: Range) -> String {
        match self.0.buf.edit().delete(range) {
            Ok(text) => text,
            Err(error) => {
                self.0.editor.message = format!("edit failed: {error}");
                String::new()
            }
        }
    }
}
impl Deref for BufferEdit<'_> {
    type Target = Buffer;
    fn deref(&self) -> &Buffer {
        &self.0.buf
    }
}
impl DerefMut for BufferEdit<'_> {
    fn deref_mut(&mut self) -> &mut Buffer {
        &mut self.0.buf
    }
}
