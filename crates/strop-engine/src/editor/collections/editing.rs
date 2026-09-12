//! Live collection publication: preflight all sources, apply through their
//! mutation leases, keep one source undo group open for the Insert session.
use super::Editor;
use strop_core::id::DocumentId;
use strop_core::{Change, ChangeOrigin, Range, Replacement};

impl Editor {
    pub(crate) fn sync_collection_write_back(
        &mut self,
        id: DocumentId,
        map_active: bool,
        position: &impl Fn(usize, &Change) -> usize,
    ) {
        let Some(document) = self.docs.get_mut(id) else {
            return;
        };
        let changes = document.buf.changes().to_vec();
        let edits = super::journal::replacements(&document.buf, &changes);
        self.analysis.edits(id, &changes);
        document.buf.clear_changes();
        self.sync_collection_anchors(id, map_active, position, &changes);

        // Check every sequential operation, not just its net result: changing
        // chrome and changing it back is still not a source edit.
        let mut spans: Vec<_> = self.collections[&id]
            .excerpts
            .iter()
            .map(|e| (e.view_start, e.view_start + e.end - e.start, e.view_end))
            .collect();
        for change in &changes {
            let edit = change.edit;
            let owner = spans.iter().position(|&(start, end, rendered_end)| {
                edit.start_byte >= start
                    && edit.start_byte <= end
                    && edit.old_end_byte <= rendered_end
                    && (edit.start_byte < rendered_end || start == rendered_end)
            });
            if change.origin != ChangeOrigin::User || owner.is_none() {
                self.refuse_collection_edit(id, "edit touches generated chrome or spans excerpts");
                return;
            }
            let delta = edit.new_end_byte as isize - edit.old_end_byte as isize;
            for (index, (start, end, rendered_end)) in spans.iter_mut().enumerate() {
                if Some(index) == owner {
                    let source_delta = (edit.new_end_byte - edit.start_byte) as isize
                        - (edit.old_end_byte.min(*end) - edit.start_byte) as isize;
                    *end = end.saturating_add_signed(source_delta);
                    *rendered_end = rendered_end.saturating_add_signed(delta);
                } else if *start >= edit.old_end_byte {
                    *start = start.saturating_add_signed(delta);
                    *end = end.saturating_add_signed(delta);
                    *rendered_end = rendered_end.saturating_add_signed(delta);
                }
            }
        }

        let mut targets: Vec<(DocumentId, Vec<Replacement>)> = Vec::new();
        for edit in edits {
            let Some(excerpt) = self.collections[&id].excerpts.iter().find(|excerpt| {
                edit.range.start.get() >= excerpt.view_start
                    && edit.range.end.get() <= excerpt.view_end
                    && edit.range.start.get() <= excerpt.view_start + excerpt.end - excerpt.start
            }) else {
                self.refuse_collection_edit(id, "edit spans excerpt boundaries");
                return;
            };
            let source = excerpt.source;
            let replacement = Replacement::new(
                Range::charwise(
                    excerpt.start + edit.range.start.get() - excerpt.view_start,
                    (excerpt.start + edit.range.end.get() - excerpt.view_start).min(excerpt.end),
                ),
                edit.text,
            );
            if let Some((_, edits)) = targets.iter_mut().find(|(doc, _)| *doc == source) {
                edits.push(replacement);
            } else {
                targets.push((source, vec![replacement]));
            }
        }
        let mut prepared = Vec::with_capacity(targets.len());
        for (source, edits) in targets {
            let Some(document) = self.docs.get(source) else {
                self.refuse_collection_edit(id, "source was closed");
                return;
            };
            let base = document.buf.revision();
            if document.buf.readonly && document.remote_metadata().is_some() {
                self.refuse_collection_edit(id, "remote file is read-only; use :remote edit first");
                return;
            }
            match document.buf.prepare_replacements(base, edits) {
                Ok(edits) if edits.is_empty() => {}
                Ok(edits) => prepared.push((source, base, edits)),
                Err(error) => {
                    let name = document
                        .buf
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "[scratch]".into());
                    self.refuse_collection_edit(id, &format!("{name}: {error}"));
                    return;
                }
            }
        }

        // Source remapping will run once, from the source journal. Only the
        // view domain moves here. Keep its revision unsynchronized so nested
        // source publication cannot overwrite the in-flight user edit.
        if let Some(collection) = self.collections.get_mut(&id) {
            collection.map_view_changes(&changes);
        }
        let undo_open = true;
        for (source, before, edits) in prepared {
            let outcome = self.doc_mut(source).buf.apply_prepared(edits, undo_open);
            if let Err(error) = outcome {
                // All preparations are checked before any source mutation. No
                // await or external writer crosses this publication boundary.
                self.refuse_collection_edit(id, &format!("source apply failed: {error}"));
                return;
            }
            let collection = self.collections.get_mut(&id).unwrap();
            if !collection
                .pending_commit
                .iter()
                .any(|(document, _)| *document == source)
            {
                collection.pending_commit.push((source, before));
            }
        }
        let revision = self.docs.get(id).unwrap().buf.revision();
        self.collections.get_mut(&id).unwrap().revision = revision;
        self.docs.get_mut(id).unwrap().buf.dirty = false;
        // A removed final newline still needs a presentation-only separator.
        // Repair only such bodies; ordinary typing never rebuilds the view.
        let repairs: Vec<_> = self.collections[&id]
            .excerpts
            .iter()
            .enumerate()
            .filter_map(|(index, excerpt)| {
                let view = &self.docs.get(id)?.buf;
                let end = excerpt.view_end;
                (end == excerpt.view_start || view.text().byte(end - 1) != b'\n').then_some(index)
            })
            .collect();
        for index in repairs {
            self.collection_splice_excerpt(id, index);
        }
    }

    fn refuse_collection_edit(&mut self, id: DocumentId, reason: &str) {
        self.collection_render_view(id);
        self.message = format!("collection edit refused: {reason}; view refreshed");
    }

    fn sync_collection_anchors(
        &mut self,
        id: DocumentId,
        map_active: bool,
        position: &impl Fn(usize, &Change) -> usize,
        changes: &[Change],
    ) {
        let active = self.active_pane;
        for change in changes {
            let map = |offset| position(offset, change);
            for (owner, pos) in self.marks.values_mut() {
                if *owner == id {
                    *pos = map(*pos);
                }
            }
            self.map_navigation_records(id, map);
            for (index, pane) in self.panes.iter_mut().enumerate() {
                if pane.doc == id && (map_active || index != active) {
                    pane.sels.map_positions(map);
                }
            }
        }
    }

    pub(crate) fn commit_collection_sources(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        let Some(pending) = self
            .collections
            .get_mut(&self.current())
            .map(|collection| std::mem::take(&mut collection.pending_commit))
        else {
            return;
        };
        let mut applied_positions = Vec::with_capacity(pending.len());
        let mut applied = Vec::with_capacity(pending.len());
        for (source, before) in pending {
            self.doc_mut(source).buf.commit_undo_group();
            if let Some(document) = self.docs.get(source) {
                if let Some(position) = document.buf.history().committed_position() {
                    applied_positions.push(position);
                }
                applied.push((source, before, document.buf.revision()));
            }
        }
        if !applied.is_empty() {
            self.message = format!("collection edit: applied to {} buffer(s)", applied.len());
            self.changes.record(crate::editor::changes::ChangeReceipt {
                producer: "collection edit".into(),
                applied,
                applied_positions,
                refused: Vec::new(),
                redo_positions: None,
            });
        }
    }
}
