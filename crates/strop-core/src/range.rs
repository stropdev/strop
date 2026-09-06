//! Ranges with vim shape (0014): charwise ops carry the motion
//! inclusivity (dfx vs dtx differ by it); linewise is line-shaped.

use crate::id;

/// How vim thinks about a range (0014): charwise ops carry the motion's
/// inclusivity (dfx vs dtx differ by it); linewise is line-shaped.
/// Blockwise lands with visual block — the enum is the extension point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionShape {
    Characterwise { inclusive: bool },
    Linewise,
}

/// A half-open byte range `[start, end)` plus its vim shape. Fields are
/// ByteOffset — the storage coordinate is typed end to end (0014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: usize,
    pub shape: MotionShape,
}

impl Range {
    pub fn charwise(start: impl Into<id::ByteOffset>, end: impl Into<id::ByteOffset>) -> Self {
        let (start, end) = (start.into().get(), end.into().get());
        debug_assert!(start <= end);
        Self {
            start,
            end,
            shape: MotionShape::Characterwise { inclusive: false },
        }
    }
    pub fn linewise(start: impl Into<id::ByteOffset>, end: impl Into<id::ByteOffset>) -> Self {
        let (start, end) = (start.into().get(), end.into().get());
        debug_assert!(start <= end);
        Self {
            start,
            end,
            shape: MotionShape::Linewise,
        }
    }
    pub fn is_linewise(&self) -> bool {
        matches!(self.shape, MotionShape::Linewise)
    }
    /// The resolver's inclusive flag folds into the shape (0014).
    pub fn with_inclusive(mut self, inclusive: bool) -> Self {
        if let MotionShape::Characterwise { inclusive: i } = &mut self.shape {
            *i = inclusive;
        }
        self
    }
    pub fn inclusive(&self) -> bool {
        matches!(self.shape, MotionShape::Characterwise { inclusive: true })
    }
    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}
