//! Edit-mapped syntax presentation (0066 §3): the syntax the engine has
//! already accepted from the display-analysis worker stays on screen
//! through edits, at the coordinates the mutation journal maps it to,
//! until the next exact result replaces it. Projection is pure journal
//! arithmetic — no parsing, no rope walks, no copies of unrelated
//! analysis payloads — and retained data is bounded by explicit caps
//! that reset to the freshly accepted window.
//!
//! Invariants (debug-asserted, relied on by every consumer):
//! - spans and coverage are sorted, disjoint and non-empty;
//! - every span lies inside coverage — a byte never analyzed must not
//!   borrow a class;
//! - rails are line-indexed, sorted and disjoint;
//! - `mapped_revision` names the coordinate space of all three.

use std::cell::RefCell;
use std::sync::Arc;

use strop_core::id::{BufferRevision, DisplayColumn};
use strop_syntax::{GuideFrame, Span};

use super::{AnalysisKey, FrameAnalysis};

/// Retention caps (0066 §3A): the presentation holds displayed content,
/// not a document-wide history. Crossing a cap drops offscreen data and
/// keeps the just-accepted window — a reset never changes what the
/// current frame shows, because the fresh result owns that window.
const MAX_SPANS: usize = 24_576;
const MAX_COVERAGE_INTERVALS: usize = 64;
const MAX_RAIL_LINES: usize = 4_096;

pub(crate) struct Presentation {
    spans: Vec<Span>,
    /// Sorted disjoint byte intervals the retained spans describe.
    coverage: Vec<(usize, usize)>,
    /// Per-line indent rails in mapped coordinates, already clipped to
    /// `rail_clip` display columns by the worker that produced them.
    rails: Vec<(usize, Vec<DisplayColumn>)>,
    rail_clip: (usize, usize),
    rail_tab: usize,
    /// The revision the retained data was last exact at; `None` once any
    /// journal edit has been folded through it (projected, not exact).
    exact_at: Option<BufferRevision>,
    mapped_revision: BufferRevision,
    /// Memoized interim frame. Presentation data is read from the
    /// readonly paint path, so the memo is interior-mutable; it keys on
    /// everything the frame depends on and any mutation clears it.
    interim: RefCell<Option<(MemoKey, Arc<FrameAnalysis>)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoKey {
    revision: u64,
    first: usize,
    last: usize,
    first_line: usize,
    last_line: usize,
    left: usize,
    right: usize,
    tab: usize,
}

impl Default for Presentation {
    fn default() -> Self {
        Self {
            spans: Vec::new(),
            coverage: Vec::new(),
            rails: Vec::new(),
            rail_clip: (0, 0),
            rail_tab: 0,
            exact_at: None,
            mapped_revision: BufferRevision::new(0),
            interim: RefCell::new(None),
        }
    }
}

impl Presentation {
    /// Publish an exact worker result: it owns its window outright —
    /// projected spans inside `[first, last)` are replaced, coverage
    /// grows by the window and rails are re-stated for its lines.
    pub(crate) fn accept(&mut self, key: &AnalysisKey, frame: &FrameAnalysis) {
        if self.spans.len().saturating_add(frame.spans.len()) > MAX_SPANS
            || self.coverage.len() >= MAX_COVERAGE_INTERVALS
        {
            self.spans.clear();
            self.coverage.clear();
        }
        if self.rails.len().saturating_add(frame.guides.rows.len()) > MAX_RAIL_LINES {
            self.rails.clear();
        }
        replace_window(&mut self.spans, key.first, key.last, &frame.spans);
        cover(&mut self.coverage, key.first, key.last);
        let first_line = frame.guides.first_line;
        let last_line = first_line + frame.guides.rows.len();
        replace_lines(
            &mut self.rails,
            first_line,
            last_line,
            frame.guides.rows.to_vec(),
        );
        self.rail_clip = (key.left, key.right);
        self.rail_tab = key.tab;
        self.exact_at = Some(key.revision);
        self.mapped_revision = key.revision;
        #[cfg(debug_assertions)]
        self.debug_check();
        *self.interim.borrow_mut() = None;
    }

