//! The only mutable text capability. Every mutation publishes pre-edit geometry.
use super::Buffer;
use crate::history::{Edit, EditKind};
use crate::id::{BufferRevision, ByteOffset};
use crate::{InputEdit, Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ChangeOrigin {
    User,
    History,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub revision: BufferRevision,
    pub origin: ChangeOrigin,
    pub edit: InputEdit,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("readonly buffer")]
    ReadOnly,
    #[error("invalid edit range")]
    InvalidRange,
    #[error("overlapping replacements in pre-edit coordinates")]
    Overlap,
    #[error("prepared replacement belongs to another buffer")]
    WrongBuffer,
    #[error("stale revision: expected {expected}, found {found}")]
    StaleRevision {
        expected: BufferRevision,
        found: BufferRevision,
    },
    #[error("buffer revision exhausted")]
    RevisionExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub range: Range,
    pub text: String,
}
impl Replacement {
    pub fn new(range: Range, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }
}

/// Validated, sorted replacements bound to one exact buffer incarnation/revision.
/// The editor may restore prompt selection state before acquiring its write lease.
pub struct PreparedReplacements {
    buffer: crate::diagnostics::BufferTraceId,
    revision: BufferRevision,
    edits: Vec<Replacement>,
}
impl PreparedReplacements {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
    /// The exact validated order shared by preview and application.
    pub fn edits(&self) -> &[Replacement] {
        &self.edits
    }
}

pub struct UserEdit<'a> {
    buffer: &'a mut Buffer,
}
pub struct SystemEdit<'a> {
    buffer: &'a mut Buffer,
}

impl Buffer {
    pub fn edit(&mut self) -> UserEdit<'_> {
        UserEdit { buffer: self }
    }
    pub fn system_edit(&mut self) -> SystemEdit<'_> {
        SystemEdit { buffer: self }
    }
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }
    pub fn clear_changes(&mut self) {
        self.changes.clear();
    }
    pub fn begin_undo_group(&mut self) {
        self.history.begin();
    }
    pub fn commit_undo_group(&mut self) {
        self.history.commit();
    }

    fn validate_range(&self, range: Range) -> Result<(), EditError> {
        if range.start > range.end || !self.is_boundary(range.start) || !self.is_boundary(range.end)
        {
            return Err(EditError::InvalidRange);
        }
        Ok(())
    }

    /// All replacements refer to one pre-edit snapshot. Validate the whole batch
    /// before opening history or touching text; reversed application preserves coordinates.
    pub fn prepare_replacements(
        &self,
        base: BufferRevision,
        mut replacements: Vec<Replacement>,
    ) -> Result<PreparedReplacements, EditError> {
        if self.readonly {
            return Err(EditError::ReadOnly);
        }
        if base != self.revision() {
            return Err(EditError::StaleRevision {
                expected: base,
                found: self.revision(),
            });
        }
        replacements.retain(|edit| !edit.range.is_empty() || !edit.text.is_empty());
        for edit in &replacements {
            self.validate_range(edit.range)?;
        }
        replacements.sort_unstable_by_key(|edit| (edit.range.start.get(), edit.range.end.get()));
        // The verified geometry kernel (0045, crate::editmap): sorted by
        // start, strictly non-overlapping, in-bounds — proven, and the
        // error here can only be an overlap (bounds checked above).
        let pairs: Vec<(usize, usize)> = replacements
            .iter()
            .map(|edit| (edit.range.start.get(), edit.range.end.get()))
            .collect();
        let sorted = crate::editmap::check_batch(self.len_bytes(), pairs)
            .map_err(|()| EditError::Overlap)?;
        // Keep the payloads beside their sorted geometry. Searching/removing
        // each payload by range made large reviewed batches quadratic.
        debug_assert!(replacements
            .iter()
            .zip(sorted)
            .all(|(edit, pair)| (edit.range.start.get(), edit.range.end.get()) == pair));
        self.epoch
            .checked_add(replacements.len() as u64)
            .ok_or(EditError::RevisionExhausted)?;
        Ok(PreparedReplacements {
            buffer: self.trace_id(),
            revision: base,
            edits: replacements,
        })
    }

    pub fn apply_prepared(
        &mut self,
        prepared: PreparedReplacements,
        undo_open: bool,
    ) -> Result<BufferRevision, EditError> {
        if prepared.buffer != self.trace_id() {
            return Err(EditError::WrongBuffer);
        }
        if self.readonly {
            return Err(EditError::ReadOnly);
        }
        if prepared.revision != self.revision() {
            return Err(EditError::StaleRevision {
                expected: prepared.revision,
                found: self.revision(),
            });
        }
        if prepared.is_empty() {
            return Ok(self.revision());
        }
        self.history.begin();
        for replacement in prepared.edits.into_iter().rev() {
            self.replace_validated(replacement.range, &replacement.text, ChangeOrigin::User);
        }
        if !undo_open {
            self.history.commit();
        }
        Ok(self.revision())
    }

    fn replace_validated(&mut self, range: Range, text: &str, origin: ChangeOrigin) {
        debug_assert!(self.validate_range(range).is_ok());
        debug_assert!(self.epoch < u64::MAX);
        let (start_byte, end_byte) = (range.start.get(), range.end.get());
        let start_point = self.point_of(start_byte);
        let old_end_point = self.point_of(end_byte);
        let extent = Self::point_extent(text);
        let new_end_point = if extent.0 == 0 {
            (start_point.0, start_point.1 + extent.1)
        } else {
            (start_point.0 + extent.0, extent.1)
        };
        let edit = InputEdit {
            start_byte,
            old_end_byte: end_byte,
            new_end_byte: start_byte + text.len(),
            start_point,
            old_end_point,
            new_end_point,
        };
        if origin == ChangeOrigin::User {
            if !range.is_empty() {
                let removed = self.rope.byte_slice(start_byte..end_byte).to_string();
                self.history.record(
                    Edit {
                        at: start_byte,
                        text: removed.clone(),
                        kind: EditKind::Insert,
                    },
                    Edit {
                        at: start_byte,
                        text: removed,
                        kind: EditKind::Delete,
                    },
                );
            }
            if !text.is_empty() {
                self.history.record(
                    Edit {
                        at: start_byte,
                        text: text.into(),
                        kind: EditKind::Delete,
                    },
                    Edit {
                        at: start_byte,
                        text: text.into(),
                        kind: EditKind::Insert,
                    },
                );
            }
        }
        let start = self.rope.byte_to_char(start_byte);
        let end = self.rope.byte_to_char(end_byte);
        if start != end {
            self.rope.remove(start..end);
        }
        if !text.is_empty() {
            self.rope.insert(start, text);
        }
        self.publish_change(edit, origin);
        self.trace_edit(origin, start_byte, range.len(), text);
    }

    fn publish_change(&mut self, edit: InputEdit, origin: ChangeOrigin) {
        debug_assert!(self.epoch < u64::MAX);
        self.epoch += 1;
        self.invalidate_line_layouts(&edit);
        self.dirty |= origin != ChangeOrigin::System;
        self.changes.push(Change {
            revision: self.revision(),
            origin,
            edit,
        });
    }
}

