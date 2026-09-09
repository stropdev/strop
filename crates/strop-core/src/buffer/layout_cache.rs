//! Per-buffer presentation cache. Edits keep only proven unchanged prefixes;
//! worker results install only at their exact revision. Text remains the rope.
use super::Buffer;
use crate::id::{BufferRevision, DisplayColumn, LineIndex};
use crate::layout::{
    LayoutCheckpoint, LineLayoutIndex, PreparedLineLayout, RopeGraphemes, INLINE_LAYOUT_BYTES,
};
use crate::InputEdit;
use std::cell::RefCell;
use std::sync::Arc;

struct CachedLine {
    line: LineIndex,
    index: Arc<LineLayoutIndex>,
    valid: usize,
}
#[derive(Default)]
pub(super) struct LineLayouts {
    entries: RefCell<Vec<CachedLine>>,
}
impl Buffer {
    pub fn prepare_line_layout(
        &self,
        line: LineIndex,
        tab: usize,
        cancelled: impl Fn() -> bool,
    ) -> Option<PreparedLineLayout> {
        let text = self
            .text()
            .byte_slice(self.line_start(line)..self.line_end(line));
        if text.len_bytes() <= INLINE_LAYOUT_BYTES {
            return None;
        }
        LineLayoutIndex::build(text, tab, cancelled).map(|index| PreparedLineLayout {
            line,
            index: Arc::new(index),
        })
    }

    pub fn install_line_layouts(
        &mut self,
        revision: BufferRevision,
        layouts: &[PreparedLineLayout],
    ) -> bool {
        if revision != self.revision() {
            return false;
        }
        for layout in layouts {
            if layout.line.get() >= self.len_lines()
                || layout.index.bytes != self.line_end(layout.line) - self.line_start(layout.line)
                || layout.index.checkpoints.is_empty()
                || layout.index.checkpoints[0].byte.get() != 0
                || layout.index.checkpoints[0].cell.get() != 0
                || layout.index.checkpoints.windows(2).any(|points| {
                    points[0].byte >= points[1].byte || points[0].cell > points[1].cell
                })
                || layout
                    .index
                    .checkpoints
                    .last()
                    .is_some_and(|point| point.byte.get() > layout.index.bytes)
            {
                return false;
            }
        }
        let entries = self.line_layouts.entries.get_mut();
        for layout in layouts {
            let value = CachedLine {
                line: layout.line,
                index: layout.index.clone(),
                valid: layout.index.checkpoints.len(),
            };
            if let Some(entry) = entries
                .iter_mut()
                .find(|entry| entry.line == layout.line && entry.index.tab == layout.index.tab)
            {
                *entry = value;
            } else {
                const CACHED_LINES: usize = 256;
                if entries.len() == CACHED_LINES {
                    entries.remove(0);
                }
                entries.push(value);
            }
        }
        true
    }

    pub(super) fn invalidate_line_layouts(&mut self, edit: &InputEdit) {
        let same_line = edit.start_point.0 == edit.old_end_point.0
            && edit.start_point.0 == edit.new_end_point.0;
        self.line_layouts.entries.get_mut().retain_mut(|entry| {
            if entry.line.get() < edit.start_point.0 {
                return true;
            }
            if !same_line {
                return false;
            }
            if entry.line.get() == edit.start_point.0 {
                entry.valid = entry.index.prefix_before(edit.start_point.1, entry.valid);
            }
            true
        });
    }

