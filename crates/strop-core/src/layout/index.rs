//! Sparse byte/cell checkpoints. Built on a worker for long lines, then shared
//! with the UI; a seek replays at most one checkpoint interval, not the prefix.
use super::RopeGraphemes;
use crate::id::{ByteOffset, DisplayColumn, LineIndex};
use ropey::RopeSlice;
use std::sync::Arc;

pub const INLINE_LAYOUT_BYTES: usize = 4096;
const CHECKPOINT_GRAPHEMES: usize = 128;

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct LayoutCheckpoint {
    pub byte: ByteOffset,
    pub cell: DisplayColumn,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LineLayoutIndex {
    pub(crate) checkpoints: Vec<LayoutCheckpoint>,
    pub(crate) bytes: usize,
    pub(crate) tab: usize,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreparedLineLayout {
    pub line: LineIndex,
    pub index: Arc<LineLayoutIndex>,
}
impl LineLayoutIndex {
    pub fn build(text: RopeSlice<'_>, tab: usize, cancelled: impl Fn() -> bool) -> Option<Self> {
        let mut checkpoints = vec![LayoutCheckpoint::default()];
        let mut end = DisplayColumn::new(0);
        for (count, (span, _)) in RopeGraphemes::new(text, tab).enumerate() {
            if count % CHECKPOINT_GRAPHEMES == 0 {
                if cancelled() {
                    return None;
                }
                if span.byte != 0 {
                    checkpoints.push(LayoutCheckpoint {
                        byte: ByteOffset::new(span.byte),
                        cell: span.cell,
                    });
                }
            }
            end = span.cell + span.width;
        }
        if cancelled() {
            return None;
        }
        if checkpoints
            .last()
            .is_some_and(|point| point.byte.get() != text.len_bytes())
        {
            checkpoints.push(LayoutCheckpoint {
                byte: ByteOffset::new(text.len_bytes()),
                cell: end,
            });
        }
        Some(Self {
            checkpoints,
            bytes: text.len_bytes(),
            tab: tab.max(1),
        })
    }
    pub(crate) fn at_byte(&self, byte: usize, valid: usize) -> LayoutCheckpoint {
        let points = &self.checkpoints[..valid];
        points[points
            .partition_point(|point| point.byte.get() <= byte)
            .saturating_sub(1)]
    }
    pub(crate) fn at_cell(&self, cell: DisplayColumn, valid: usize) -> LayoutCheckpoint {
        let points = &self.checkpoints[..valid];
        points[points
            .partition_point(|point| point.cell <= cell)
            .saturating_sub(1)]
    }
    pub(crate) fn prefix_before(&self, byte: usize, valid: usize) -> usize {
        self.checkpoints[..valid]
            .partition_point(|point| point.byte.get() < byte)
            .max(1)
    }
}
