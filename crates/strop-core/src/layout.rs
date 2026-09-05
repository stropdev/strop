//! The layout layer (0017): one line's byte↔display-cell maps and
//! grapheme boundaries. Every visible-line consumer — renderer, cursor
//! placement, selection overlays, diagnostics, mouse hit-testing —
//! reads this instead of deriving positions by char index (the old
//! `.chars().enumerate()` walk drifted after the first multibyte char).
//!
//! Storage stays byte-native (`ByteOffset` is canonical); this is the
//! single translation seam.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// One grapheme cluster's placement on the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphemeSpan {
    /// Byte offset of the cluster's start within the line.
    pub byte: usize,
    /// Display cell where the cluster starts.
    pub cell: u16,
    /// Display width in cells (0 for pure combining/zero-width, 2 for
    /// CJK/emoji, tab expands to its stop).
    pub width: u8,
}

/// A laid-out line: grapheme spans in order.
#[derive(Debug, Clone, Default)]
pub struct LineLayout {
    spans: Vec<GraphemeSpan>,
    /// The line's byte length (cursor-at-end needs it).
    pub len_bytes: usize,
    /// Total rendered width in cells.
    pub width: u16,
}

impl LineLayout {
    /// Lay out one line's text (no trailing newline). `tab` is the tab
    /// stop; control chars render zero-width (terminals show them raw).
    pub fn build(text: &str, tab: u16) -> Self {
        let tab = tab.max(1);
        let mut spans = Vec::with_capacity(text.len() / 2 + 4);
        let mut cell: u16 = 0;
        for (byte, g) in text.grapheme_indices(true) {
            let w = if g == "\t" {
                (tab - cell % tab) as u8
            } else {
                UnicodeWidthStr::width(g).min(u8::MAX as usize) as u8
            };
            spans.push(GraphemeSpan {
                byte,
                cell,
                width: w,
            });
            cell = cell.saturating_add(w as u16);
        }
        LineLayout {
            spans,
            len_bytes: text.len(),
            width: cell,
        }
    }

    /// The grapheme spans, in order.
    pub fn spans(&self) -> &[GraphemeSpan] {
        &self.spans
    }

    /// Display cell where a byte offset renders (cursor placement).
    /// A byte mid-cluster maps to the cluster's cell; at/past the end
    /// maps to the line's end cell (vim's virtual cursor position).
    pub fn cell_at_byte(&self, byte: usize) -> u16 {
        if byte >= self.len_bytes {
            return self.width;
        }
        self.spans
            .iter()
            .rev()
            .find(|s| s.byte <= byte)
            .map(|s| s.cell)
            .unwrap_or(0)
    }

    /// Byte offset of the cluster at a display cell (mouse hit-testing,
    /// desired-column). A cell inside a wide cluster maps to its start;
    /// past the end maps to the line's byte length... caller clamps.
    pub fn byte_at_cell(&self, cell: u16) -> usize {
        match self.spans.iter().rev().find(|s| s.cell <= cell) {
            Some(s) => s.byte,
            None => 0,
        }
    }

    /// Is the line free of wide/zero-width complications? Hot paths may
    /// skip the layout for the common ASCII case.
    pub fn is_ascii_fast(&self) -> bool {
        self.spans.iter().all(|s| s.width == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_identity() {
        let l = LineLayout::build("hello", 8);
        assert_eq!(l.width, 5);
        assert_eq!(l.cell_at_byte(3), 3);
        assert_eq!(l.byte_at_cell(3), 3);
        assert!(l.is_ascii_fast());
    }

    #[test]
    fn cjk_is_two_cells() {
        let l = LineLayout::build("a界b", 8);
        assert_eq!(l.width, 4);
        assert_eq!(l.cell_at_byte(1), 1); // 界 starts at cell 1
        assert_eq!(l.cell_at_byte(4), 3); // b (byte 4) at cell 3
        assert_eq!(l.byte_at_cell(3), 4);
    }

    #[test]
    fn emoji_cluster_is_one_unit() {
        let l = LineLayout::build("x\u{1F9D1}\u{200D}\u{1F680}y", 8); // 🧑‍🚀
        assert_eq!(l.spans().len(), 3);
        assert_eq!(l.spans()[1].width, 2);
        assert_eq!(l.width, 4);
    }

    #[test]
    fn tab_expands_to_its_stop() {
        let l = LineLayout::build("ab\tc", 4);
        assert_eq!(l.spans()[2].width, 2); // cell 2 → stop at 4
        assert_eq!(l.cell_at_byte(3), 4); // c at cell 4
        assert_eq!(l.width, 5);
    }
}
