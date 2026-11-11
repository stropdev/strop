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
        position: impl Fn(usize, &[(usize, usize, usize)]) -> usize,
    ) {
        self.sync_filename_draft(id);
        self.collection_write_pending(id);
        let journal: Vec<(usize, usize, usize)> = match self.docs.get(id) {
            Some(doc) if !doc.buf.changes().is_empty() => {
                doc.buf.changes().iter().map(Change::extent).collect()
            }
            _ => return,
        };
        let map = |offset| position(offset, &journal);
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
        // column 1 and covers the mark took the mark's line with it.
        let line_starts: Vec<bool> = journal
            .iter()
            .map(|(start, _, _)| *start == document.buf.line_start(document.buf.line_of(*start)))
            .collect();
        self.marks.retain(|_, (owner, position)| {
            *owner != id
                || !journal.iter().zip(&line_starts).any(
                    |((start, old_end, new_end), line_start)| {
                        *line_start
                            && new_end == start
                            && *start <= *position
                            && *position < *old_end
                    },
                )
        });
        for (owner, position) in self.marks.values_mut() {
            if *owner == id {
                *position = map(*position);
            }
        }
        for (index, pane) in self.panes.iter_mut().enumerate() {
            if pane.doc == id && (map_active || index != active) {
                pane.sels.map_positions(map);
            }
        }
        for collection in self.collections.values_mut() {
            for excerpt in &mut collection.excerpts {
                if excerpt.source != id {
                    continue;
                }
                for anchor in &mut excerpt.hit_anchors {
                    *anchor = map(*anchor);
                }
                // Spans and matches are decided per edit, in the
                // coordinates that edit addressed.
                for step in 0..journal.len() {
                    let edit = &journal[step..=step];
                    map_excerpt_span(excerpt, edit[0]);
                    let map = |offset| position(offset, edit);
                    let (s, e, _) = edit[0];
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
        let ends_in_break = |index: usize| {
            document
                .buf
                .change_inserted(index)
                .is_some_and(|text| text.ends_with('\n'))
        };
        let marked: Vec<(usize, usize, usize, bool)> = journal
            .iter()
            .enumerate()
            .map(|(index, &(start, old_end, new_end))| {
                (start, old_end, new_end, ends_in_break(index))
            })
            .collect();
        self.pending.ex.marked.map_edits(id, &marked, &document.buf);
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

/// Collection excerpts are SPANS, not points (0049 §5): an edit replacing
/// the span's first byte keeps the start — the pointwise collapse rule would
/// slide the anchor past the new text and drop it from the view.
fn map_excerpt_span(
    excerpt: &mut super::collections::Excerpt,
    (s, e, new_end): (usize, usize, usize),
) {
    let new_len = new_end - s;
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
        // covered (or a boundary insertion — deleted-then-reinserted text
        // regrows the span)
        s + new_len
    };
}
