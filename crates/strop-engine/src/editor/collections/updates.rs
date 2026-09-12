//! Project source journals into clean views. Ordinary source edits never replace
//! an entire excerpt: their exact view edits also remap carets, marks and history.
use super::{Collection, CollectionRow, Editor};
use std::collections::HashSet;
use strop_core::id::DocumentId;
use strop_core::{Change, Range, Replacement};

pub(crate) struct ProjectionUpdate {
    document: DocumentId,
    edits: Vec<ProjectedEdit>,
    rebuild: Option<super::view_positions::RebuildPositions>,
}
struct ProjectedEdit {
    replacement: Replacement,
    terminal_excerpt: Option<usize>,
}

impl Collection {
    pub(super) fn map_view_changes(&mut self, changes: &[Change]) {
        for change in changes {
            let edit = change.edit;
            let delta = edit.new_end_byte as isize - edit.old_end_byte as isize;
            let lines = edit.new_end_point.0 as isize - edit.old_end_point.0 as isize;
            for excerpt in &mut self.excerpts {
                if edit.start_byte >= excerpt.view_start && edit.old_end_byte <= excerpt.view_end {
                    excerpt.view_end = excerpt.view_end.saturating_add_signed(delta);
                    excerpt.view_lines = excerpt.view_lines.saturating_add_signed(lines);
                } else if excerpt.view_start >= edit.old_end_byte {
                    excerpt.view_start = excerpt.view_start.saturating_add_signed(delta);
                    excerpt.view_end = excerpt.view_end.saturating_add_signed(delta);
                    excerpt.view_line = excerpt.view_line.saturating_add_signed(lines);
                }
            }
            let row = edit.start_point.0;
            self.rows.splice(
                row..edit.old_end_point.0,
                std::iter::repeat_n(CollectionRow::Body, edit.new_end_point.0 - row),
            );
        }
    }
}

impl Editor {
    pub(crate) fn prepare_collection_updates(
        &self,
        source: DocumentId,
        clean: &HashSet<DocumentId>,
    ) -> Vec<ProjectionUpdate> {
        if !self.collections.iter().any(|(id, collection)| {
            clean.contains(id)
                && collection
                    .excerpts
                    .iter()
                    .any(|excerpt| excerpt.source == source)
        }) {
            return Vec::new();
        }
        let buffer = &self.doc(source).buf;
        let edits = super::journal::replacements(buffer, buffer.changes());
        let line_change = buffer
            .changes()
            .iter()
            .any(|change| change.edit.old_end_point.0 != change.edit.new_end_point.0);
        let mut updates = Vec::new();
        for (id, collection) in &self.collections {
            if !clean.contains(id) {
                continue;
            }
            let mut update = ProjectionUpdate {
                document: *id,
                edits: Vec::new(),
                rebuild: None,
            };
            let mut rebuild = false;
            let mut first = usize::MAX;
            let mut last = 0;
            for (index, excerpt) in collection.excerpts.iter().enumerate() {
                if excerpt.source != source {
                    continue;
                }
                first = first.min(excerpt.start);
                last = last.max(excerpt.end);
                for edit in &edits {
                    let (start, end) = (edit.range.start.get(), edit.range.end.get());
                    if end < excerpt.start
                        || start > excerpt.end
                        || (end == excerpt.start && start < end)
                    {
                        continue;
                    }
                    if start < excerpt.start || end > excerpt.end {
                        rebuild = true;
                        continue;
                    }
                    let terminal = end == excerpt.end;
                    update.edits.push(ProjectedEdit {
                        replacement: Replacement::new(
                            Range::charwise(
                                excerpt.view_start + start - excerpt.start,
                                if terminal {
                                    excerpt.view_end
                                } else {
                                    excerpt.view_start + end - excerpt.start
                                },
                            ),
                            edit.text.clone(),
                        ),
                        terminal_excerpt: terminal.then_some(index),
                    });
                }
            }
            if line_change && first != usize::MAX {
                // Edits in an omitted gap may change its displayed line count.
                rebuild |= edits.iter().any(|edit| {
                    let (start, end) = (edit.range.start.get(), edit.range.end.get());
                    start < last
                        && end > first
                        && !collection.excerpts.iter().any(|excerpt| {
                            excerpt.source == source && start >= excerpt.start && end <= excerpt.end
                        })
                });
            }
            if rebuild {
                update.edits.clear();
                update.rebuild =
                    Some(self.capture_rebuild_positions(*id, source, buffer.changes()));
            }
            if update.rebuild.is_some() || !update.edits.is_empty() {
                updates.push(update);
            }
        }
        updates
    }

    pub(crate) fn publish_collection_updates(&mut self, updates: Vec<ProjectionUpdate>) {
        for mut update in updates {
            let id = update.document;
            if let Some(previous) = update.rebuild.take() {
                self.rebuild_collection_sources(id, previous);
                continue;
            }
            for edit in &mut update.edits {
                if let Some(index) = edit.terminal_excerpt {
                    let excerpt = &self.collections[&id].excerpts[index];
                    let source = &self.doc(excerpt.source).buf;
                    if excerpt.start == excerpt.end || source.text().byte(excerpt.end - 1) != b'\n'
                    {
                        edit.replacement.text.push('\n');
                    }
                }
            }
            let Some(document) = self.docs.get_mut(id) else {
                continue;
            };
            let mut failure = None;
            for edit in update.edits.into_iter().rev() {
                if let Err(error) = document
                    .buf
                    .system_edit()
                    .replace(edit.replacement.range, &edit.replacement.text)
                {
                    failure = Some(error);
                    break;
                }
            }
            if let Some(error) = failure {
                self.invalidate_collection_projection(id, error);
                continue;
            }
            let changes = document.buf.changes().to_vec();
            if let Some(collection) = self.collections.get_mut(&id) {
                collection.map_view_changes(&changes);
                collection.revision = document.buf.revision();
            }
            self.sync_document(id, true);
        }
    }

    pub(super) fn invalidate_collection_projection(
        &mut self,
        id: DocumentId,
        error: strop_core::EditError,
    ) {
        // A stale representation must never regain source write authority.
        if let Some(document) = self.docs.get_mut(id) {
            document.buf.readonly = true;
            document.buf.name = Some("stale collection — projection failed".into());
        }
        self.collections.remove(&id);
        self.message = format!("collection projection failed: {error}; source edits are retained");
    }
}