impl UserEdit<'_> {
    pub fn insert(&mut self, at: impl Into<ByteOffset>, text: &str) -> Result<(), EditError> {
        let at = at.into();
        self.replace(Range::charwise(at, at), text)
    }
    pub fn delete(&mut self, range: Range) -> Result<String, EditError> {
        if self.buffer.readonly {
            return Err(EditError::ReadOnly);
        }
        self.buffer.validate_range(range)?;
        let removed = self.buffer.slice_string(range);
        self.replace(range, "")?;
        Ok(removed)
    }
    pub fn replace(&mut self, range: Range, text: &str) -> Result<(), EditError> {
        if self.buffer.readonly {
            return Err(EditError::ReadOnly);
        }
        self.buffer.validate_range(range)?;
        if range.is_empty() && text.is_empty() {
            return Ok(());
        }
        self.buffer
            .epoch
            .checked_add(1)
            .ok_or(EditError::RevisionExhausted)?;
        self.buffer
            .replace_validated(range, text, ChangeOrigin::User);
        Ok(())
    }
}

impl SystemEdit<'_> {
    /// A partial generated-surface edit (0049 §5 collection view
    /// splices): system origin, never undoable, and it never dirties the
    /// buffer.
    pub fn replace(&mut self, range: Range, text: &str) -> Result<(), EditError> {
        self.buffer.validate_range(range)?;
        if range.is_empty() && text.is_empty() {
            return Ok(());
        }
        self.buffer
            .epoch
            .checked_add(1)
            .ok_or(EditError::RevisionExhausted)?;
        self.buffer
            .replace_validated(range, text, ChangeOrigin::System);
        Ok(())
    }

    /// A generated surface is a new system snapshot, never an undoable user edit.
    pub fn replace_all(&mut self, text: &str) -> Result<(), EditError> {
        self.buffer
            .epoch
            .checked_add(1)
            .ok_or(EditError::RevisionExhausted)?;
        self.buffer.replace_validated(
            Range::charwise(0, self.buffer.len_bytes()),
            text,
            ChangeOrigin::System,
        );
        self.buffer.history = Default::default();
        Ok(())
    }

    /// Publish worker-built text without flattening or rebuilding a rope on the
    /// event loop. Identity, readonly policy and monotonic revision stay local.
    pub fn replace_rope(&mut self, rope: ropey::Rope) -> Result<(), EditError> {
        self.buffer
            .epoch
            .checked_add(1)
            .ok_or(EditError::RevisionExhausted)?;
        let old_end_byte = self.buffer.len_bytes();
        let old_end_point = self.buffer.point_of(old_end_byte);
        let new_end_byte = rope.len_bytes();
        let line = rope.byte_to_line(new_end_byte);
        let new_end_point = (line, new_end_byte - rope.line_to_byte(line));
        self.buffer.rope = rope;
        self.buffer.publish_change(
            InputEdit {
                start_byte: 0,
                old_end_byte,
                new_end_byte,
                start_point: (0, 0),
                old_end_point,
                new_end_point,
            },
            ChangeOrigin::System,
        );
        self.buffer.history = Default::default();
        self.buffer.trace_snapshot(old_end_byte);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HistoryMove {
    pub start: ByteOffset,
    pub end: ByteOffset,
}

impl Buffer {
    pub fn undo(&mut self) -> Result<Option<HistoryMove>, EditError> {
        self.move_history(crate::history::HistoryAction::Undo)
    }
    pub fn redo(&mut self) -> Result<Option<HistoryMove>, EditError> {
        self.move_history(crate::history::HistoryAction::Redo)
    }
    pub fn restore_revision(&mut self, revision: usize) -> Result<Option<HistoryMove>, EditError> {
        self.move_history(crate::history::HistoryAction::Jump(revision))
    }
    /// Preflight a grouped undo without changing history or buffer contents.
    pub fn check_undo(&self) -> Result<bool, EditError> {
        self.check_history_move(crate::history::HistoryAction::Undo)
            .map(|cost| cost.is_some())
    }

    /// Preflight a grouped history restore, including authority and revision capacity.
    pub fn check_restore_revision(&self, revision: usize) -> Result<bool, EditError> {
        self.check_history_move(crate::history::HistoryAction::Jump(revision))
            .map(|cost| cost.is_some())
    }

    fn check_history_move(
        &self,
        action: crate::history::HistoryAction,
    ) -> Result<Option<usize>, EditError> {
        if self.readonly {
            return Err(EditError::ReadOnly);
        }
        let cost = self.history.movement_cost(action);
        if let Some(cost) = cost {
            self.epoch
                .checked_add(cost as u64)
                .ok_or(EditError::RevisionExhausted)?;
        }
        Ok(cost)
    }
    fn move_history(
        &mut self,
        action: crate::history::HistoryAction,
    ) -> Result<Option<HistoryMove>, EditError> {
        let Some(cost) = self.check_history_move(action)? else {
            return Ok(None);
        };
        // A valid history is guaranteed by construction and validated at restore.
        let Some(ops) = self.history.navigate(action) else {
            return Ok(None);
        };
        debug_assert_eq!(cost, ops.len());
        let start = ops.iter().map(|edit| edit.at).min().unwrap_or(0);
        let end = ops
            .last()
            .map(|edit| {
                edit.at
                    + if edit.kind == EditKind::Insert {
                        edit.text.len()
                    } else {
                        0
                    }
            })
            .unwrap_or(start);
        for op in &ops {
            let (range, text) = match op.kind {
                EditKind::Insert => (Range::charwise(op.at, op.at), op.text.as_str()),
                EditKind::Delete => (Range::charwise(op.at, op.at + op.text.len()), ""),
            };
            debug_assert!(self.validate_range(range).is_ok(), "sealed history range");
            debug_assert!(
                op.kind != EditKind::Delete
                    || self.rope.byte_slice(range.start.get()..range.end.get()) == op.text,
                "sealed history content"
            );
            self.replace_validated(range, text, ChangeOrigin::History);
        }
        self.trace_history(&ops);
        Ok(Some(HistoryMove {
            start: start.into(),
            end: end.into(),
        }))
    }
}
