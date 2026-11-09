//! Edit geometry (0045, 0057 VF02): how anchors move through an edit, and
//! which batches of edits may be applied at all. `Buffer::prepare_replacements`,
//! completion acceptance, `editor/transact.rs` and every selection/view
//! remap call these exact functions.
//!
//! Proved (proofs/Kernel/EditMap.lean):
//!
//! - `map_position` is right-affinity mapping: anchors before the edit stay,
//!   anchors at/after the old end shift by the delta, anchors inside the
//!   replaced range collapse to its start; it is monotone and stays within
//!   the post-edit buffer;
//! - `validate_batch` accepts exactly the batches production can apply in
//!   reverse order: strictly ascending starts, each edit in bounds, no edit
//!   reaching into the next. Each refusal names a real defect of the input;
//! - folding `map_position` over an accepted batch in production's reverse
//!   application order equals mapping once against the original coordinates
//!   (composition).

/// Map one anchor through one edit that replaced `start..old_end` with text
/// now ending at `new_end` (= `start` + replacement length). Right
/// affinity: an anchor at the old end moves with the text after it.
pub fn map_position(position: usize, start: usize, old_end: usize, new_end: usize) -> usize {
    if position < start {
        position
    } else if position >= old_end {
        new_end.saturating_add(position - old_end)
    } else {
        start
    }
}

/// Why a batch of `(start, end)` byte ranges cannot be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchRefusal {
    /// Some edit has `start > end` or ends past the buffer.
    OutOfBounds,
    /// Two neighbouring edits start at the same offset, or the earlier one
    /// reaches past the later one's start.
    Overlap,
    /// The batch is not ordered by start. Callers sort before validating, so
    /// this is a caller defect, never a user-visible refusal.
    Unsorted,
}

/// Validate a batch of `(start, end)` ranges, sorted by start, against a
/// buffer of `len` bytes. An accepted batch can be applied back to front
/// with every edit still addressing its original coordinates.
pub fn validate_batch(len: usize, edits: &[(usize, usize)]) -> Result<(), BatchRefusal> {
    let mut k = 0;
    while k < edits.len() {
        let current = edits[k];
        if current.0 > current.1 || current.1 > len {
            return Err(BatchRefusal::OutOfBounds);
        }
        if k > 0 {
            let previous = edits[k - 1];
            if previous.0 > current.0 {
                return Err(BatchRefusal::Unsorted);
            }
            if previous.0 == current.0 || previous.1 > current.0 {
                return Err(BatchRefusal::Overlap);
            }
        }
        k += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_follow_right_affinity() {
        // replace 4..8 with 2 bytes: new_end = 6
        assert_eq!(map_position(3, 4, 8, 6), 3);
        assert_eq!(map_position(5, 4, 8, 6), 4);
        assert_eq!(map_position(8, 4, 8, 6), 6);
        assert_eq!(map_position(10, 4, 8, 6), 8);
        // pure insertion at 4 of 3 bytes: an anchor at 4 moves right
        assert_eq!(map_position(4, 4, 4, 7), 7);
    }

    #[test]
    fn batches_accept_only_ordered_disjoint_in_bounds_edits() {
        assert_eq!(validate_batch(10, &[(0, 2), (3, 3), (6, 8)]), Ok(()));
        assert_eq!(validate_batch(10, &[(0, 3), (3, 4)]), Ok(()));
        assert_eq!(validate_batch(10, &[]), Ok(()));
        let refused = |edits: &[(usize, usize)]| validate_batch(10, edits).unwrap_err();
        assert_eq!(refused(&[(0, 4), (3, 5)]), BatchRefusal::Overlap);
        assert_eq!(refused(&[(3, 3), (3, 3)]), BatchRefusal::Overlap);
        assert_eq!(refused(&[(2, 11)]), BatchRefusal::OutOfBounds);
        assert_eq!(refused(&[(5, 4)]), BatchRefusal::OutOfBounds);
        assert_eq!(refused(&[(6, 8), (0, 2)]), BatchRefusal::Unsorted);
    }
}
