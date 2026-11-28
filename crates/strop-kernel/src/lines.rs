//! Line geometry across a diff (plans 2026-11-28 line-geometry kernel): the
//! rows a selection covers, the change regions a hunk body denotes, whether a
//! region list describes a diff, and where a line is on the other side of
//! one. `strop-git`'s `Hunk::regions` feeds every hunk body through `runs`;
//! `strop-engine`'s hunk set checks the concatenated list with `coherent` and
//! maps buffer, index and HEAD lines through it.
//!
//! Lines are 0-based. A region says old lines `old..old + old_len` became new
//! lines `new..new + new_len`; a pure insertion has `old_len == 0`, a pure
//! deletion `new_len == 0`, and then `old`/`new` name the line the gap sits
//! before on that side.
//!
//! Proved (proofs/Kernel/Lines.lean):
//!
//! - `runs` appends exactly the regions of the body: one per maximal stretch
//!   of non-context lines, its lengths counting that stretch's deletions and
//!   additions and its starts the lines before it on each side; what it
//!   produces for one hunk is always a diff `coherent` accepts;
//! - `coherent` accepts exactly the region lists where every region starts
//!   at or after the previous one's end on both sides, at the same distance
//!   on both, is non-empty, and ends inside `usize`;
//! - `span` orders a selection's two ends;
//! - over a coherent list, `old_line` and `new_line` answer exactly the
//!   kept lines (lines no region added, deleted or replaced), paired at the
//!   same offset into matching gaps, and `old_span` answers a range exactly
//!   when every line of it is kept and they stay consecutive on the old
//!   side; a kept line is never a deleted or an added line and the pairing
//!   keeps order. None of the four fails on any input.

use alloc::vec::Vec;

/// One change run between two versions of a file, in 0-based lines: old
/// lines `old..old + old_len` became new lines `new..new + new_len`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub old: usize,
    pub old_len: usize,
    pub new: usize,
    pub new_len: usize,
}

/// A unified hunk body line, by which sides it exists on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Context,
    Addition,
    Deletion,
}

/// Append `run` when it changed anything.
fn close(run: Region, out: &mut Vec<Region>) {
    if run.old_len != 0 || run.new_len != 0 {
        out.push(run);
    }
}

/// Append the change runs of one hunk body whose first line is old line
/// `old_start` and new line `new_start`: one region per maximal stretch of
/// non-context lines.
pub fn runs(old_start: usize, new_start: usize, body: &[Origin], out: &mut Vec<Region>) {
    let mut run = Region {
        old: old_start,
        old_len: 0,
        new: new_start,
        new_len: 0,
    };
    let mut k = 0;
    while k < body.len() {
        match body[k] {
            Origin::Context => {
                close(run, out);
                run = Region {
                    old: run.old.saturating_add(run.old_len).saturating_add(1),
                    old_len: 0,
                    new: run.new.saturating_add(run.new_len).saturating_add(1),
                    new_len: 0,
                };
            }
            Origin::Deletion => run.old_len = run.old_len.saturating_add(1),
            Origin::Addition => run.new_len = run.new_len.saturating_add(1),
        }
        k += 1;
    }
    close(run, out);
}

/// Whether `regions` describes a diff: each region starts at or after the
/// previous one's end on both sides, at the same distance on both, is not
/// empty on both sides, and ends inside `usize` on both.
pub fn coherent(regions: &[Region]) -> bool {
    let mut old_base = 0;
    let mut new_base = 0;
    let mut k = 0;
    while k < regions.len() {
        let r = regions[k];
        if r.old < old_base || r.new < new_base {
            return false;
        }
        if r.old - old_base != r.new - new_base {
            return false;
        }
        if r.old_len == 0 && r.new_len == 0 {
            return false;
        }
        if r.old_len > usize::MAX - r.old || r.new_len > usize::MAX - r.new {
            return false;
        }
        old_base = r.old + r.old_len;
        new_base = r.new + r.new_len;
        k += 1;
    }
    true
}