    /// Fold one journal batch through the retained data. `publish_change`
    /// numbers each change exactly one past the previous revision, so a
    /// gap means an edit this presentation never saw — the projection is
    /// dropped rather than guessed at (0066 §3B).
    pub(crate) fn apply(&mut self, changes: &[strop_core::Change]) {
        for change in changes {
            if change.revision.get() != self.mapped_revision.get().saturating_add(1) {
                self.spans.clear();
                self.coverage.clear();
                self.rails.clear();
                self.exact_at = None;
                self.mapped_revision = change.revision;
                continue;
            }
            project_ranges(&mut self.spans, &change.edit);
            project_ranges(&mut self.coverage, &change.edit);
            project_rails(&mut self.rails, &change.edit);
            self.mapped_revision = change.revision;
            self.exact_at = None;
        }
        if !changes.is_empty() {
            #[cfg(debug_assertions)]
            self.debug_check();
            *self.interim.borrow_mut() = None;
        }
    }

    /// A real analysis failure invalidates the window's provisional
    /// coverage: failed coverage must not look settled (0066 §3C).
    pub(crate) fn fail(&mut self, key: &AnalysisKey) {
        uncover(&mut self.coverage, key.first, key.last);
        let coverage = &self.coverage;
        self.spans.retain(|span| {
            coverage
                .iter()
                .any(|&(a, b)| span.start >= a && span.end <= b)
        });
        self.rails.clear();
        #[cfg(debug_assertions)]
        self.debug_check();
        *self.interim.borrow_mut() = None;
    }

    pub(crate) fn mapped_revision(&self) -> BufferRevision {
        self.mapped_revision
    }

    /// The interim frame for a requested window: projected spans clipped
    /// to the window, plus retained rails when the clip and tab still
    /// match. `None` when nothing retained intersects — paint then runs
    /// the cold path exactly as before 0066. Search summaries and line
    /// layouts stay current-revision-only by contract (0066 §3C).
    pub(crate) fn interim_frame(
        &self,
        key: &AnalysisKey,
        first_line: usize,
        last_line: usize,
        len_bytes: usize,
    ) -> Option<Arc<FrameAnalysis>> {
        let last = key.last.min(len_bytes);
        let first = key.first.min(last);
        let memo = MemoKey {
            revision: self.mapped_revision.get(),
            first,
            last,
            first_line,
            last_line,
            left: key.left,
            right: key.right,
            tab: key.tab,
        };
        if let Some((seen, frame)) = self.interim.borrow().as_ref() {
            if *seen == memo {
                return Some(frame.clone());
            }
        }
        let mut spans = Vec::new();
        clip_into(&self.spans, first, last, &mut spans);
        let guides = if self.rail_clip == (key.left, key.right) && self.rail_tab == key.tab {
            GuideFrame {
                first_line,
                rows: (first_line..last_line)
                    .map(|line| self.rails_for(line).to_vec())
                    .collect(),
            }
        } else {
            GuideFrame::default()
        };
        if spans.is_empty() && guides.rows.iter().all(Vec::is_empty) {
            return None;
        }
        let frame = Arc::new(FrameAnalysis {
            spans,
            guides,
            search: None,
            layouts: Vec::new(),
        });
        *self.interim.borrow_mut() = Some((memo, frame.clone()));
        Some(frame)
    }

    fn rails_for(&self, line: usize) -> &[DisplayColumn] {
        self.rails
            .binary_search_by_key(&line, |(at, _)| *at)
            .map_or(&[], |index| self.rails[index].1.as_slice())
    }

