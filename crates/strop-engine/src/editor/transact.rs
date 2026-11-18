//! A mutation lease is the publication boundary, independent of undo grouping.
mod capability;
pub use capability::{BufferEdit, DocumentEdit};
#[cfg(test)]
mod tests;
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
    #[error(transparent)]
    Excerpt(#[from] super::collections::ExcerptRefusal),
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
        // A collection admits the whole batch or none of it; the batch is
        // placed in the sources when this document's lease settles.
        for edit in &changes.edits {
            self.collection_admit(
                document,
                edit.range.start.get(),
                edit.range.end.get(),
                &edit.text,
            )?;
        }
        let prepared = buffer.prepare_replacements(base, changes.edits)?;
        if prepared.is_empty() {
            return Ok(Committed { revision: base });
        }
        if self
            .cmdline
            .line()
            .is_some_and(|prompt| prompt.origin().pane.doc == document)
        {
            self.cancel_cmdline();
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
        self.collection_commit_sources(self.current());
        strop_trace::record_with(strop_trace::EventKind::History, || {
            serde_json::json!({
                "operation":"commit","buffer":self.buf().trace_id(),"document":self.current(),
                "revision":self.buf().revision(),"history_nodes":self.buf().history().depth(),
            })
        });
    }

    /// The lease cannot be released without consuming each change exactly once.
    /// Core records geometry BEFORE mutation; no post-edit reconstruction or
    /// cloning deleted text to reconstruct syntax/anchor effects. Points map
    /// through the whole journal with the proved kernel fold.
    pub(super) fn sync_document(&mut self, id: DocumentId, map_active: bool) {
        self.sync_document_positions(id, map_active, strop_kernel::editmap::map_through_edits);
    }

    /// `position` maps one offset through a journal slice of
    /// `(start, old_end, new_end)` entries in application order.
    fn sync_document_positions(
        &mut self,
        id: DocumentId,
        map_active: bool,
        position_through: impl Fn(usize, &[(usize, usize, usize)]) -> usize,
    ) {
        self.sync_filename_draft(id);
        self.collection_write_pending(id);
        let journal: Vec<(usize, usize, usize)> = match self.docs.get(id) {
            Some(doc) if !doc.buf.changes().is_empty() => {
                doc.buf.changes().iter().map(Change::extent).collect()
            }
            _ => return,
        };
        let map = |offset| position_through(offset, &journal);
        // Return views live in other documents: remap them before
        // borrowing the source mutably.
        self.map_navigation_records(id, map);
        // Snapshot clean views before the mutable document borrow: source
        // refreshes never overwrite an in-flight system projection update.
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
        // A view that did not make this edit can no longer undo into this
        // source: its recorded steps are not the source's newest ones.
        for view in &clean_views {
            if self.collections[view]
                .excerpts
                .iter()
                .any(|excerpt| excerpt.source == id)
            {
                if let Some(document) = self.docs.get_mut(*view) {
                    document.buf.system_edit().forget_history();
                }
            }
        }
        let collection_updates = self.prepare_collection_updates(id, &clean_views);
        let Some(document) = self.docs.get_mut(id) else {
            return;
        };
        if document.buf.changes().is_empty() {
            return;
        }
        let active = self.active_pane;
        // Vim: a mark goes with its line. A deletion that starts in
        // column 1 and covers the mark took the mark's line with it. Each
        // edit is judged in the coordinates it addressed (its own start
        // point, the mark followed through the edits before it), so a batch
        // of several edits decides like the same edits made one by one.
        let line_starts: Vec<bool> = document
            .buf
            .changes()
            .iter()
            .map(|change| change.edit.start_point.1 == 0)
            .collect();
        let ends_in_break: Vec<bool> = (0..journal.len())
            .map(|index| {
                document
                    .buf
                    .change_inserted(index)
                    .is_some_and(|text| text.ends_with('\n'))
            })
            .collect();
        self.marks.retain(|_, (owner, position)| {
            if *owner != id {
                return true;
            }
            let mut at = *position;
            for (step, &(start, old_end, new_end)) in journal.iter().enumerate() {
                if line_starts[step] && new_end == start && start <= at && at < old_end {
                    return false;
                }
                at = position_through(at, &journal[step..=step]);
            }
            *position = at;
            true
        });
        for (index, pane) in self.panes.iter_mut().enumerate() {
            if pane.doc == id && (map_active || index != active) {
                pane.sels.map_positions(map);
            }
        }
        // a parked pane is an inactive one
        for pane in self.tabs.panes_mut() {
            if pane.doc == id {
                pane.sels.map_positions(map);
            }
        }
        for collection in self.collections.values_mut() {
            let writing = collection.writing;
            for excerpt in &mut collection.excerpts {
                if excerpt.source != id {
                    continue;
                }
                for anchor in &mut excerpt.hit_anchors {
                    *anchor = map(*anchor);
                }
                // Spans and matches are decided per edit, in the
                // coordinates that edit addressed. An excerpt is whole
                // source lines, so its end is column 1 of the line below:
                // an edit there belongs to that line (0108 B4) — unless the
                // view wrote it through this excerpt (a line opened, typed
                // or restored by undo at the excerpt's end), or the end is
                // in mid-line (a last line without a newline goes on).
                for step in 0..journal.len() {
                    let edit = &journal[step..=step];
                    let (s, e, new_end) = edit[0];
                    (excerpt.start, excerpt.end) = strop_kernel::editmap::map_span(
                        excerpt.start,
                        excerpt.end,
                        s,
                        e,
                        new_end,
                        writing || !line_starts[step],
                    );
                    let map = |offset| position_through(offset, edit);
                    excerpt.matches.retain_mut(|(start, length)| {
                        let finish = start.saturating_add(*length);
                        if s < finish && e > *start {
                            return false;
                        }
                        let mapped = map(*start);
                        *length = map(finish).saturating_sub(mapped);
                        *start = mapped;
                        true
                    });
                }
            }
        }
        let marked: Vec<(usize, usize, usize, bool)> = journal
            .iter()
            .enumerate()
            .map(|(index, &(start, old_end, new_end))| {
                (start, old_end, new_end, ends_in_break[index])
            })
            .collect();
        self.cmdline.ex.marked.map_edits(id, &marked, &document.buf);
        self.analysis.edits(id, document.buf.changes());
        self.resolution.edits(id, document.buf.changes());
        if let Err(error) = self.completion.edits(
            &self.tape,
            id,
            document.buf.revision(),
            document.buf.text(),
            document.buf.changes(),
        ) {
            self.message = error;
        }
        document.buf.clear_changes();
        self.publish_collection_updates(collection_updates);
        self.collection_settle(id);
    }
}