    pub fn layout_checkpoint(
        &self,
        line: LineIndex,
        origin: DisplayColumn,
        tab: usize,
    ) -> Option<LayoutCheckpoint> {
        let entries = self.line_layouts.entries.borrow();
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.line == line && entry.index.tab == tab.max(1))
        {
            let checkpoint = entry.index.at_cell(origin, entry.valid);
            if entry.valid == entry.index.checkpoints.len()
                || origin.get().saturating_sub(checkpoint.cell.get()) <= INLINE_LAYOUT_BYTES
            {
                return Some(checkpoint);
            }
        }
        (origin.get() == 0 || self.line_end(line) - self.line_start(line) <= INLINE_LAYOUT_BYTES)
            .then_some(LayoutCheckpoint::default())
    }

    pub fn try_cell_col_with_tab(&self, offset: usize, tab: usize) -> Option<DisplayColumn> {
        self.column_from_layout(offset, tab, true)
    }

    pub(super) fn column_from_layout(
        &self,
        offset: usize,
        tab: usize,
        bounded: bool,
    ) -> Option<DisplayColumn> {
        let offset = offset.min(self.len_bytes());
        let line = LineIndex::new(self.line_of(offset));
        let start = self.line_start(line);
        let text = self.text().byte_slice(start..self.line_end(line));
        let byte = offset.saturating_sub(start).min(text.len_bytes());
        let (point, indexed) = self
            .line_layouts
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.line == line && entry.index.tab == tab.max(1))
            .map_or((LayoutCheckpoint::default(), false), |entry| {
                (
                    entry.index.at_byte(byte, entry.valid),
                    entry.valid == entry.index.checkpoints.len(),
                )
            });
        if bounded && !indexed && byte.saturating_sub(point.byte.get()) > INLINE_LAYOUT_BYTES {
            return None;
        }
        let mut end = point.cell;
        for (span, cluster) in RopeGraphemes::from_checkpoint(text, tab, point) {
            if byte < span.byte + cluster.len() {
                return Some(span.cell);
            }
            end = span.cell + span.width;
        }
        Some(end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{id::ByteOffset, Range};

    fn warm(buffer: &mut Buffer, line: usize, tab: usize) -> PreparedLineLayout {
        let layout = buffer
            .prepare_line_layout(LineIndex::new(line), tab, || false)
            .unwrap();
        assert!(buffer.install_line_layouts(buffer.revision(), std::slice::from_ref(&layout)));
        layout
    }

    #[test]
    fn indexed_unicode_cells_and_byte_offsets_match_the_frozen_line() {
        let source = format!("{}\t界👩‍💻e\u{301} tail", "ab".repeat(4096));
        let mut buffer = Buffer::from_text(&source);
        warm(&mut buffer, 0, 4);
        let plain = Buffer::from_snapshot(buffer.snapshot());
        for (byte, _) in source.char_indices().filter(|(byte, _)| *byte >= 8192) {
            assert_eq!(
                buffer.try_cell_col_with_tab(byte, 4),
                Some(plain.cell_col_with_tab(byte, 4))
            );
        }
        let origin = DisplayColumn::new(8196);
        let checkpoint = buffer
            .layout_checkpoint(LineIndex::new(0), origin, 4)
            .unwrap();
        let text = buffer.text().slice(..);
        let indexed: Vec<_> = RopeGraphemes::from_checkpoint(text, 4, checkpoint)
            .filter(|(span, _)| span.cell >= origin)
            .map(|(span, text)| (span.byte, span.cell, text.into_owned()))
            .collect();
        let expected: Vec<_> = RopeGraphemes::new(text, 4)
            .filter(|(span, _)| span.cell >= origin)
            .map(|(span, text)| (span.byte, span.cell, text.into_owned()))
            .collect();
        assert_eq!(indexed, expected);
    }

    #[test]
    fn edits_keep_only_proven_prefixes_and_reject_old_publications() {
        let mut buffer = Buffer::from_text(&format!("{}\t界end", "a".repeat(8192)));
        let old_revision = buffer.revision();
        let old = warm(&mut buffer, 0, 4);
        buffer.edit().insert(ByteOffset::new(8196), "Z").unwrap();
        assert!(!buffer.install_line_layouts(old_revision, &[old]));
        let byte = buffer.len_bytes();
        let fresh = Buffer::from_snapshot(buffer.snapshot());
        assert_eq!(
            buffer.try_cell_col_with_tab(byte, 4),
            Some(fresh.cell_col_with_tab(byte, 4))
        );
        buffer
            .edit()
            .replace(Range::charwise(0usize, 1usize), "\t")
            .unwrap();
        // A distant changed prefix cannot silently reuse its former cells.
        assert!(buffer.try_cell_col_with_tab(byte, 4).is_none());
        warm(&mut buffer, 0, 4);
        let fresh = Buffer::from_snapshot(buffer.snapshot());
        assert_eq!(
            buffer.try_cell_col_with_tab(byte, 4),
            Some(fresh.cell_col_with_tab(byte, 4))
        );
    }
}
