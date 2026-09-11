//! A mutation lease is the publication boundary, independent of undo grouping.
mod capability;
pub use capability::{BufferEdit, DocumentEdit};
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
        // Snapshot clean views before the mutable document borrow: a
        // view with an unsynced user edit is never regenerated (0049 §5).
        let clean_views: std::collections::HashSet<DocumentId> = self
            .collections
            .iter()
            .filter(|(cid, collection)| {
                self.docs
                    .get(**cid)
                    .is_some_and(|doc| doc.buf.revision() == collection.revision)
            })
            .map(|(cid, _)| *cid)
            .collect();
        let Some(document) = self.docs.get_mut(id) else {
            return;
        };
        if document.buf.changes().is_empty() {
            return;
        }
        let active = self.active_pane;
        // 0049 §5: classify dependent collection excerpts against each
        // change BEFORE the remap moves their source spans. A change
        // strictly inside one excerpt splices that excerpt's view rows;
        // anything touching an edge re-renders the whole view. Views
        // with an unsynced user edit are never regenerated underneath
        // the typist — the next write-back's render covers them.
        let mut splices: Vec<(DocumentId, usize)> = Vec::new();
        let mut renders: Vec<DocumentId> = Vec::new();
        for change in document.buf.changes() {
            let (start, end) = (change.edit.start_byte, change.edit.old_end_byte);
            for (collection_id, collection) in self.collections.iter() {
                if renders.contains(collection_id) {
                    continue;
                }
                if !clean_views.contains(collection_id) {
                    continue;
                }
                for (index, excerpt) in collection.excerpts.iter().enumerate() {
                    if excerpt.source != id {
                        continue;
                    }
                    // Insertions at a boundary belong to the span (the
                    // remap grows it onto the new bytes) — only a
                    // change strictly outside skips the refresh.
                    if end < excerpt.start || start > excerpt.end {
                        continue;
                    }
                    if start > excerpt.start && end < excerpt.end {
                        if !splices.contains(&(*collection_id, index)) {
                            splices.push((*collection_id, index));
                        }
                    } else {
                        renders.push(*collection_id);
                        break;
                    }
                }
            }
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
            // Collection excerpts are SPANS, not points (0049 §5): an
            // edit replacing the span's first byte keeps the start —
            // the pointwise collapse rule would slide the anchor past
            // the new text and drop it from the view.
            for collection in self.collections.values_mut() {
                for excerpt in &mut collection.excerpts {
                    if excerpt.source == id {
                        let (s, e) = (change.edit.start_byte, change.edit.old_end_byte);
                        let new_len = change.edit.new_end_byte - change.edit.start_byte;
                        let delta = new_len as isize - (e - s) as isize;
                        let shift = |p: usize| (p as isize + delta) as usize;
                        let insertion = s == e;
                        excerpt.start = if excerpt.start <= s {
                            excerpt.start
                        } else if excerpt.start >= e {
                            shift(excerpt.start)
                        } else {
                            s
                        };
                        excerpt.end = if excerpt.end < s {
                            excerpt.end
                        } else if excerpt.end > e || (excerpt.end == e && !insertion) {
                            shift(excerpt.end)
                        } else {
                            // covered (or a boundary insertion — deleted-
                            // then-reinserted text regrows the span)
                            s + new_len
                        };
                    }
                }
            }
        }
        self.analysis.edits(id, document.buf.changes());
        document.buf.clear_changes();
        // Refresh the dependent views after the journal is consumed.
        #[cfg(test)]
        if !renders.is_empty() || !splices.is_empty() {
            eprintln!("hook on doc {id:?}: renders={renders:?} splices={splices:?}");
        }
        for collection_id in renders {
            self.collection_render_view(collection_id);
        }
        for (collection_id, index) in splices {
            // A collection re-rendered above already shows the new text.
            if self
                .collections
                .get(&collection_id)
                .is_some_and(|collection| {
                    self.docs
                        .get(collection_id)
                        .is_some_and(|doc| doc.buf.revision() == collection.revision)
                })
            {
                self.collection_splice_excerpt(collection_id, index);
            }
        }
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