/// The ordered inclusive rows a selection covers, whichever end is the anchor.
pub fn span(anchor: usize, head: usize) -> (usize, usize) {
    if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    }
}

/// `to + (line - from)` when it fits a `usize`; callers pass `from <= line`.
fn shift(line: usize, from: usize, to: usize) -> Option<usize> {
    let delta = line - from;
    if delta > usize::MAX - to {
        None
    } else {
        Some(to + delta)
    }
}

/// Where new-side line `line` is on the old side; `None` when a region
/// added or replaced it.
pub fn old_line(line: usize, regions: &[Region]) -> Option<usize> {
    let mut old_base = 0;
    let mut new_base = 0;
    let mut k = 0;
    while k < regions.len() {
        let r = regions[k];
        if line < r.new {
            break;
        }
        if line < r.new.saturating_add(r.new_len) {
            return None;
        }
        old_base = r.old.saturating_add(r.old_len);
        new_base = r.new.saturating_add(r.new_len);
        k += 1;
    }
    shift(line, new_base, old_base)
}

/// Where old-side line `line` is on the new side; `None` when a region
/// deleted or replaced it.
pub fn new_line(line: usize, regions: &[Region]) -> Option<usize> {
    let mut old_base = 0;
    let mut new_base = 0;
    let mut k = 0;
    while k < regions.len() {
        let r = regions[k];
        if line < r.old {
            break;
        }
        if line < r.old.saturating_add(r.old_len) {
            return None;
        }
        old_base = r.old.saturating_add(r.old_len);
        new_base = r.new.saturating_add(r.new_len);
        k += 1;
    }
    shift(line, old_base, new_base)
}

/// The old-side lines of new-side lines `first..=last` when all of them are
/// kept and no deletion falls between them; `None` otherwise.
pub fn old_span(first: usize, last: usize, regions: &[Region]) -> Option<(usize, usize)> {
    if last < first {
        return None;
    }
    let mut old_base = 0;
    let mut new_base = 0;
    let mut k = 0;
    while k < regions.len() {
        let r = regions[k];
        if last < r.new {
            break;
        }
        if first < r.new.saturating_add(r.new_len) {
            return None;
        }
        old_base = r.old.saturating_add(r.old_len);
        new_base = r.new.saturating_add(r.new_len);
        k += 1;
    }
    let delta = last - new_base;
    if delta > usize::MAX - old_base {
        return None;
    }
    let end = old_base + delta;
    Some((end - (last - first), end))
}

#[cfg(test)]
mod tests {
    use super::Origin::{Addition as A, Context as C, Deletion as D};
    use super::*;
    use alloc::vec;

    fn region(old: usize, old_len: usize, new: usize, new_len: usize) -> Region {
        Region {
            old,
            old_len,
            new,
            new_len,
        }
    }

    fn of(old_start: usize, new_start: usize, body: &[Origin]) -> Vec<Region> {
        let mut out = Vec::new();
        runs(old_start, new_start, body, &mut out);
        out
    }

    #[test]
    fn runs_split_on_context_and_count_each_side() {
        // one hunk, two runs: a replacement and, after context, an insertion
        assert_eq!(
            of(0, 0, &[C, D, A, A, C, A]),
            [region(1, 1, 1, 2), region(3, 0, 4, 1)]
        );
    }

    #[test]
    fn a_pure_deletion_sits_before_the_next_new_line() {
        // `one two three four five`, the first two deleted
        assert_eq!(of(0, 0, &[D, D, C, C, C]), [region(0, 2, 0, 0)]);
        // in the middle, after context
        assert_eq!(of(0, 0, &[C, D, D, C]), [region(1, 2, 1, 0)]);
        // at the end: the gap sits after the last kept line
        assert_eq!(of(2, 2, &[C, D]), [region(3, 1, 3, 0)]);
    }

    #[test]
    fn context_only_bodies_have_no_regions() {
        assert_eq!(of(4, 7, &[C, C]), []);
        assert_eq!(of(4, 7, &[]), []);
    }

