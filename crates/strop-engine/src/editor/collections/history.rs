//! Scoped source undo/redo and source save completion.
use super::Editor;
use std::path::PathBuf;
use strop_core::id::DocumentId;

impl Editor {
    /// `u` in a collection (0049 §5): undo the newest edit group that
    /// came FROM this collection, across its actual sources. Preflight
    /// every member — a source edited since refuses the whole group by
    /// name, and the receipt is never consumed on refusal.
    pub(crate) fn collection_undo(&mut self) {
        let id = self.current();
        let Some(sources) = self
            .collections
            .get(&id)
            .map(|c| c.excerpts.iter().map(|e| e.source).collect::<Vec<_>>())
        else {
            return;
        };
        let Some((index, mut receipt)) = self.changes.take_newest_matching(|receipt| {
            receipt.producer == "collection edit"
                && receipt
                    .applied
                    .iter()
                    .any(|(document, ..)| sources.contains(document))
        }) else {
            self.message = "already at oldest change".into();
            return;
        };
        let mut moved = 0;
        for (document, ..) in &receipt.applied {
            let refusal = match self.docs.get(*document) {
                Some(doc) => match doc.buf.check_undo() {
                    Ok(true) => None,
                    Ok(false) => Some(format!("{}: no undo remains", doc.label(&self.cwd))),
                    Err(error) => Some(format!("{}: {error}", doc.label(&self.cwd))),
                },
                None => Some("source buffer was closed".into()),
            };
            if let Some(reason) = refusal {
                self.changes.restore(index, receipt);
                self.message = format!("collection undo refused: {reason}; receipt retained");
                return;
            }
        }
        for (member, (document, ..)) in receipt.applied.iter().enumerate() {
            match self.docs.get(*document) {
                Some(doc)
                    if doc.buf.history().committed_position()
                        == receipt.applied_positions.get(member).copied() => {}
                Some(_) => {
                    let name = self
                        .docs
                        .get(*document)
                        .and_then(|d| d.buf.path.clone())
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "a source".into());
                    self.changes.restore(index, receipt);
                    self.message =
                        format!("collection undo refused: {name} changed since — resolve it first");
                    return;
                }
                None => {
                    self.changes.restore(index, receipt);
                    self.message = "collection undo refused: a source buffer was closed".into();
                    return;
                }
            }
        }
        let mut positions = Vec::with_capacity(receipt.applied.len());
        for (document, ..) in &receipt.applied {
            let undone_ok = matches!(self.doc_mut(*document).buf.undo(), Ok(Some(_)));
            if undone_ok {
                moved += 1;
            }
            if let Some(position) = self
                .docs
                .get(*document)
                .and_then(|doc| doc.buf.history().committed_position())
            {
                positions.push(position);
            }
        }
        receipt.redo_positions = Some(positions);
        // The undo's own journal refreshes the dependent views (0049 §5
        // invalidation) — no explicit render here.
        self.changes.push_undone(receipt);
        self.message = format!("undid collection edit across {moved} buffer(s)");
    }

    /// `ctrl-r` in a collection: redo the newest undone group of this
    /// collection, same preflight rules as undo.
    pub(crate) fn collection_redo(&mut self) {
        let id = self.current();
        let Some(sources) = self
            .collections
            .get(&id)
            .map(|c| c.excerpts.iter().map(|e| e.source).collect::<Vec<_>>())
        else {
            return;
        };
        let Some(receipt) = self.changes.take_undone_matching(|receipt| {
            receipt.producer == "collection edit"
                && receipt
                    .applied
                    .iter()
                    .any(|(document, ..)| sources.contains(document))
        }) else {
            self.message = "nothing to redo".into();
            return;
        };
        let positions = receipt.redo_positions.as_ref();
        for (member, (document, ..)) in receipt.applied.iter().enumerate() {
            let refusal = match self.docs.get(*document) {
                Some(doc) => match doc
                    .buf
                    .check_restore_revision(receipt.applied_positions[member])
                {
                    Ok(true) => None,
                    Ok(false) => Some(format!(
                        "{}: redo target is unavailable",
                        doc.label(&self.cwd)
                    )),
                    Err(error) => Some(format!("{}: {error}", doc.label(&self.cwd))),
                },
                None => Some("source buffer was closed".into()),
            };
            if let Some(reason) = refusal {
                self.changes.push_undone(receipt);
                self.message = format!("collection redo refused: {reason}; receipt retained");
                return;
            }
        }
        for (member, (document, ..)) in receipt.applied.iter().enumerate() {
            let at = self
                .docs
                .get(*document)
                .and_then(|doc| doc.buf.history().committed_position());
            if at.is_none() || at != positions.and_then(|p| p.get(member)).copied() {
                self.changes.push_undone(receipt);
                self.message = "collection redo refused: a source changed since the undo".into();
                return;
            }
        }
        let mut moved = 0;
        for (member, (document, ..)) in receipt.applied.iter().enumerate() {
            if matches!(
                self.doc_mut(*document)
                    .buf
                    .restore_revision(receipt.applied_positions[member]),
                Ok(Some(_))
            ) {
                moved += 1;
            }
        }
        self.changes.push_receipt_back(receipt);
        self.message = format!("redid collection edit across {moved} buffer(s)");
    }
}

