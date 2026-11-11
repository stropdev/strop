//! Excerpt geometry (0094): where an edit made in a collection's view lands
//! in the source the excerpt shows. `editor/collections/write.rs` asks this
//! exact function before any view byte changes, and again to place the
//! source edit.
//!
//! An excerpt shows the source bytes `source_start..source_end` as the view
//! bytes `view_start..view_end`. The view body is the source text plus, when
//! the source's last line has no newline, one terminator the view adds; so
//! the body is never shorter than the source span and at most one byte
//! longer.
//!
//! Proved (proofs/Kernel/Excerpt.lean):
//!
//! - `source_edit` admits exactly the edits that lie inside the body and
//!   leave its terminator in place, and refuses every other one: an edit can
//!   never reach the card borders, the gap rows or another file's excerpt;
//! - an admitted edit lands inside `source_start..source_end`, at the same
//!   offset it had in the body (clamped onto the source's end where the body
//!   has its added terminator);
//! - an anchor followed through an admitted edit in the view and then
//!   translated to the source is the translated anchor followed through the
//!   source edit (`editmap::map_position` on both sides): carets, marks and
//!   later excerpts agree in both buffers.

/// `value`, or `bound` when it lies past it.
fn clamp(value: usize, bound: usize) -> usize {
    if value > bound {
        bound
    } else {
        value
    }
}

/// Whether a view edit replacing `start..end` stays inside the body
/// `view_start..view_end`. `keeps_terminator` is the caller's reading of the
/// replacement text: after the edit the body still ends in a newline (or is
/// empty), so the row below it stays a row of its own.
pub fn admits(
    view_start: usize,
    view_end: usize,
    start: usize,
    end: usize,
    keeps_terminator: bool,
) -> bool {
    if start < view_start || start > end || end > view_end {
        return false;
    }
    end < view_end || keeps_terminator
}

/// Translate an admitted view edit replacing `start..end` into its source
/// range; refuse every other edit.
pub fn source_edit(
    view_start: usize,
    view_end: usize,
    source_start: usize,
    source_end: usize,
    start: usize,
    end: usize,
    keeps_terminator: bool,
) -> Option<(usize, usize)> {
    if source_start > source_end || !admits(view_start, view_end, start, end, keeps_terminator) {
        return None;
    }
    let shown = source_end - source_start;
    let from = clamp(start - view_start, shown);
    let to = clamp(end - view_start, shown);
    Some((source_start + from, source_start + to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_inside_the_body_keep_their_offset() {
        // body 10..30 shows source 100..120
        assert_eq!(
            source_edit(10, 30, 100, 120, 10, 10, false),
            Some((100, 100))
        );
        assert_eq!(
            source_edit(10, 30, 100, 120, 12, 17, false),
            Some((102, 107))
        );
        assert_eq!(
            source_edit(10, 30, 100, 120, 25, 30, true),
            Some((115, 120))
        );
    }

    #[test]
    fn edits_reaching_chrome_are_refused() {
        assert_eq!(source_edit(10, 30, 100, 120, 9, 12, true), None);
        assert_eq!(source_edit(10, 30, 100, 120, 28, 31, true), None);
        assert_eq!(source_edit(10, 30, 100, 120, 14, 12, true), None);
        // joining the last body line with the border below it
        assert_eq!(source_edit(10, 30, 100, 120, 29, 30, false), None);
        assert_eq!(source_edit(10, 30, 120, 100, 12, 14, true), None);
    }

    #[test]
    fn the_added_terminator_maps_onto_the_source_end() {
        // body 10..31 shows source 100..120 plus one added newline
        assert_eq!(
            source_edit(10, 31, 100, 120, 26, 31, true),
            Some((116, 120))
        );
        assert_eq!(
            source_edit(10, 31, 100, 120, 31, 31, true),
            Some((120, 120))
        );
    }
}
