//! Window layout along one axis (plans/0104 §4.2): how one split's cells are
//! divided among its children. `strop-engine`'s window tree
//! (`editor/panes/layout.rs`) asks this once per split node; the tree itself
//! stays in the engine.
//!
//! A child holding `tracks` windows side by side needs `unit` cells per
//! window and `gap` cells between neighbours (a divider column side by side;
//! nothing when stacked, where the lower title is the boundary). Every child
//! first gets that minimum; the spare cells are then shared by window count
//! (vim's `win_equal_rec`), rounding down, and the last child takes what is
//! left. When the minimum does not fit, earlier children keep theirs and later
//! ones shrink to nothing.
//!
//! Proved (proofs/Kernel/Layout.lean):
//!
//! - no span ever passes the frame, and the spans are ordered and disjoint,
//!   at any size: panes never overlap and nothing reaches the modeline;
//! - when the minimum fits, the spans and the gaps between them cover the
//!   frame exactly: a closed child's cells go back to the others;
//! - when the minimum fits, every child gets at least its minimum: a column
//!   per window side by side, a title and a row per window stacked.

/// One child's cells along its parent's axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub at: usize,
    pub len: usize,
}

/// What the owner fills `out` with before `tile`.
pub fn empty() -> Span {
    Span { at: 0, len: 0 }
}

fn least(a: usize, b: usize) -> usize {
    if a < b {
        a
    } else {
        b
    }
}

/// The fewest cells `tracks` windows side by side need: `unit` each and
/// `gap` between neighbours.
pub fn minimum(tracks: usize, unit: usize, gap: usize) -> usize {
    if tracks == 0 {
        0
    } else {
        tracks * unit + (tracks - 1) * gap
    }
}

/// The windows across all children.
pub fn total(tracks: &[usize]) -> usize {
    let mut sum = 0;
    let mut k = 0;
    while k < tracks.len() {
        sum += tracks[k];
        k += 1;
    }
    sum
}

/// A child of `tracks` windows takes this much of `room` spare cells while
/// `weight` windows are still to be served (itself included). Rounds down:
/// the odd cells go to later children.
pub fn share(room: usize, tracks: usize, weight: usize) -> usize {
    (tracks * room).checked_div(weight).unwrap_or_default()
}

/// Divide `cells` among children; child `k` holds `tracks[k]` windows side by
/// side and its span is written to `out[k]`. Children past `out.len()` are
/// not placed.
pub fn tile(cells: usize, unit: usize, gap: usize, tracks: &[usize], out: &mut [Span]) {
    let mut weight = total(tracks);
    let need = minimum(weight, unit, gap);
    let mut room = if cells > need { cells - need } else { 0 };
    let mut at = 0;
    let mut k = 0;
    while k < tracks.len() && k < out.len() {
        let left = cells - at;
        let len = if k + 1 == tracks.len() {
            left
        } else {
            let extra = share(room, tracks[k], weight);
            room -= extra;
            weight -= tracks[k];
            least(minimum(tracks[k], unit, gap) + extra, left)
        };
        out[k] = Span { at, len };
        at = least(at + len + gap, cells);
        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(cells: usize, unit: usize, gap: usize, tracks: &[usize]) -> [Span; 8] {
        let mut out = [empty(); 8];
        tile(cells, unit, gap, tracks, &mut out);
        out
    }

    fn lens(cells: usize, unit: usize, gap: usize, tracks: &[usize]) -> [usize; 8] {
        let out = spans(cells, unit, gap, tracks);
        let mut lens = [0; 8];
        let mut k = 0;
        while k < 8 {
            lens[k] = out[k].len;
            k += 1;
        }
        lens
    }

    /// 0104 §5.2: the layouts strop could already show keep their cells.
    #[test]
    fn flat_splits_keep_their_sizes() {
        assert_eq!(&lens(80, 1, 1, &[1, 1])[..2], &[39, 40]);
        assert_eq!(&lens(23, 2, 0, &[1, 1])[..2], &[11, 12]);
        assert_eq!(&lens(80, 1, 1, &[1, 1, 1])[..3], &[26, 26, 26]);
        // the remainder is spread, not all given to the last
        assert_eq!(&lens(80, 1, 0, &[1, 1, 1])[..3], &[26, 27, 27]);
    }

    /// 0104 §5.3 and §6 (e): a child's share is by windows across.
    #[test]
    fn a_nested_group_gets_its_share_by_window_count() {
        let out = spans(80, 1, 1, &[1, 2]);
        assert_eq!((out[0].at, out[0].len), (0, 26));
        assert_eq!((out[1].at, out[1].len), (27, 53));
    }

    /// The spans stay inside the frame and in order at every size; when the
    /// minimum fits they and the gaps cover it exactly.
    #[test]
    fn spans_are_in_bounds_and_exact_when_they_fit() {
        let shapes: [&[usize]; 6] = [&[1], &[1, 1], &[2, 1], &[1, 3, 1], &[2, 2, 2, 1], &[4]];
        for tracks in shapes {
            for (unit, gap) in [(1, 1), (2, 0)] {
                for cells in 0..=200 {
                    let out = spans(cells, unit, gap, tracks);
                    let n = tracks.len();
                    let mut end = 0;
                    for (k, span) in out.iter().take(n).enumerate() {
                        assert!(span.at + span.len <= cells, "{cells} {tracks:?}");
                        assert!(k == 0 || span.at >= end, "{cells} {tracks:?}");
                        end = span.at + span.len;
                    }
                    if minimum(total(tracks), unit, gap) <= cells {
                        assert_eq!(out[0].at, 0);
                        for k in 0..n {
                            assert!(out[k].len >= minimum(tracks[k], unit, gap));
                            if k + 1 < n {
                                assert_eq!(out[k + 1].at, out[k].at + out[k].len + gap);
                            }
                        }
                        assert_eq!(out[n - 1].at + out[n - 1].len, cells);
                    }
                }
            }
        }
    }
}
