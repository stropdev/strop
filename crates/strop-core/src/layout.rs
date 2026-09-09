//! The layout layer (0017, R6 in 0031): one line's byte↔display-cell maps
//! and grapheme boundaries. Every visible-line consumer — renderer, cursor
//! placement, selection overlays, diagnostics, mouse hit-testing — reads
//! this instead of deriving positions by char index.
//!
//! Cells are `id::DisplayColumn` (unsaturated usize): positions past 65535
//! columns stay exact; u16 exists only at the final terminal conversion in
//! the renderer. Storage stays byte-native (`ByteOffset` is canonical); this
//! is the single translation seam. `RopeGraphemes` streams clusters off a
//! rope slice so long lines never pay a whole-line String or layout vector.

use std::borrow::Cow;

use ropey::RopeSlice;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

use crate::id::DisplayColumn;

mod index;
pub use index::{LayoutCheckpoint, LineLayoutIndex, PreparedLineLayout, INLINE_LAYOUT_BYTES};

/// A terminal cell must contain printable text, never protocol bytes. Use the
/// same one-cell replacement in layout and emission; tab expansion is separate.
pub fn printable_grapheme(grapheme: &str) -> &str {
    if grapheme.chars().any(char::is_control) {
        "\u{fffd}"
    } else {
        grapheme
    }
}

/// Printable metadata with the same grapheme policy as buffer emission.
/// Preserve an already-owned label without copying when no control needs replacing.
pub fn printable_text<'a>(text: impl Into<Cow<'a, str>>) -> Cow<'a, str> {
    let text = text.into();
    if !text.chars().any(char::is_control) {
        return text;
    }
    let mut output = String::with_capacity(text.len());
    for grapheme in text.graphemes(true) {
        output.push_str(printable_grapheme(grapheme));
    }
    Cow::Owned(output)
}

/// One cluster's width: tabs expand to their stop from the ABSOLUTE cell;
/// everything else is the printable form's terminal width.
fn grapheme_width(text: &str, cell: DisplayColumn, tab: usize) -> usize {
    let tab = tab.max(1);
    if text == "\t" {
        tab - cell.get() % tab
    } else {
        UnicodeWidthStr::width(printable_grapheme(text))
    }
}

/// One grapheme cluster's placement on the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphemeSpan {
    /// Byte offset of the cluster's start within the line.
    pub byte: usize,
    /// Display cell where the cluster starts.
    pub cell: DisplayColumn,
    /// Display width in cells (0 for pure combining/zero-width, 2 for
    /// CJK/emoji, tab expands to its stop).
    pub width: usize,
}

/// Streaming grapheme iteration over a rope slice, chunk-aligned like
/// ropey's own iteration: only a cluster crossing rope chunks needs an
/// owned string. Carries the absolute cell so consumers never re-walk.
pub struct RopeGraphemes<'a> {
    text: RopeSlice<'a>,
    cursor: GraphemeCursor,
    chunk: &'a str,
    chunk_start: usize,
    cell: DisplayColumn,
    tab: usize,
    base_byte: usize,
}

impl<'a> RopeGraphemes<'a> {
    pub fn new(text: RopeSlice<'a>, tab: usize) -> Self {
        Self::new_at(text, tab, DisplayColumn::new(0))
    }

    /// Start at an absolute cell — virtual EOL annotations continue the
    /// line's tab stops instead of restarting at column zero.
    pub fn new_at(text: RopeSlice<'a>, tab: usize, cell: DisplayColumn) -> Self {
        let (chunk, chunk_start, _, _) = text.chunk_at_byte(0);
        Self {
            text,
            cursor: GraphemeCursor::new(0, text.len_bytes(), true),
            chunk,
            chunk_start,
            cell,
            tab: tab.max(1),
            base_byte: 0,
        }
    }
    /// Resume at a proven grapheme boundary while retaining original line bytes.
    pub fn from_checkpoint(text: RopeSlice<'a>, tab: usize, point: LayoutCheckpoint) -> Self {
        let mut iterator = Self::new_at(text.byte_slice(point.byte.get()..), tab, point.cell);
        iterator.base_byte = point.byte.get();
        iterator
    }
}

impl<'a> Iterator for RopeGraphemes<'a> {
    type Item = (GraphemeSpan, Cow<'a, str>);

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.cursor.cur_cursor();
        if start == self.text.len_bytes() {
            return None;
        }
        let end = loop {
            match self.cursor.next_boundary(self.chunk, self.chunk_start) {
                Ok(Some(end)) => break end,
                Ok(None) => return None,
                Err(GraphemeIncomplete::NextChunk) => {
                    let next = self.chunk_start + self.chunk.len();
                    let (chunk, offset, _, _) = self.text.chunk_at_byte(next);
                    self.chunk = chunk;
                    self.chunk_start = offset;
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    let (chunk, offset, _, _) = self.text.chunk_at_byte(end - 1);
                    self.cursor.provide_context(&chunk[..end - offset], offset);
                }
                Err(other) => unreachable!("forward grapheme traversal: {other:?}"),
            }
        };
        let text = if start >= self.chunk_start && end <= self.chunk_start + self.chunk.len() {
            // fast path: borrow straight out of the current chunk
            Cow::Borrowed(&self.chunk[start - self.chunk_start..end - self.chunk_start])
        } else {
            let slice = self.text.byte_slice(start..end);
            match slice.as_str() {
                Some(text) => Cow::Borrowed(text),
                None => Cow::Owned(slice.to_string()),
            }
        };
        let width = grapheme_width(&text, self.cell, self.tab);
        let span = GraphemeSpan {
            byte: self.base_byte + start,
            cell: self.cell,
            width,
        };
        self.cell += width;
        Some((span, text))
    }
}

