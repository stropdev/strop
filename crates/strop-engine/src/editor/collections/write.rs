//! Write-through (0094 §3.1): an edit inside an excerpt body is an edit of
//! its source. The kernel decides admission and placement
//! (`strop_kernel::excerpt::source_edit`); this module finds the excerpt,
//! reads the replacement text and applies the placed edit to the source.
//!
//! Two shapes reach here. A leased edit (`BufferEdit::insert`/`delete`) is
//! admitted before the view changes and written to the source immediately
//! after, so the next edit of the same lease sees current geometry on both
//! sides. A batch (`Editor::apply`, admitted as a whole first) and a history
//! move (`u`, `Ctrl-R`) reach `sync_document` with a journal the view's
//! geometry has not accounted for. A batch is composed into disjoint
//! replacements and written per source; a history move is repeated in the
//! history of each source it touched, which holds the exact inverse.
use super::{Collection, Editor};
use strop_core::id::DocumentId;
use strop_core::{ChangeOrigin, Range, Replacement};

/// Why a view edit was not made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExcerptRefusal {
    #[error("not text: a collection edits inside its excerpts; borders, gaps and the title stay")]
    Chrome,
    #[error("{0} is read-only; its excerpts cannot change")]
    ReadonlySource(String),
}

/// One view edit placed in its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceWrite {
    source: DocumentId,
    start: usize,
    end: usize,
    /// Whole lines were added below a last line that has no newline: the
    /// source takes the newline before them instead of after (`terminated`).
    terminate: bool,
    /// The write put a newline where the view had added its own terminator:
    /// the body may now carry one line more than its source (the empty line
    /// an insert session is about to fill). Checked when the session ends.
    recheck: bool,
}

impl Collection {
    /// The excerpt whose body holds view byte `at` (or ends there).
    fn excerpt_at(&self, at: usize) -> Option<&super::Excerpt> {
        let index = self
            .excerpts
            .partition_point(|excerpt| excerpt.view_start <= at)
            .checked_sub(1)?;
        self.excerpts.get(index)
    }

    /// Place the view edit `start..end -> text`, or refuse it. `line_start`:
    /// `start` begins a line (the byte before it is a newline).
    fn place(&self, start: usize, end: usize, text: &str, line_start: bool) -> Option<SourceWrite> {
        let excerpt = self.excerpt_at(start)?;
        let keeps_terminator = text.ends_with('\n')
            || (text.is_empty() && (line_start || start == excerpt.view_start));
        let (from, to) = strop_kernel::excerpt::source_edit(
            excerpt.view_start,
            excerpt.view_end,
            excerpt.start,
            excerpt.end,
            start,
            end,
            keeps_terminator,
        )?;
        let shown = excerpt.end - excerpt.start;
        let body = excerpt.view_end - excerpt.view_start;
        debug_assert!(body >= shown && body - shown <= 1);
        let added = body > shown;
        let offset = start - excerpt.view_start;
        Some(SourceWrite {
            source: excerpt.source,
            start: from,
            end: to,
            terminate: added && offset > shown,
            recheck: added && offset == shown && from == to && text.ends_with('\n'),
        })
    }
}

impl Editor {
    /// Admission for one edit of `document`, before any byte changes.
    /// `Ok(None)`: not a collection, nothing to write through.
    pub(crate) fn collection_admit(
        &self,
        document: DocumentId,
        start: usize,
        end: usize,
        text: &str,
    ) -> Result<Option<SourceWrite>, ExcerptRefusal> {
        let Some(collection) = self.collections.get(&document) else {
            return Ok(None);
        };
        if collection.voided {
            return Err(ExcerptRefusal::Chrome);
        }
        let buffer = &self.doc(document).buf;
        let line_start = start
            .checked_sub(1)
            .and_then(|at| buffer.byte_at(at))
            .is_none_or(|byte| byte == b'\n');
        let write = collection
            .place(start, end, text, line_start)
            .ok_or(ExcerptRefusal::Chrome)?;
        match self.docs.get(write.source) {
            Some(source) if !source.buf.readonly => Ok(Some(write)),
            Some(source) => Err(ExcerptRefusal::ReadonlySource(source.label(&self.cwd))),
            None => Err(ExcerptRefusal::Chrome),
        }
    }

    /// An edit was refused. When it was the first thing its command tried,
    /// the rest of the command is refused with it: `J` on a last line must
    /// not keep the space it adds after the join it could not make.
    pub(crate) fn collection_refused(&mut self, document: DocumentId) {
        if let Some(collection) = self.collections.get_mut(&document) {
            collection.voided |= !collection.touched;
        }
    }

