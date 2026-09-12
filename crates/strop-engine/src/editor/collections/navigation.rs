//! Source and file-card navigation.
use super::{CollectionRow, Editor};
use strop_core::id::DocumentId;

impl Editor {
    /// `g<Space>` in a collection, Enter on a header row, and
    /// `:collection source` (0049 §5): open the full source under the
    /// caret — the live document with its unsaved edits, never a disk
    /// reload. A body row maps to the exact source position; a header
    /// row opens the file at that excerpt's first line. The jump is
    /// recorded so Ctrl-O returns to the collection working context.
    pub fn collection_open_source_pub(&mut self) {
        self.collection_open_source();
    }

    pub(crate) fn collection_open_source(&mut self) {
        let id = self.current();
        let cursor_line = self.buf().line_of(self.head());
        let cursor_col = self.buf().col_of(self.head());
        let Some(collection) = self.collections.get(&id) else {
            return;
        };
        let mut target: Option<(DocumentId, usize)> = None;
        for excerpt in &collection.excerpts {
            if cursor_line == excerpt.view_line {
                // header row: the file, at this excerpt's first line
                target = Some((excerpt.source, excerpt.start));
                break;
            }
            if cursor_line > excerpt.view_line
                && cursor_line <= excerpt.view_line + excerpt.view_lines
            {
                // body row: same line-in-excerpt, same column
                let Some(source) = self.docs.get(excerpt.source) else {
                    break;
                };
                let source_line =
                    source.buf.line_of(excerpt.start) + (cursor_line - excerpt.view_line - 1);
                let line = source_line.min(source.buf.len_lines().saturating_sub(1));
                target = Some((
                    excerpt.source,
                    source
                        .buf
                        .clamp_boundary(source.buf.line_start(line).saturating_add(cursor_col)),
                ));
                break;
            }
        }
        let Some((document, head)) = target else {
            self.message = "not on an excerpt".into();
            return;
        };
        if self.docs.get(document).is_none() {
            self.message = "collection: that source was closed".into();
            return;
        }
        self.push_jump();
        self.jump_land(document, head);
        self.lsp_maybe_attach();
    }
}

impl Editor {
    /// `]f` / `[f` in a collection: next / previous file card (0049 §5's
    /// excerpt navigation through the command registry).
    pub fn collection_file_step_pub(&mut self, forward: bool) {
        self.collection_file_step(forward);
    }

    pub(crate) fn collection_file_step(&mut self, forward: bool) {
        let id = self.current();
        let Some(collection) = self.collections.get(&id) else {
            self.message = "file cards live in collections".into();
            return;
        };
        let caret = self.buf().line_of(self.head());
        let mut tops: Vec<usize> = Vec::new();
        for (row, kind) in collection.rows.iter().enumerate() {
            if matches!(kind, CollectionRow::CardTop(_)) {
                tops.push(row);
            }
        }
        let target = if forward {
            tops.iter().copied().find(|row| *row > caret)
        } else {
            tops.iter().copied().rev().find(|row| *row < caret)
        };
        let Some(row) = target else {
            self.message = if forward { "last card" } else { "first card" }.into();
            return;
        };
        self.push_jump();
        self.set_head(self.buf().line_start(row));
        self.clamp_cursor();
        self.scroll_to_cursor(self.view_rows());
    }
}

impl Editor {
    pub fn collection_excerpt_step(&mut self, forward: bool) {
        let Some(collection) = self.collections.get(&self.current()) else {
            self.message = "excerpt navigation belongs to a collection".into();
            return;
        };
        let after = collection
            .excerpts
            .partition_point(|excerpt| excerpt.view_start <= self.head());
        let index = if forward {
            Some(after)
        } else {
            after.checked_sub(2)
        };
        let target = index
            .and_then(|index| collection.excerpts.get(index))
            .map(|excerpt| excerpt.view_start);
        let Some(target) = target else {
            self.message = if forward {
                "last excerpt"
            } else {
                "first excerpt"
            }
            .into();
            return;
        };
        self.push_jump();
        self.set_head(target);
        self.place_jump_target();
    }
}
