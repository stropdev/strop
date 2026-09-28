//! Bounded geometry accumulation while the index worker is busy. Repeated
//! typing in one region stays one window rather than one retained job per key.
use super::{IndexError, MAX_JOURNAL_CHANGES};
use strop_core::id::BufferRevision;
use strop_core::{Change, InputEdit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChangedWindow {
    pub before: Span,
    pub after: Span,
}

#[derive(Debug, Clone, Copy)]
enum Continuity {
    Intact,
    Lost(IndexError),
}

#[derive(Debug, Clone)]
pub(crate) struct WordJournal {
    pub(super) before_revision: BufferRevision,
    pub(super) revision: BufferRevision,
    pub(super) before_bytes: usize,
    pub(super) bytes: usize,
    pub(super) windows: Vec<ChangedWindow>,
    continuity: Continuity,
}

impl WordJournal {
    pub fn new(revision: BufferRevision, bytes: usize) -> Self {
        Self {
            before_revision: revision,
            revision,
            before_bytes: bytes,
            bytes,
            windows: Vec::with_capacity(MAX_JOURNAL_CHANGES + 1),
            continuity: Continuity::Intact,
        }
    }

    /// Recycle the geometry allocation after transferring a snapshot to work.
    pub fn reset(&mut self, revision: BufferRevision, bytes: usize) {
        self.before_revision = revision;
        self.revision = revision;
        self.before_bytes = bytes;
        self.bytes = bytes;
        self.windows.clear();
        self.continuity = Continuity::Intact;
    }

    pub fn revision(&self) -> BufferRevision {
        self.revision
    }
    #[cfg(test)]
    pub fn retained_regions(&self) -> usize {
        self.windows.len()
    }

    pub fn check(&self) -> Result<(), IndexError> {
        match self.continuity {
            Continuity::Intact => Ok(()),
            Continuity::Lost(error) => Err(error),
        }
    }

    pub fn record(&mut self, changes: &[Change]) -> Result<(), IndexError> {
        self.check()?;
        let result = self.record_checked(changes);
        if let Err(error) = result {
            self.continuity = Continuity::Lost(error);
        }
        result
    }

    fn record_checked(&mut self, changes: &[Change]) -> Result<(), IndexError> {
        if changes.len() > MAX_JOURNAL_CHANGES {
            return Err(IndexError::JournalLimit);
        }
        for change in changes {
            if self.revision.get().checked_add(1) != Some(change.revision.get()) {
                return Err(IndexError::JournalGap);
            }
            self.record_geometry(change.edit)?;
            self.revision = change.revision;
        }
        Ok(())
    }

    fn record_geometry(&mut self, edit: InputEdit) -> Result<(), IndexError> {
        if edit.start_byte > edit.old_end_byte
            || edit.start_byte > edit.new_end_byte
            || edit.old_end_byte > self.bytes
        {
            return Err(IndexError::InvalidGeometry);
        }
        let bytes = self
            .bytes
            .checked_sub(edit.old_end_byte - edit.start_byte)
            .and_then(|size| size.checked_add(edit.new_end_byte - edit.start_byte))
            .ok_or(IndexError::InvalidGeometry)?;
        let before = Span {
            start: self.original_position(edit.start_byte, Edge::Start),
            end: self.original_position(edit.old_end_byte, Edge::End),
        };
        for window in &mut self.windows {
            window.after.start = map_edge(
                window.after.start,
                edit.start_byte,
                edit.old_end_byte,
                edit.new_end_byte,
                Edge::Start,
            );
            window.after.end = map_edge(
                window.after.end,
                edit.start_byte,
                edit.old_end_byte,
                edit.new_end_byte,
                Edge::End,
            );
        }
        self.windows.push(ChangedWindow {
            before,
            after: Span {
                start: edit.start_byte,
                end: edit.new_end_byte,
            },
        });
        self.windows
            .sort_unstable_by_key(|window| (window.before.start, window.after.start));
        let mut retained = 0usize;
        for index in 0..self.windows.len() {
            let window = self.windows[index];
            if retained > 0
                && (window.before.start <= self.windows[retained - 1].before.end
                    || window.after.start <= self.windows[retained - 1].after.end)
            {
                let previous = &mut self.windows[retained - 1];
                previous.before.end = previous.before.end.max(window.before.end);
                previous.after.end = previous.after.end.max(window.after.end);
            } else {
                self.windows[retained] = window;
                retained += 1;
            }
        }
        self.windows.truncate(retained);
        if retained > MAX_JOURNAL_CHANGES {
            // Leave no over-bound geometry retained; Lost prevents publication.
            self.windows.clear();
            return Err(IndexError::JournalLimit);
        }
        self.bytes = bytes;
        Ok(())
    }

    fn original_position(&self, position: usize, edge: Edge) -> usize {
        let mut previous_before = 0usize;
        let mut previous_after = 0usize;
        for window in &self.windows {
            if position < window.after.start
                || (position == window.after.start && matches!(edge, Edge::Start))
            {
                return previous_before + (position - previous_after);
            }
            if position <= window.after.end {
                return if position == window.after.end {
                    window.before.end
                } else {
                    match edge {
                        Edge::Start => window.before.start,
                        Edge::End => window.before.end,
                    }
                };
            }
            previous_before = window.before.end;
            previous_after = window.after.end;
        }
        previous_before + (position - previous_after)
    }
}

#[derive(Clone, Copy)]
enum Edge {
    Start,
    End,
}

fn map_edge(position: usize, start: usize, old_end: usize, new_end: usize, edge: Edge) -> usize {
    if position < start || (position == start && matches!(edge, Edge::Start)) {
        position
    } else if position >= old_end {
        new_end + (position - old_end)
    } else {
        match edge {
            Edge::Start => start,
            Edge::End => new_end,
        }
    }
}