    #[test]
    fn runs_append() {
        let mut out = vec![region(0, 1, 0, 1)];
        runs(5, 5, &[A], &mut out);
        assert_eq!(out, [region(0, 1, 0, 1), region(5, 0, 5, 1)]);
        assert!(coherent(&out));
    }

    #[test]
    fn coherent_accepts_diffs_only() {
        assert!(coherent(&[]));
        assert!(coherent(&[region(1, 1, 1, 2), region(3, 0, 4, 1)]));
        // the same distance from the previous region on both sides
        assert!(!coherent(&[region(1, 1, 1, 2), region(3, 0, 5, 1)]));
        // overlapping the previous region
        assert!(!coherent(&[region(1, 2, 1, 1), region(2, 1, 2, 1)]));
        // empty on both sides
        assert!(!coherent(&[region(1, 0, 1, 0)]));
        // ending past usize
        assert!(!coherent(&[region(usize::MAX, 1, usize::MAX, 0)]));
    }

    #[test]
    fn span_orders_either_direction() {
        assert_eq!(span(3, 7), (3, 7));
        assert_eq!(span(7, 3), (3, 7));
        assert_eq!(span(4, 4), (4, 4));
    }

    #[test]
    fn no_regions_map_every_line_to_itself() {
        assert_eq!(old_line(9, &[]), Some(9));
        assert_eq!(new_line(9, &[]), Some(9));
        assert_eq!(old_span(2, 5, &[]), Some((2, 5)));
    }

    #[test]
    fn lines_around_an_insertion() {
        // two lines inserted before old line 2
        let rs = [region(2, 0, 2, 2)];
        assert_eq!(old_line(1, &rs), Some(1));
        assert_eq!(old_line(2, &rs), None);
        assert_eq!(old_line(3, &rs), None);
        assert_eq!(old_line(4, &rs), Some(2));
        assert_eq!(new_line(1, &rs), Some(1));
        assert_eq!(new_line(2, &rs), Some(4));
    }

    #[test]
    fn lines_around_a_deletion() {
        // old lines 2 and 3 deleted
        let rs = [region(2, 2, 2, 0)];
        assert_eq!(old_line(1, &rs), Some(1));
        assert_eq!(old_line(2, &rs), Some(4));
        assert_eq!(new_line(1, &rs), Some(1));
        assert_eq!(new_line(2, &rs), None);
        assert_eq!(new_line(3, &rs), None);
        assert_eq!(new_line(4, &rs), Some(2));
    }

    #[test]
    fn lines_around_a_replacement() {
        // old line 2 became new lines 2..5
        let rs = [region(2, 1, 2, 3)];
        assert_eq!(old_line(1, &rs), Some(1));
        assert_eq!(old_line(2, &rs), None);
        assert_eq!(old_line(4, &rs), None);
        assert_eq!(old_line(5, &rs), Some(3));
        assert_eq!(new_line(2, &rs), None);
        assert_eq!(new_line(3, &rs), Some(5));
    }

    #[test]
    fn old_span_answers_kept_consecutive_ranges_only() {
        let deletion = [region(4, 2, 4, 0)];
        // a deletion exactly at `first` and exactly after `last`
        assert_eq!(old_span(4, 6, &deletion), Some((6, 8)));
        assert_eq!(old_span(1, 3, &deletion), Some((1, 3)));
        // a deletion strictly inside the range
        assert_eq!(old_span(3, 4, &deletion), None);
        // an equal-length replacement inside the range
        let replaced = [region(4, 1, 4, 1)];
        assert_eq!(old_span(3, 5, &replaced), None);
        // an added line in the range
        let inserted = [region(4, 0, 4, 1)];
        assert_eq!(old_span(4, 4, &inserted), None);
        assert_eq!(old_span(5, 7, &inserted), Some((4, 6)));
        // an empty range
        assert_eq!(old_span(5, 4, &[]), None);
    }

    #[test]
    fn answers_past_usize_are_refused() {
        let rs = [region(0, 0, 0, 1)];
        assert_eq!(new_line(usize::MAX, &rs), None);
        assert_eq!(old_line(usize::MAX, &[region(0, 1, 0, 0)]), None);
    }
}