    /// A command boundary: the next refusal is judged on its own.
    pub(crate) fn collection_command_boundary(&mut self, document: DocumentId) {
        if let Some(collection) = self.collections.get_mut(&document) {
            collection.voided = false;
            collection.touched = false;
        }
    }

    /// The view made the admitted edit: account for it and make the same
    /// edit in the source.
    pub(crate) fn collection_written(
        &mut self,
        document: DocumentId,
        write: SourceWrite,
        text: &str,
    ) {
        let Some(change) = self.doc(document).buf.changes().last().copied() else {
            return;
        };
        if let Some(collection) = self.collections.get_mut(&document) {
            collection.map_view_changes(&[change]);
            collection.mapped = change.revision;
            collection.touched = true;
            collection.recheck |= write.recheck;
        }
        let result = {
            let mut source = self.doc_mut(write.source);
            let range = Range::charwise(write.start, write.end);
            if write.terminate {
                source.buf.edit().replace(range, &terminated(text))
            } else {
                source.buf.edit().replace(range, text)
            }
        };
        if let Err(error) = result {
            self.collection_write_failed(document, &error.to_string());
        }
    }

    /// `u` / `Ctrl-R` is about to move the view's history. Each step the
    /// view recorded is one step in every source it changed, so the sources
    /// take the same move in their own histories: the exact inverse of what
    /// was written, not a second translation of it.
    pub(crate) fn collection_history_move(&mut self, document: DocumentId, undo: Option<bool>) {
        if let Some(collection) = self.collections.get_mut(&document) {
            collection.history_move = undo;
        }
    }

    /// Batches and history moves: the view's journal past what its geometry
    /// accounts for. A batch is composed and written per source; a history
    /// move is repeated in the histories of the sources it touched.
    pub(crate) fn collection_write_pending(&mut self, document: DocumentId) {
        let (changes, writes, moved) = {
            let Some(collection) = self.collections.get(&document) else {
                return;
            };
            let buffer = &self.doc(document).buf;
            let journal = buffer.changes();
            let first = journal.partition_point(|change| change.revision <= collection.mapped);
            let pending = &journal[first..];
            if pending.is_empty()
                || pending
                    .iter()
                    .any(|change| change.origin == ChangeOrigin::System)
            {
                // A system publication is rendered from the sources already.
                return;
            }
            let mut writes: Vec<(DocumentId, Replacement)> = Vec::new();
            let mut recheck = false;
            let mut refused = false;
            // The composed edits address the text before `pending`, which is
            // the text the view's geometry still describes. A batch was
            // admitted whole and a history move restores a body that ended
            // in a newline, so an emptied tail is a whole-line removal.
            let composed = super::journal::replacements(buffer, pending);
            let moved = collection.history_move.map(|undo| {
                let mut sources: Vec<DocumentId> = composed
                    .iter()
                    .filter_map(|edit| collection.excerpt_at(edit.range.start.get()))
                    .map(|excerpt| excerpt.source)
                    .collect();
                sources.sort_unstable_by_key(|source| source.index());
                sources.dedup();
                (undo, sources)
            });
            for edit in composed {
                if moved.is_some() {
                    break;
                }
                let (start, end) = (edit.range.start.get(), edit.range.end.get());
                match collection.place(start, end, &edit.text, true) {
                    Some(write) => {
                        recheck |= write.recheck;
                        let text = if write.terminate {
                            terminated(&edit.text)
                        } else {
                            edit.text
                        };
                        writes.push((
                            write.source,
                            Replacement::new(Range::charwise(write.start, write.end), text),
                        ));
                    }
                    None => refused = true,
                }
            }
            (
                pending.to_vec(),
                (!refused).then_some((writes, recheck)),
                moved,
            )
        };
        let Some(collection) = self.collections.get_mut(&document) else {
            return;
        };
        collection.map_view_changes(&changes);
        if let Some(last) = changes.last() {
            collection.mapped = last.revision;
        }
        if let Some((undo, sources)) = moved {
            collection.recheck = true;
            for source in sources {
                let moved = {
                    let mut source = self.doc_mut(source);
                    if undo {
                        source.buf.undo()
                    } else {
                        source.buf.redo()
                    }
                };
                if !matches!(moved, Ok(Some(_))) {
                    self.collection_write_failed(document, "a source's history had moved on");
                    return;
                }
            }
            if !self.collection_terminators_agree(document) {
                if let Some(collection) = self.collections.get_mut(&document) {
                    collection.rerender = true;
                }
            }
            return;
        }
        let Some((mut writes, recheck)) = writes else {
            self.collection_write_failed(document, "an edit left its excerpt");
            return;
        };
        collection.recheck |= recheck;
        writes.sort_by_key(|(source, edit)| (source.index(), edit.range.start.get()));
        let mut at = 0;
        while at < writes.len() {
            let source = writes[at].0;
            let mut edits = Vec::new();
            while at < writes.len() && writes[at].0 == source {
                edits.push(writes[at].1.clone());
                at += 1;
            }
            let Some(base) = self.docs.get(source).map(|doc| doc.buf.revision()) else {
                self.collection_write_failed(document, "a source was closed");
                return;
            };
            let changes = crate::editor::transact::ChangeSet {
                edits,
                undo_open: false,
            };
            if let Err(error) = self.apply(source, base, changes) {
                self.collection_write_failed(document, &error.to_string());
                return;
            }
        }
        // No insert session follows a batch or a history move.
        if !self.collection_terminators_agree(document) {
            if let Some(collection) = self.collections.get_mut(&document) {
                collection.rerender = true;
            }
        }
    }

