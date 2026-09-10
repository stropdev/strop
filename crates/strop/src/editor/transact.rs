//! A mutation lease is the publication boundary, independent of undo grouping.
mod capability;
pub(crate) use capability::{BufferEdit, DocumentEdit};
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::{Change, EditError, Replacement};

/// Every range refers to the same pre-edit document snapshot.
#[derive(Debug, Clone)]
pub struct ChangeSet {
    pub edits: Vec<Replacement>,
    pub undo_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplyError {
    #[error("no such document")]
    NoDocument,
    #[error(transparent)]
    Edit(#[from] EditError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Committed {
    pub revision: BufferRevision,
}

impl super::Editor {
    pub(crate) fn apply(
        &mut self,
        document: DocumentId,
        base: BufferRevision,
        changes: ChangeSet,
    ) -> Result<Committed, ApplyError> {
        let buffer = &self.docs.get(document).ok_or(ApplyError::NoDocument)?.buf;
        let prepared = buffer.prepare_replacements(base, changes.edits)?;
        if prepared.is_empty() {
            return Ok(Committed { revision: base });
        }
        if self
            .pending
            .prompt()
            .is_some_and(|prompt| prompt.origin().pane.doc == document)
        {
            self.cancel_pending();
        }
        let revision = self
            .doc_mut(document)
            .buf
            .apply_prepared(prepared, changes.undo_open)?;
        Ok(Committed { revision })
    }

    pub(crate) fn replace_system(
        &mut self,
        document: DocumentId,
        text: &str,
    ) -> Result<(), EditError> {
        self.doc_mut(document).buf.system_edit().replace_all(text)
    }

    pub(crate) fn tx_begin(&mut self) {
        self.buf_mut().begin_undo_group();
    }
    pub(crate) fn tx_commit(&mut self) {
        self.buf_mut().commit_undo_group();
        strop_trace::record_with(strop_trace::EventKind::History, || {
            serde_json::json!({
                "operation":"commit","buffer":self.buf().trace_id(),"document":self.current(),
                "revision":self.buf().revision(),"history_nodes":self.buf().history().depth(),
            })
        });
    }

    /// The lease cannot be released without consuming each change exactly once.
    /// Core records geometry BEFORE mutation; no post-edit reconstruction or
    /// cloning deleted text to reconstruct syntax/anchor effects.
    pub(super) fn sync_document(&mut self, id: DocumentId, map_active: bool) {
        self.sync_document_positions(id, map_active, map_position);
    }

    fn sync_document_positions(
        &mut self,
        id: DocumentId,
        map_active: bool,
        position: impl Fn(usize, &Change) -> usize,
    ) {
        let Some(document) = self.docs.get_mut(id) else {
            return;
        };
        if document.buf.changes().is_empty() {
            return;
        }
        let active = self.active_pane;
        for change in document.buf.changes() {
            let map = |offset| position(offset, change);
            for (owner, position) in self.marks.values_mut() {
                if *owner == id {
                    *position = map(*position);
                }
            }
            for (owner, position) in self
                .jumplist_past
                .iter_mut()
                .chain(self.jumplist_future.iter_mut())
            {
                if *owner == id {
                    *position = map(*position);
                }
            }
            for (index, pane) in self.panes.iter_mut().enumerate() {
                if pane.doc == id && (map_active || index != active) {
                    pane.sels.map_positions(map);
                }
            }
            // Collection excerpts anchor into sources like any other mark.
            for collection in self.collections.values_mut() {
                for excerpt in &mut collection.excerpts {
                    if excerpt.source == id {
                        excerpt.start = map(excerpt.start);
                        excerpt.end = map(excerpt.end);
                    }
                }
            }
        }
        self.analysis.edits(id, document.buf.changes());
        document.buf.clear_changes();
    }
}

fn map_position(position: usize, change: &Change) -> usize {
    let edit = change.edit;
    debug_assert!(edit.start_byte <= edit.old_end_byte);
    // The verified kernel (strop_core::editmap, 0045): positions are
    // byte offsets bounded by the buffer length, so its no-overflow
    // precondition is established by the rope, not by this caller.
    debug_assert!(edit.new_end_byte >= edit.start_byte);
    strop_core::editmap::map_position(
        position,
        edit.start_byte,
        edit.old_end_byte,
        edit.new_end_byte,
    )
}
