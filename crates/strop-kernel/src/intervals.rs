//! Byte intervals of retained syntax (0066 §3, 0111 D9): how highlight spans
//! and the coverage they vouch for follow an edit, and how an exact result
//! replaces a window of them. `strop-engine`'s `editor/analysis/presentation.rs`
//! keeps the spans' classes beside these intervals and asks every geometric
//! question here.
//!
//! An interval list is *sorted and disjoint*: every interval is non-empty and
//! ends at or before the next one starts. Outputs name, per interval, the
//! input interval it came from, so the owner carries a span's class across.
//!
//! Proved (proofs/Kernel/Intervals.lean):
//!
//! - `project` follows one edit: a byte after the edit is covered, by the same
//!   source interval, exactly when it lies outside the inserted text and its
//!   pre-image was covered — inserted text never borrows a class and surviving
//!   text never loses one; sorted-disjoint lists stay sorted and disjoint;
//! - `cover` is union with a window, `uncover` difference, `clip` intersection,
//!   each keeping a list sorted and disjoint;
//! - `replace_window` gives every byte inside the window the fresh result's
//!   interval and every byte outside it the old one.

use crate::editmap::map_position;
use alloc::vec::Vec;

fn least(a: usize, b: usize) -> usize {
    if a < b {
        a
    } else {
        b
    }
}

fn most(a: usize, b: usize) -> usize {
    if a < b {
        b
    } else {
        a
    }
}

/// Follow `ranges` through one edit that replaced `start..old_end` with text
/// now ending at `new_end`, appending `(start, end, source)` per surviving
/// fragment: the part before the edit keeps its coordinates, the part from
/// `old_end` on shifts with `map_position`, the replaced bytes carry nothing.
pub fn project(
    ranges: &[(usize, usize)],
    start: usize,
    old_end: usize,
    new_end: usize,
    out: &mut Vec<(usize, usize, usize)>,
) {
    let mut k = 0;
    while k < ranges.len() {
        let range = ranges[k];
        if range.0 < start {
            out.push((range.0, least(range.1, start), k));
        }
        if range.1 > old_end {
            let from = map_position(most(range.0, old_end), start, old_end, new_end);
            let to = map_position(range.1, start, old_end, new_end);
            out.push((from, to, k));
        }
        k += 1;
    }
}

/// Append `coverage` with the window `first..last` added, merging every
/// interval that overlaps or touches the window into one.
pub fn cover(
    coverage: &[(usize, usize)],
    first: usize,
    last: usize,
    out: &mut Vec<(usize, usize)>,
) {
    let mut k = 0;
    if first >= last {
        while k < coverage.len() {
            out.push(coverage[k]);
            k += 1;
        }
        return;
    }
    let mut low = first;
    let mut high = last;
    while k < coverage.len() {
        let interval = coverage[k];
        if interval.1 < first {
            out.push(interval);
        } else if interval.0 <= last {
            low = least(low, interval.0);
            high = most(high, interval.1);
        }
        k += 1;
    }
    out.push((low, high));
    k = 0;
    while k < coverage.len() {
        let interval = coverage[k];
        if interval.0 > last {
            out.push(interval);
        }
        k += 1;
    }
}

/// Append `coverage` with the window `first..last` removed, each part with
/// the index of the interval it came from.
pub fn uncover(
    coverage: &[(usize, usize)],
    first: usize,
    last: usize,
    out: &mut Vec<(usize, usize, usize)>,
) {
    let mut k = 0;
    while k < coverage.len() {
        let interval = coverage[k];
        let below = least(interval.1, first);
        if interval.0 < below {
            out.push((interval.0, below, k));
        }
        let above = most(interval.0, last);
        if above < interval.1 {
            out.push((above, interval.1, k));
        }
        k += 1;
    }
}

/// Append the result of an exact `fresh` result (inside `first..last`)
/// replacing that window of `spans`: the old spans' parts outside the window,
/// then `fresh`. A source below `spans.len()` is an old span, from it on the
/// fresh span `source - spans.len()`.
pub fn replace_window(
    spans: &[(usize, usize)],
    first: usize,
    last: usize,
    fresh: &[(usize, usize)],
    out: &mut Vec<(usize, usize, usize)>,
) {
    let mut k = 0;
    while k < spans.len() {
        let span = spans[k];
        let below = least(span.1, first);
        if span.0 < below {
            out.push((span.0, below, k));
        }
        k += 1;
    }
    k = 0;
    while k < fresh.len() {
        out.push((fresh[k].0, fresh[k].1, spans.len() + k));
        k += 1;
    }
    k = 0;
    while k < spans.len() {
        let span = spans[k];
        let above = most(span.0, last);
        if above < span.1 {
            out.push((above, span.1, k));
        }
        k += 1;
    }
}

/// Append the parts of `spans` inside `first..last`. The owner passes only
/// the spans that can reach the window (found by binary search) and offsets
/// the sources by where that slice starts.
pub fn clip(
    spans: &[(usize, usize)],
    first: usize,
    last: usize,
    out: &mut Vec<(usize, usize, usize)>,
) {
    let mut k = 0;
    while k < spans.len() {
        let span = spans[k];
        let low = most(span.0, first);
        let high = least(span.1, last);
        if low < high {
            out.push((low, high, k));
        }
        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn projection_drops_the_replaced_bytes() {
        let mut out = Vec::new();
        // replace 4..8 with 2 bytes (new_end 6)
        project(&[(0, 2), (3, 10), (12, 14)], 4, 8, 6, &mut out);
        assert_eq!(out, vec![(0, 2, 0), (3, 4, 1), (6, 8, 1), (10, 12, 2)]);
        out.clear();
        // an insertion splits the span around the new text
        project(&[(0, 4)], 2, 2, 5, &mut out);
        assert_eq!(out, vec![(0, 2, 0), (5, 7, 0)]);
    }

    #[test]
    fn coverage_is_union_and_difference() {
        let mut out = Vec::new();
        cover(&[(0, 2), (5, 7), (9, 12), (20, 22)], 6, 10, &mut out);
        assert_eq!(out, vec![(0, 2), (5, 12), (20, 22)]);
        out.clear();
        cover(&[(0, 2)], 2, 4, &mut out);
        assert_eq!(out, vec![(0, 4)]);
        out.clear();
        cover(&[(0, 2)], 3, 3, &mut out);
        assert_eq!(out, vec![(0, 2)]);
        out.clear();
        let mut parts = Vec::new();
        uncover(&[(0, 4), (6, 12)], 2, 8, &mut parts);
        assert_eq!(parts, vec![(0, 2, 0), (8, 12, 1)]);
    }

    #[test]
    fn a_window_is_replaced_and_clipped() {
        let mut out = Vec::new();
        replace_window(&[(0, 5), (8, 12)], 3, 10, &[(4, 6)], &mut out);
        assert_eq!(out, vec![(0, 3, 0), (4, 6, 2), (10, 12, 1)]);
        out.clear();
        clip(&[(0, 2), (3, 8), (9, 12)], 4, 10, &mut out);
        assert_eq!(out, vec![(4, 8, 1), (9, 10, 2)]);
    }
}
