//! Expand compact original/final dirty windows against their owning ropes.
//! Distant cursor edits remain disjoint; word expansion happens before merging.
use super::{scan, IndexError, WordJournal};
use ropey::Rope;
use std::ops::Range;
use strop_core::id::BufferRevision;

pub(super) struct DirtyWindows {
    pub before: Vec<Range<usize>>,
    pub after: Vec<Range<usize>>,
}

pub(super) fn changed(
    before: &Rope,
    before_revision: BufferRevision,
    after: &Rope,
    journal: &WordJournal,
) -> Result<DirtyWindows, IndexError> {
    journal.check()?;
    if before_revision != journal.before_revision {
        return Err(IndexError::JournalGap);
    }
    if before.len_bytes() != journal.before_bytes || after.len_bytes() != journal.bytes {
        return Err(IndexError::InvalidGeometry);
    }
    let mut original = Vec::with_capacity(journal.windows.len());
    let mut current = Vec::with_capacity(journal.windows.len());
    for window in &journal.windows {
        original.push(scan::expand(
            before,
            window.before.start..window.before.end,
        )?);
        current.push(scan::expand(after, window.after.start..window.after.end)?);
    }
    Ok(DirtyWindows {
        before: merge(original),
        after: merge(current),
    })
}

fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_unstable_by_key(|range| range.start);
    let mut retained = 0usize;
    for index in 0..ranges.len() {
        let range = ranges[index].clone();
        if retained > 0 && range.start <= ranges[retained - 1].end {
            ranges[retained - 1].end = ranges[retained - 1].end.max(range.end);
        } else {
            ranges[retained] = range;
            retained += 1;
        }
    }
    ranges.truncate(retained);
    ranges
}
