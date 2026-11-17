//! Edit geometry (0045, 0057 VF02, 0108 D4): how anchors and spans move
//! through an edit, and which batches of edits may be applied at all.
//! `Buffer::prepare_replacements`, completion acceptance and the multicursor
//! plan admit batches here; every engine remap of a point through a journal
//! calls `map_through_edits`, and every remap of a span calls `map_span`.
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
//! - `map_through_edits` folds `map_position` over a journal in application
//!   order, monotonically; over the journal of an accepted batch (its edits
//!   applied back to front) it equals classifying the anchor once against the
//!   original coordinates (composition);
//! - `map_span` keeps a span ordered and inside the edited buffer, keeps two
//!   ordered spans ordered, changes a span's length by exactly the edit's
//!   delta when the edit lies inside it and not at all when it lies outside,
//!   and moves an end the edit starts at only when the caller says the
//!   inserted text belongs to the span;
//! - `keep_disjoint` keeps a range exactly when it starts at or after the end
//!   of every range kept before it, so the kept non-empty ranges are a batch
//!   `validate_batch` accepts.

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

/// Map one anchor through a journal of edits `(start, old_end, new_end)`
/// in application order: each edit addresses the buffer the previous ones
/// left. A batch `validate_batch` accepted is applied back to front, so its
/// journal is the batch in descending order.
pub fn map_through_edits(position: usize, edits: &[(usize, usize, usize)]) -> usize {
    let mut mapped = position;
    let mut k = 0;
    while k < edits.len() {
        let edit = edits[k];
        mapped = map_position(mapped, edit.0, edit.1, edit.2);
        k += 1;
    }
    mapped
}

/// Map the span `start..end` through one edit that replaced `s..e` with text
/// now ending at `new_end`. The start is left-affine (an edit at the start
/// lands inside the span, and a start inside the replaced range stays at the
/// edit's start); an end inside or at the end of the replaced range takes the
/// whole replacement in. An edit that starts exactly at `end` belongs to the
/// span only when `grow_at_end` says so. Ends past the edit shift by its delta,
/// saturating like `map_position`.
pub fn map_span(
    start: usize,
    end: usize,
    s: usize,
    e: usize,
    new_end: usize,
    grow_at_end: bool,
) -> (usize, usize) {
    let mapped_start = if start <= s {
        start
    } else if start >= e {
        new_end.saturating_add(start - e)
    } else {
        s
    };
    let mapped_end = if end < s || (end == s && !grow_at_end) {
        end
    } else if end <= e {
        new_end
    } else {
        new_end.saturating_add(end - e)
    };
    (mapped_start, mapped_end)
}

/// The multicursor plan filter: over `ranges` sorted by start, keep a range
/// only when it starts at or after the end of every range kept before it (the
/// first cursor to claim a region owns it). `keep[k]` receives the verdict for
/// `ranges[k]`; `keep` must be as long as `ranges`.
pub fn keep_disjoint(ranges: &[(usize, usize)], keep: &mut [bool]) {
    // The end of the last kept range: with every range ordered, the
    // furthest any kept range reaches.
    let mut reach = 0;
    let mut k = 0;
    while k < ranges.len() && k < keep.len() {
        let range = ranges[k];
        if range.0 >= reach {
            keep[k] = true;
            reach = range.1;
        } else {
            keep[k] = false;
        }
        k += 1;
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
    fn journals_map_in_application_order() {
        // batch (0..4 -> ""), (6..8 -> "x"), journaled back to front
        let journal = [(6, 8, 7), (0, 4, 0)];
        assert_eq!(map_through_edits(7, &journal), 2);
        assert_eq!(map_through_edits(9, &journal), 4);
        assert_eq!(map_through_edits(2, &journal), 0);
        assert_eq!(map_through_edits(5, &[]), 5);
    }

    #[test]
    fn spans_keep_their_edges() {
        // replace 4..8 with 2 bytes (new_end 6) around span 2..10
        assert_eq!(map_span(2, 10, 4, 8, 6, false), (2, 8));
        // an edit replacing the span's first bytes keeps its start
        assert_eq!(map_span(4, 10, 4, 8, 6, false), (4, 8));
        // a start inside the replaced range stays at the edit's start
        assert_eq!(map_span(5, 10, 4, 8, 6, false), (4, 8));
        // an end inside the replaced range takes the replacement in
        assert_eq!(map_span(0, 5, 4, 8, 6, false), (0, 6));
        // an edit starting at the end belongs to the span only on request
        assert_eq!(map_span(0, 4, 4, 8, 6, false), (0, 4));
        assert_eq!(map_span(0, 4, 4, 4, 7, false), (0, 4));
        assert_eq!(map_span(0, 4, 4, 4, 7, true), (0, 7));
        // spans wholly after shift, wholly before stay
        assert_eq!(map_span(9, 12, 4, 8, 6, false), (7, 10));
        assert_eq!(map_span(0, 3, 4, 8, 6, false), (0, 3));
    }

    #[test]
    fn plans_keep_the_first_claim_on_a_region() {
        let mut keep = [false; 5];
        keep_disjoint(&[(0, 4), (2, 6), (4, 4), (4, 9), (9, 10)], &mut keep);
        assert_eq!(keep, [true, false, true, true, true]);
        let mut keep = [true; 2];
        keep_disjoint(&[(3, 8), (5, 6)], &mut keep);
        assert_eq!(keep, [true, false]);
        keep_disjoint(&[], &mut []);
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
