//! Structural refreshes preserve source positions, not offsets into a wholesale
//! replacement of the presentation buffer. Only cold rebuilds snapshot
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
    edit: Option<SourceEdit>,
    excerpts: Vec<ExcerptPosition>,
    title_end: usize,
}

struct SourceEdit {
    source: DocumentId,
    /// `(start, old_end, new_end)` in application order.
    journal: Vec<(usize, usize, usize)>,
}

impl Editor {
    pub(super) fn capture_rebuild_positions(
        &self,
        document: DocumentId,
        edit: Option<(DocumentId, &[Change])>,
    ) -> RebuildPositions {
        let buffer = &self.doc(document).buf;
        RebuildPositions {
            edit: edit.map(|(source, changes)| SourceEdit {
                source,
                journal: changes.iter().map(Change::extent).collect(),
            }),
            excerpts: positions(&self.collections[&document]),
            title_end: buffer.line_start(1),
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
        // Pane viewports are line numbers. Read each pane's own top against
        // the old rope, including panes whose tab is currently parked.
        let old_text = doc.buf.snapshot();
        let old_last = doc.buf.last_content_line();
        if let Err(error) = doc.buf.system_edit().replace_all(&text) {
            self.invalidate_collection_projection(document, error);
            return;
        }
        self.analysis.edits(document, doc.buf.changes());
        if let Err(error) = self.completion.edits(
            &self.tape,
            document,
            doc.buf.revision(),
            doc.buf.text(),
            doc.buf.changes(),
        ) {
            self.message = error;
        }
        doc.buf.clear_changes();
        doc.buf.mark_clean();
        collection.revision = doc.buf.revision();
        collection.mapped = collection.revision;
        let title_end = doc.buf.line_start(1);
        let text = doc.buf.snapshot();
        let map = |position| {
            let byte = previous
                .map(position, &next, title_end)
                .min(text.len_bytes());
            // A changed file heading can put a chrome offset inside a glyph.
            text.char_to_byte(text.byte_to_char(byte))
        };
        self.map_navigation_records(document, map);
        for (owner, position) in self.marks.values_mut() {
            if *owner == document {
                *position = map(*position);
            }
        }
        for pane in self.panes.iter_mut().chain(self.tabs.panes_mut()) {
            if pane.doc == document {
                pane.sels.map_positions(map);
                let top = old_text.line_to_byte(pane.view_top.min(old_last));
                pane.view_top = text.byte_to_line(map(top));
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
    fn source_position(&self, source: DocumentId, byte: usize) -> usize {
        match &self.edit {
            Some(edit) if edit.source == source => {
                strop_kernel::editmap::map_through_edits(byte, &edit.journal)
            }
            _ => byte,
        }
    }

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
                let source_position = self.source_position(
                    old.source,
                    old.start + (position - old.view_start).min(old.end - old.start),
                );
                return nearest(next, index - 1, old.source, source_position).map_or(0, |i| {
                    let new = &next[i];
                    // Keep omitted context on the nearest surviving body byte,
                    // not on the following block's heading.
                    new.view_start
                        + source_position
                            .saturating_sub(new.start)
                            .min((new.view_end - new.view_start).saturating_sub(1))
                });
            }
        }
        // Chrome belongs to the following body (or the end of the last block).
        if let Some(old) = self.excerpts.get(index) {
            let start = self.source_position(old.source, old.start);
            let Some(i) = nearest(next, index, old.source, start) else {
                return 0; // the file was removed
            };
            let new = &next[i];
            let lower = i
                .checked_sub(1)
                .and_then(|i| next.get(i))
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

/// Excerpts may split, merge, or disappear. The old slot is merely a fast
/// path; the file and byte are the identity. A removed file lands on the title.
fn nearest(
    next: &[ExcerptPosition],
    slot: usize,
    source: DocumentId,
    byte: usize,
) -> Option<usize> {
    if next
        .get(slot)
        .is_some_and(|e| e.source == source && (e.start..e.end).contains(&byte))
    {
        return Some(slot);
    }
    next.iter()
        .enumerate()
        .filter(|(_, e)| e.source == source && e.start < e.end)
        .min_by_key(|(_, e)| byte.abs_diff(byte.clamp(e.start, e.end - 1)))
        .map(|(i, _)| i)
}