    #[cfg(debug_assertions)]
    fn debug_check(&self) {
        let sorted = |ranges: &[(usize, usize)]| {
            ranges
                .windows(2)
                .all(|pair| pair[0].0 < pair[0].1 && pair[0].1 <= pair[1].0)
                && ranges.last().is_none_or(|&(a, b)| a < b)
        };
        debug_assert!(sorted(&self.coverage), "coverage {:?}", self.coverage);
        debug_assert!(
            self.rails.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "rails {:?}",
            self.rails
        );
        for span in &self.spans {
            debug_assert!(span.start < span.end, "empty span {span:?}");
            debug_assert!(
                self.coverage
                    .iter()
                    .any(|&(a, b)| span.start >= a && span.end <= b),
                "span {span:?} outside coverage {:?}",
                self.coverage
            );
        }
    }
}

/// The fragments of `[a, b)` that provably survive `[start, old_end)`
/// being replaced by `[start, new_end)`: the part strictly before the
/// edit keeps its coordinates, the part from `old_end` on shifts by the
/// delta, and the replaced interval itself carries nothing forward.
/// Order is preserved — the before-fragment precedes the after-fragment.
fn fragments(a: usize, b: usize, edit: &strop_core::InputEdit) -> [Option<(usize, usize)>; 2] {
    let before = (a < edit.start_byte).then(|| (a, b.min(edit.start_byte)));
    let after = (b > edit.old_end_byte).then(|| {
        let shift = |byte: usize| edit.new_end_byte + (byte - edit.old_end_byte);
        (shift(a.max(edit.old_end_byte)), shift(b))
    });
    [before, after]
}

/// A half-open byte range that can be re-stated at new coordinates.
trait ByteRange: Copy {
    fn bounds(&self) -> (usize, usize);
    fn with_bounds(&self, start: usize, end: usize) -> Self;
}

impl ByteRange for Span {
    fn bounds(&self) -> (usize, usize) {
        (self.start, self.end)
    }
    fn with_bounds(&self, start: usize, end: usize) -> Self {
        Span {
            start,
            end,
            ..*self
        }
    }
}

impl ByteRange for (usize, usize) {
    fn bounds(&self) -> (usize, usize) {
        *self
    }
    fn with_bounds(&self, start: usize, end: usize) -> Self {
        (start, end)
    }
}

/// Fold one edit through a sorted disjoint range list, mapping each
/// range's surviving fragments (0066 §3B). A multiline token keeps its
/// color up to the edit and resumes after it; the replaced bytes never
/// inherit a class.
fn project_ranges<T: ByteRange>(ranges: &mut Vec<T>, edit: &strop_core::InputEdit) {
    let mut out = Vec::with_capacity(ranges.len() + 4);
    for range in ranges.iter() {
        let (a, b) = range.bounds();
        for (start, end) in fragments(a, b, edit).into_iter().flatten() {
            out.push(range.with_bounds(start, end));
        }
    }
    *ranges = out;
}

/// Rails are line-indexed. A single-line edit cannot renumber any line,
/// so retained rails stay (the edited line's own rail is at worst one
/// frame stale — before 0066 it blanked). A multi-line edit drops rails
/// for the lines it consumed and shifts everything after them by the
/// line delta; the edit's first line keeps its rails (a split leaves
/// its indentation prefix intact).
fn project_rails(rails: &mut Vec<(usize, Vec<DisplayColumn>)>, edit: &strop_core::InputEdit) {
    let (start_row, old_end_row, new_end_row) = (
        edit.start_point.0,
        edit.old_end_point.0,
        edit.new_end_point.0,
    );
    if start_row == old_end_row && start_row == new_end_row {
        return;
    }
    let delta = new_end_row as isize - old_end_row as isize;
    let mut out = Vec::with_capacity(rails.len());
    for (line, columns) in rails.drain(..) {
        if line <= start_row {
            out.push((line, columns));
        } else if line > old_end_row {
            out.push(((line as isize + delta).max(0) as usize, columns));
        }
    }
    *rails = out;
}

/// Replace the spans inside `[first, last)` with `fresh` (sorted,
/// disjoint, within the window). A retained span straddling a boundary
/// keeps its outside fragment, so scrolling back never loses the edge
/// of a previously accepted region.
fn replace_window(spans: &mut Vec<Span>, first: usize, last: usize, fresh: &[Span]) {
    let mut kept: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans.iter() {
        let (a, b) = (span.start, span.end);
        if b <= first || a >= last {
            kept.push(*span);
        } else {
            if a < first {
                kept.push(Span {
                    end: first,
                    ..*span
                });
            }
            if b > last {
                kept.push(Span {
                    start: last,
                    ..*span
                });
            }
        }
    }
    let mut out = Vec::with_capacity(kept.len() + fresh.len());
    let (mut index, mut at) = (0usize, 0usize);
    while index < kept.len() && at < fresh.len() {
        if fresh[at].start < kept[index].start {
            out.push(fresh[at]);
            at += 1;
        } else {
            out.push(kept[index]);
            index += 1;
        }
    }
    out.extend_from_slice(&fresh[at..]);
    out.extend_from_slice(&kept[index..]);
    *spans = out;
}

/// Insert `[first, last)` into the sorted disjoint coverage, coalescing
/// touching or overlapping intervals (sequential scrolling keeps the
/// list small — one growing interval, not one per frame).
fn cover(coverage: &mut Vec<(usize, usize)>, first: usize, last: usize) {
    if first >= last {
        return;
    }
    let mut merged = Vec::with_capacity(coverage.len() + 1);
    let mut pending = (first, last);
    let mut flushed = false;
    for &(a, b) in coverage.iter() {
        if b < pending.0 {
            merged.push((a, b));
        } else if a > pending.1 {
            if !flushed {
                merged.push(pending);
                flushed = true;
            }
            merged.push((a, b));
        } else {
            pending = (pending.0.min(a), pending.1.max(b));
        }
    }
    if !flushed {
        merged.push(pending);
    }
    *coverage = merged;
}

/// Remove `[first, last)` from the coverage, truncating straddling
/// intervals to their outside parts.
fn uncover(coverage: &mut Vec<(usize, usize)>, first: usize, last: usize) {
    let mut out = Vec::with_capacity(coverage.len() + 2);
    for &(a, b) in coverage.iter() {
        if a < first {
            out.push((a, b.min(first)));
        }
        if b > last {
            out.push((last.max(a), b));
        }
    }
    out.retain(|&(a, b)| a < b);
    *coverage = out;
}

/// Replace the retained rails for lines `[first, last)` with `fresh`
/// (one entry per line, possibly empty — an empty row is exact "no
/// rails" data, not a hole).
fn replace_lines(
    rails: &mut Vec<(usize, Vec<DisplayColumn>)>,
    first_line: usize,
    last_line: usize,
    fresh: Vec<Vec<DisplayColumn>>,
) {
    let mut out = Vec::with_capacity(rails.len() + fresh.len());
    let mut index = 0usize;
    let flush =
        |out: &mut Vec<_>, index: &mut usize, fresh: &Vec<Vec<DisplayColumn>>, upto: usize| {
            while *index < fresh.len() && first_line + *index < upto {
                out.push((first_line + *index, fresh[*index].clone()));
                *index += 1;
            }
        };
    for (line, columns) in rails.drain(..) {
        if line < first_line || line >= last_line {
            flush(&mut out, &mut index, &fresh, line);
            out.push((line, columns));
        }
    }
    flush(&mut out, &mut index, &fresh, usize::MAX);
    *rails = out;
}

/// Copy the spans intersecting `[first, last)` into `out`, truncated to
/// the window — the interim frame serves exactly what it can vouch for.
fn clip_into(spans: &[Span], first: usize, last: usize, out: &mut Vec<Span>) {
    let mut index = spans.partition_point(|span| span.end <= first);
    while let Some(span) = spans.get(index) {
        if span.start >= last {
            break;
        }
        out.push(Span {
            start: span.start.max(first),
            end: span.end.min(last),
            ..*span
        });
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::id::{Arena, DocumentKind};
    use strop_syntax::{Class, Emphasis};

    fn doc_id() -> strop_core::id::DocumentId {
        Arena::<DocumentKind, u8>::default()
            .try_insert(0)
            .expect("arena id")
    }

    fn key(first: usize, last: usize) -> AnalysisKey {
        AnalysisKey {
            target: super::super::AnalysisTarget::Document(doc_id()),
            revision: BufferRevision::new(0),
            first,
            last,
            tab: 4,
            guides: false,
            left: 0,
            right: 80,
            syntax_path: None,
            search: None,
        }
    }

    fn span(a: usize, b: usize) -> Span {
        Span {
            start: a,
            end: b,
            class: Class::Keyword,
            emphasis: Emphasis::default(),
        }
    }

    fn edit(start: usize, old_end: usize, new_end: usize) -> strop_core::InputEdit {
        strop_core::InputEdit {
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: new_end,
            start_point: (0, start),
            old_end_point: (0, old_end),
            new_end_point: (0, new_end),
        }
    }

    fn change(revision: u64, edit: strop_core::InputEdit) -> strop_core::Change {
        strop_core::Change {
            revision: BufferRevision::new(revision),
            origin: strop_core::ChangeOrigin::User,
            edit,
            history: None,
        }
    }

    fn presentation(spans: &[(usize, usize)]) -> Presentation {
        Presentation {
            spans: spans.iter().map(|&(a, b)| span(a, b)).collect(),
            coverage: vec![(0, 10_000)],
            mapped_revision: BufferRevision::new(0),
            ..Default::default()
        }
    }

    fn projected(spans: &[(usize, usize)], edit: strop_core::InputEdit) -> Vec<(usize, usize)> {
        let mut p = presentation(spans);
        p.apply(&[change(1, edit)]);
        p.spans.iter().map(|s| (s.start, s.end)).collect()
    }

    #[test]
    fn insert_before_a_span_shifts_it() {
        assert_eq!(projected(&[(5, 9)], edit(2, 2, 3)), vec![(6, 10)]);
    }

    #[test]
    fn delete_before_a_span_shifts_it_left() {
        assert_eq!(projected(&[(5, 9)], edit(2, 4, 2)), vec![(3, 7)]);
    }

    #[test]
    fn replacement_excludes_the_replaced_interval() {
        // a token straddling the edit survives as two fragments; the new
        // bytes in between carry no previous class (0066 §3B)
        assert_eq!(projected(&[(2, 12)], edit(5, 7, 9)), vec![(2, 5), (9, 14)]);
    }

    #[test]
    fn a_span_inside_the_replaced_region_drops() {
        assert_eq!(projected(&[(5, 7), (8, 11)], edit(4, 9, 6)), vec![(6, 8)]);
    }

    #[test]
    fn insertion_at_the_start_boundary_shifts_but_never_grows() {
        // text starting at the insertion point moves right; a span
        // ending there must not absorb the inserted bytes
        assert_eq!(
            projected(&[(4, 8), (8, 12)], edit(8, 8, 11)),
            vec![(4, 8), (11, 15)]
        );
    }

    #[test]
    fn insertion_inside_a_span_leaves_the_new_interval_plain() {
        assert_eq!(projected(&[(2, 9)], edit(4, 4, 6)), vec![(2, 4), (6, 11)]);
    }

    #[test]
    fn pure_insertion_before_everything_maps_the_whole_list() {
        // the 0066 field report: one space at byte zero shifts all spans
        assert_eq!(
            projected(&[(0, 2), (3, 7), (20, 30)], edit(0, 0, 1)),
            vec![(1, 3), (4, 8), (21, 31)]
        );
    }

    #[test]
    fn multibyte_offsets_are_bytes_and_stay_ordered() {
        // 界 occupies three bytes; an edit after it is plain arithmetic
        assert_eq!(
            projected(&[(0, 3), (4, 8)], edit(3, 3, 6)),
            vec![(0, 3), (7, 11)]
        );
    }

    #[test]
    fn a_revision_gap_drops_the_projection_instead_of_guessing() {
        let mut p = presentation(&[(1, 4)]);
        p.apply(&[change(7, edit(0, 0, 1))]);
        assert!(p.spans.is_empty() && p.coverage.is_empty() && p.exact_at.is_none());
        // numbering continues: the next contiguous change maps cleanly
        p.spans = vec![span(2, 5)];
        p.coverage = vec![(0, 100)];
        p.apply(&[change(8, edit(9, 9, 11))]);
        assert_eq!(
            p.spans.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(),
            vec![(2, 5)]
        );
    }

    #[test]
    fn an_edit_marks_the_data_projected_and_an_accept_exact() {
        let mut p = presentation(&[(1, 4)]);
        assert!(p.exact_at.is_none(), "seeded by hand, never accepted");
        p.accept(
            &key(0, 100),
            &FrameAnalysis {
                spans: vec![span(1, 4)],
                ..Default::default()
            },
        );
        assert_eq!(p.exact_at, Some(BufferRevision::new(0)));
        p.apply(&[change(1, edit(50, 50, 51))]);
        assert_eq!(p.exact_at, None);
    }

    #[test]
    fn coverage_projects_and_never_covers_replaced_bytes() {
        let mut p = Presentation {
            coverage: vec![(0, 100)],
            ..Default::default()
        };
        p.apply(&[change(1, edit(10, 30, 15))]);
        assert_eq!(p.coverage, vec![(0, 10), (15, 85)]);
    }

    #[test]
    fn accept_replaces_only_its_window_and_extends_coverage() {
        let mut p = presentation(&[(1, 3), (5, 8), (12, 20)]);
        p.accept(
            &key(5, 15),
            &FrameAnalysis {
                spans: vec![span(6, 7), span(9, 11)],
                ..Default::default()
            },
        );
        assert_eq!(
            p.spans.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(),
            vec![(1, 3), (6, 7), (9, 11), (15, 20)]
        );
        assert_eq!(p.coverage, vec![(0, 10_000)]);
    }

    #[test]
    fn accept_coalesces_touching_coverage_from_scrolling() {
        let mut p = Presentation::default();
        let empty = FrameAnalysis::default();
        p.accept(&key(0, 100), &empty);
        p.accept(&key(100, 200), &empty);
        p.accept(&key(300, 400), &empty);
        assert_eq!(p.coverage, vec![(0, 200), (300, 400)]);
    }

    #[test]
    fn caps_reset_to_the_fresh_window_only() {
        let mut p = Presentation::default();
        for window in 0..MAX_COVERAGE_INTERVALS {
            p.accept(
                &key(window * 10, window * 10 + 5),
                &FrameAnalysis::default(),
            );
        }
        assert_eq!(p.coverage.len(), MAX_COVERAGE_INTERVALS);
        p.accept(&key(1_000, 1_010), &FrameAnalysis::default());
        assert_eq!(p.coverage, vec![(1_000, 1_010)]);
    }

    #[test]
    fn fail_uncovers_its_window_and_what_it_disgorges() {
        let mut p = presentation(&[(1, 3), (5, 8)]);
        p.fail(&key(4, 6));
        assert_eq!(p.coverage, vec![(0, 4), (6, 10_000)]);
        assert_eq!(
            p.spans.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(),
            vec![(1, 3)]
        );
    }

    #[test]
    fn single_line_edits_keep_rails_and_multiline_edits_shift_them() {
        let rail = |line: usize| (line, vec![DisplayColumn::new(4)]);
        let mut p = Presentation {
            rails: vec![rail(2), rail(3), rail(9), rail(10)],
            ..Default::default()
        };
        // same-line edit: nothing renumbers
        p.apply(&[change(1, edit(0, 0, 1))]);
        assert_eq!(p.rails.len(), 4);
        // a two-line replacement at lines 3..5: line 3 keeps (edit's
        // first line), consumed lines drop, later lines shift by delta 1
        let multiline = strop_core::InputEdit {
            start_byte: 30,
            old_end_byte: 60,
            new_end_byte: 45,
            start_point: (3, 0),
            old_end_point: (5, 0),
            new_end_point: (6, 0),
        };
        p.apply(&[change(2, multiline)]);
        let lines: Vec<usize> = p.rails.iter().map(|(line, _)| *line).collect();
        assert_eq!(lines, vec![2, 3, 10, 11]);
    }

    #[test]
    fn interim_frame_clips_to_the_window_and_memos() {
        let mut p = presentation(&[(1, 3), (5, 8), (12, 20)]);
        let request = key(4, 13);
        let frame = p.interim_frame(&request, 0, 4, 10_000).unwrap();
        assert_eq!(
            frame
                .spans
                .iter()
                .map(|s| (s.start, s.end))
                .collect::<Vec<_>>(),
            vec![(5, 8), (12, 13)]
        );
        assert!(frame.search.is_none() && frame.layouts.is_empty());
        // the memo returns the same Arc for the same request
        let again = p.interim_frame(&request, 0, 4, 10_000).unwrap();
        assert!(Arc::ptr_eq(&frame, &again));
        // a projection invalidates it; the request still serves
        p.apply(&[change(1, edit(100, 100, 101))]);
        let rebuilt = p.interim_frame(&request, 0, 4, 10_000).unwrap();
        assert!(!Arc::ptr_eq(&frame, &rebuilt));
    }

    #[test]
    fn interim_rails_follow_the_retained_clip_and_tab() {
        let mut p = Presentation {
            rails: vec![(1, vec![DisplayColumn::new(4)])],
            rail_clip: (0, 80),
            rail_tab: 4,
            ..Default::default()
        };
        let request = key(0, 100);
        let frame = p.interim_frame(&request, 0, 3, 10_000).unwrap();
        assert_eq!(frame.guides.columns(1), &[DisplayColumn::new(4)]);
        // a different clip or tab cannot reuse the retained columns
        p.spans = vec![span(0, 5)];
        p.coverage = vec![(0, 100)];
        let mut scrolled = request.clone();
        scrolled.left = 8;
        scrolled.right = 88;
        let frame = p.interim_frame(&scrolled, 0, 3, 10_000).unwrap();
        assert!(frame.guides.rows.iter().all(|row| row.is_empty()));
    }

    #[test]
    fn interim_frame_is_none_when_nothing_is_retained() {
        let p = Presentation::default();
        assert!(p.interim_frame(&key(0, 100), 0, 4, 10_000).is_none());
    }
}