    /// Whether every body is its source's text plus at most the terminator
    /// the view adds to an unterminated last line. Clears the pending check.
    fn collection_terminators_agree(&mut self, document: DocumentId) -> bool {
        let Some(collection) = self.collections.get_mut(&document) else {
            return true;
        };
        if !std::mem::take(&mut collection.recheck) {
            return true;
        }
        collection.excerpts.iter().all(|excerpt| {
            let shown = excerpt.end - excerpt.start;
            let added = excerpt.view_end - excerpt.view_start > shown;
            !(added
                && shown > 0
                && self
                    .docs
                    .get(excerpt.source)
                    .and_then(|source| source.buf.byte_at(excerpt.end - 1))
                    == Some(b'\n'))
        })
    }

    /// A write the view made did not reach its source: the sources are the
    /// truth, so the view is rendered from them again.
    fn collection_write_failed(&mut self, document: DocumentId, reason: &str) {
        if let Some(collection) = self.collections.get_mut(&document) {
            collection.rerender = true;
        }
        self.message = format!("collection: {reason}; the view was reloaded from its sources");
    }

    /// The view's lease is settled: its text is what the sources say, and
    /// its dirty bit is not the story (the sources own unsaved state).
    pub(crate) fn collection_settle(&mut self, document: DocumentId) {
        let Some(collection) = self.collections.get_mut(&document) else {
            return;
        };
        let rerender = std::mem::take(&mut collection.rerender);
        let Some(doc) = self.docs.get_mut(document) else {
            return;
        };
        collection.revision = doc.buf.revision();
        collection.mapped = collection.revision;
        doc.buf.mark_clean();
        if rerender {
            self.collection_render_view(document);
        }
    }

    /// The excerpt bodies of a collection's view, in order; empty for any
    /// other document.
    pub(crate) fn collection_body_spans(&self, document: DocumentId) -> Vec<(usize, usize)> {
        self.collections
            .get(&document)
            .into_iter()
            .flat_map(|collection| collection.excerpts.iter())
            .map(|excerpt| (excerpt.view_start, excerpt.view_end))
            .collect()
    }

    /// The distinct sources a collection shows, in card order.
    pub(crate) fn collection_source_documents(&self, document: DocumentId) -> Vec<DocumentId> {
        let mut sources: Vec<DocumentId> = self
            .collections
            .get(&document)
            .into_iter()
            .flat_map(|collection| collection.excerpts.iter().map(|excerpt| excerpt.source))
            .collect();
        sources.dedup();
        sources
    }

    /// The view's undo group closed: so do the groups it opened in its
    /// sources, and one `u` there undoes what one `u` here would.
    pub(crate) fn collection_commit_sources(&mut self, document: DocumentId) {
        for source in self.collection_source_documents(document) {
            if let Some(doc) = self.docs.get_mut(source) {
                doc.buf.commit_undo_group();
            }
        }
        self.collection_command_boundary(document);
        if !self.collection_terminators_agree(document) {
            self.collection_render_view(document);
        }
    }

    /// `:w` in a collection: write every dirty source it shows.
    pub(crate) fn collection_save(&mut self, close: bool) {
        let mut sources = self.collection_source_documents(self.current());
        sources.retain(|source| self.docs.get(*source).is_some_and(|doc| doc.buf.is_dirty()));
        if sources.is_empty() {
            self.message = "collection: no changed files to write".into();
        } else {
            self.write_documents("collection", sources, None);
        }
        if close {
            self.close_buffer(false);
        }
    }
}

/// Lines added below an unterminated last line, as the source spells them:
/// the newline moves in front, and the new last line stays unterminated as
/// the file was.
fn terminated(lines: &str) -> String {
    format!("\n{}", lines.strip_suffix('\n').unwrap_or(lines))
}