/// A glyph's intersection with the visible cell window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellClip {
    /// Leftmost visible cell, relative to the window origin.
    pub x: usize,
    /// Visible cell count (a partially clipped wide glyph's cells).
    pub width: usize,
    /// The whole cluster is visible — false means the renderer must use
    /// styled blanks, never half a glyph.
    pub complete: bool,
}

/// Clip one grapheme span to `[origin, origin+width)`. `None` when the
/// cluster occupies no cell in the window (zero-width or fully outside).
pub fn clip(span: GraphemeSpan, origin: DisplayColumn, width: usize) -> Option<CellClip> {
    let left = span.cell.get().max(origin.get());
    let end = span.cell.get() + span.width;
    let right = end.min(origin.get().saturating_add(width));
    (left < right).then(|| CellClip {
        x: left - origin.get(),
        width: right - left,
        complete: left == span.cell.get() && right == end,
    })
}

/// A laid-out line: grapheme spans in order.
#[derive(Debug, Clone, Default)]
pub struct LineLayout {
    spans: Vec<GraphemeSpan>,
    /// The line's byte length (cursor-at-end needs it).
    pub len_bytes: usize,
    /// Total rendered width in cells.
    pub width: DisplayColumn,
}

impl LineLayout {
    /// Lay out one content line. Tabs expand to stops; control graphemes use
    /// the same visible one-cell replacement as the renderer.
    pub fn build(text: &str, tab: usize) -> Self {
        let mut spans = Vec::new();
        let mut cell = DisplayColumn::new(0);
        for (byte, text) in text.grapheme_indices(true) {
            let width = grapheme_width(text, cell, tab);
            spans.push(GraphemeSpan { byte, cell, width });
            cell += width;
        }
        Self {
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
    pub fn cell_at_byte(&self, byte: usize) -> DisplayColumn {
        if byte >= self.len_bytes {
            return self.width;
        }
        let next = self.spans.partition_point(|s| s.byte <= byte);
        next.checked_sub(1)
            .map_or(DisplayColumn::new(0), |i| self.spans[i].cell)
    }

    /// Byte offset of the cluster at a display cell (mouse hit-testing,
    /// desired-column). A cell inside a wide cluster maps to its start;
    /// past the end maps to the line's byte length... caller clamps.
    pub fn byte_at_cell(&self, cell: DisplayColumn) -> usize {
        if cell >= self.width {
            return self.len_bytes;
        }
        let next = self.spans.partition_point(|s| s.cell <= cell);
        next.checked_sub(1).map_or(0, |i| self.spans[i].byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_tabs_and_unicode_cells() {
        let text = "ab\t界e\u{301}\x1bZ";
        for (tab, cells, width) in [(3, [0, 1, 2, 3, 5, 6, 7], 8), (4, [0, 1, 2, 4, 6, 7, 8], 9)] {
            let layout = LineLayout::build(text, tab);
            assert_eq!(
                layout
                    .spans()
                    .iter()
                    .map(|s| s.cell.get())
                    .collect::<Vec<_>>(),
                cells
            );
            assert_eq!(layout.width.get(), width);
            assert_eq!(layout.cell_at_byte(8).get(), cells[4]);
            assert_eq!(layout.byte_at_cell(DisplayColumn::new(cells[3] + 1)), 3);
            assert_eq!(layout.byte_at_cell(DisplayColumn::new(width)), text.len());
        }
    }

    #[test]
    fn columns_do_not_alias_at_terminal_limit() {
        let text = format!("{}\t界Z", "x".repeat(70_001));
        let layout = LineLayout::build(&text, 4);
        assert_eq!(layout.cell_at_byte(70_002).get(), 70_004);
        assert_eq!(layout.cell_at_byte(70_005).get(), 70_006);
        assert_eq!(layout.byte_at_cell(DisplayColumn::new(70_005)), 70_002);
        assert_eq!(LineLayout::build("x\tZ", 300).cell_at_byte(2).get(), 300);
    }

    #[test]
    fn rope_chunk_boundaries_preserve_extended_clusters() {
        let text = format!("{}e{}\t界🧑‍🚀Z", "a".repeat(997), "\u{301}".repeat(2000));
        let rope = ropey::Rope::from_str(&text);
        let got = RopeGraphemes::new(rope.slice(..), 3).collect::<Vec<_>>();
        assert_eq!(
            got[997].0,
            GraphemeSpan {
                byte: 997,
                cell: DisplayColumn::new(997),
                width: 1
            }
        );
        assert_eq!(got[997].1, format!("e{}", "\u{301}".repeat(2000)));
        let tail = got
            .iter()
            .skip(998)
            .map(|(s, t)| (s.cell.get(), s.width, t.as_ref()))
            .collect::<Vec<_>>();
        assert_eq!(
            tail,
            [
                (998, 1, "\t"),
                (999, 2, "界"),
                (1001, 2, "🧑‍🚀"),
                (1003, 1, "Z")
            ]
        );
    }

    #[test]
    fn clipping_preserves_cells_not_partial_glyphs() {
        let span = GraphemeSpan {
            byte: 0,
            cell: DisplayColumn::new(4),
            width: 2,
        };
        assert_eq!(
            clip(span, DisplayColumn::new(5), 4),
            Some(CellClip {
                x: 0,
                width: 1,
                complete: false
            })
        );
        assert_eq!(
            clip(span, DisplayColumn::new(3), 2),
            Some(CellClip {
                x: 1,
                width: 1,
                complete: false
            })
        );
        assert_eq!(
            clip(span, DisplayColumn::new(3), 3),
            Some(CellClip {
                x: 1,
                width: 2,
                complete: true
            })
        );
    }
}
