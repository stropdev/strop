//! Structural source refreshes preserve source positions, not offsets into a
//! wholesale replacement of the presentation buffer. Only cold rebuilds snapshot
//! excerpt geometry; ordinary edits use the exact projected journal instead.
use super::Editor;
use strop_core::id::DocumentId;
use strop_core::Change;

struct ExcerptPosition {
    source: DocumentId,
    start: usize,
    end: usize,
    view_start: usize,
    view_end: usize,
}

pub(super) struct RebuildPositions {
    source: DocumentId,
    changes: Vec<Change>,
    excerpts: Vec<ExcerptPosition>,
    title_end: usize,
    viewports: Vec<(usize, usize)>,
}

impl Editor {
    pub(super) fn capture_rebuild_positions(
        &self,
        document: DocumentId,
        source: DocumentId,
        changes: &[Change],
    ) -> RebuildPositions {
        let buffer = &self.doc(document).buf;
        RebuildPositions {
            source,
            changes: changes.to_vec(),
            excerpts: positions(&self.collections[&document]),
            title_end: buffer.line_start(1),
            viewports: self
                .panes
                .iter()
                .enumerate()
                .filter(|(_, pane)| pane.doc == document)
                .map(|(index, pane)| {
                    (
                        index,
                        buffer.line_start(pane.view_top.min(buffer.last_content_line())),
                    )
                })
                .collect(),
        }
    }

    pub(super) fn rebuild_collection_sources(
        &mut self,
        document: DocumentId,
        previous: RebuildPositions,
    ) {
        let Some(collection) = self.collections.get_mut(&document) else {
            return;
        };
        let text = super::projection::render(&self.docs, &self.cwd, collection);
        let next = positions(collection);
        let Some(doc) = self.docs.get_mut(document) else {
            return;
        };
        if let Err(error) = doc.buf.system_edit().replace_all(&text) {
            self.invalidate_collection_projection(document, error);
            return;
        }
        self.analysis.edits(document, doc.buf.changes());
        doc.buf.clear_changes();
        doc.buf.dirty = false;
        collection.revision = doc.buf.revision();
        let title_end = doc.buf.line_start(1);
        let length = doc.buf.len_bytes();
        let map = |position| previous.map(position, &next, title_end).min(length);
        self.map_navigation_records(document, map);
        for (owner, position) in self.marks.values_mut() {
            if *owner == document {
                *position = map(*position);
            }
        }
        for pane in &mut self.panes {
            if pane.doc == document {
                pane.sels.map_positions(map);
            }
        }
        if let Some(doc) = self.docs.get(document) {
            for &(index, top) in &previous.viewports {
                self.panes[index].view_top = doc.buf.line_of(map(top));
            }
        }
        if self.current() == document {
            self.clamp_cursor();
        }
    }
}

fn positions(collection: &super::Collection) -> Vec<ExcerptPosition> {
    collection
        .excerpts
        .iter()
        .map(|excerpt| ExcerptPosition {
            source: excerpt.source,
            start: excerpt.start,
            end: excerpt.end,
            view_start: excerpt.view_start,
            view_end: excerpt.view_end,
        })
        .collect()
}

impl RebuildPositions {
    fn map(&self, position: usize, next: &[ExcerptPosition], title_end: usize) -> usize {
        if position < self.title_end {
            return position.min(title_end.saturating_sub(1));
        }
        let index = self
            .excerpts
            .partition_point(|excerpt| excerpt.view_start <= position);
        if let Some(old) = index
            .checked_sub(1)
            .and_then(|index| self.excerpts.get(index))
        {
            if position < old.view_end {
                let mut source_position =
                    old.start + (position - old.view_start).min(old.end - old.start);
                if old.source == self.source {
                    for change in &self.changes {
                        let edit = change.edit;
                        source_position = strop_core::editmap::map_position(
                            source_position,
                            edit.start_byte,
                            edit.old_end_byte,
                            edit.new_end_byte,
                        );
                    }
                }
                if let Some(new) = next.get(index - 1).filter(|new| new.source == old.source) {
                    return new.view_start
                        + source_position
                            .saturating_sub(new.start)
                            .min(new.end - new.start);
                }
                return 0;
            }
        }
        // Chrome belongs to the following body (or the final card footer).
        if let Some((old, new)) = self.excerpts.get(index).zip(next.get(index)) {
            let lower = index
                .checked_sub(1)
                .and_then(|index| next.get(index))
                .map_or(title_end, |old| old.view_end);
            return new
                .view_start
                .saturating_sub(old.view_start.saturating_sub(position))
                .max(lower);
        }
        self.excerpts
            .last()
            .zip(next.last())
            .map_or(0, |(old, new)| {
                new.view_end
                    .saturating_add(position.saturating_sub(old.view_end))
            })
    }
}