impl Editor {
    /// `:w` in a collection (0049 §5): save the dirty SOURCES through
    /// their own save paths — never the presentation. `:w PATH` refuses:
    /// the view is not a file and exporting it is not this operation.
    pub(crate) fn collection_save(&mut self, target: Option<PathBuf>, force: bool, close: bool) {
        let id = self.current();
        if target.is_some() {
            self.message = "a collection has no file of its own — :w saves its sources;                             exporting the view is unsupported"
                .into();
            return;
        }
        let Some(sources) = self.collections.get(&id).map(|c| {
            let mut seen: Vec<DocumentId> = c.excerpts.iter().map(|e| e.source).collect();
            seen.dedup();
            seen
        }) else {
            return;
        };
        let mut pending = std::collections::HashSet::new();
        let mut refused: Vec<String> = Vec::new();
        for source in sources {
            let Some(doc) = self.docs.get(source) else {
                continue;
            };
            if !doc.buf.dirty {
                continue;
            }
            let name = doc.label(&self.cwd);
            // Admission owns readonly/remote-permit checks; :w! grants
            // no new capability. Count only writes actually admitted.
            if self.request_save_document(source, None, force, false) {
                pending.insert(source);
            } else {
                refused.push(format!("{name}: {}", self.message));
            }
        }
        let queued = pending.len();
        if let Some(collection) = self.collections.get_mut(&id) {
            collection.pending_saves = pending;
            collection.close_when_saved = close && refused.is_empty() && queued > 0;
        }
        if queued == 0 && refused.is_empty() {
            if close {
                self.close_pane_or_buffer(false);
            } else {
                self.message = "collection: all sources are saved".into();
            }
            return;
        }
        if !refused.is_empty() {
            self.message = format!(
                "collection: saving {queued} source(s); refused: {}",
                refused.join(", ")
            );
        } else {
            self.message = format!("collection: saving {queued} source(s)");
        }
    }

    /// A source save completed (0049 §5): count down; the `:wq` view
    /// closes only when every save confirmed. A failure cancels the
    /// close and stays visible.
    pub(crate) fn collection_save_progress(&mut self, document: DocumentId, saved: bool) {
        let mut close: Option<DocumentId> = None;
        for (id, collection) in self.collections.iter_mut() {
            if !collection.pending_saves.remove(&document) {
                continue;
            }
            if !saved {
                collection.pending_saves.clear();
                collection.close_when_saved = false;
                continue;
            }
            if collection.pending_saves.is_empty() && collection.close_when_saved {
                close = Some(*id);
            }
        }
        if let Some(id) = close {
            if self.current() == id {
                self.close_pane_or_buffer(false);
            } else if let Some(collection) = self.collections.get_mut(&id) {
                collection.close_when_saved = false;
                self.message = "collection: sources saved".into();
            }
        }
    }
}
