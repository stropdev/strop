//! Where a view row is in its file, and file-card navigation. Hits are in
//! `quickfix.rs`.
use super::{CollectionRow, Editor};
use strop_core::id::DocumentId;

impl Editor {
    /// Shared source context for navigation actions, including a collection's
    /// source-bearing file header. Presentation chrome never becomes a path.
    pub(crate) fn navigation_source(&self) -> Option<(DocumentId, usize)> {
        if let Some(source) = self.source_position(self.current(), self.head()) {
            return Some(source);
        }
        let line = self.buf().line_of(self.head());
        self.collections
            .get(&self.current())?
            .excerpts
            .iter()
            .find(|excerpt| excerpt.view_line == line)
            .map(|excerpt| (excerpt.source, excerpt.start))
    }

    /// `]f` / `[f` in a collection: next / previous file card (0049 §5's
    /// excerpt navigation through the command registry).
    pub fn collection_file_step_pub(&mut self, forward: bool) {
        self.collection_file_step(forward);
    }

    pub(crate) fn collection_file_step(&mut self, forward: bool) {
        let id = self.current();
        let Some(collection) = self.collections.get(&id) else {
            self.message = "]f and [f move between the files of a collection".into();
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
            self.message = if forward { "last file" } else { "first file" }.into();
            return;
        };
        self.push_jump();
        self.set_head(self.buf().line_start(row));
        self.clamp_cursor();
        self.scroll_to_cursor(self.view_rows());
    }
}
