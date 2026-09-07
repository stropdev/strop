//! Validation at the untrusted persistence boundary, using shared rope snapshots.
use super::{Edit, EditKind, History};
use ropey::Rope;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    #[error("invalid history revision graph")]
    Graph,
    #[error("history edit range is outside text or splits UTF-8")]
    Range,
    #[error("history edit does not match the recorded text")]
    TextMismatch,
}

impl History {
    pub fn validate(&self) -> Result<(), HistoryError> {
        if self.revisions.is_empty() || self.current >= self.revisions.len() {
            return Err(HistoryError::Graph);
        }
        for (index, revision) in self.revisions.iter().enumerate() {
            if (index == 0 && revision.parent != 0) || (index > 0 && revision.parent >= index) {
                return Err(HistoryError::Graph);
            }
            if let Some(child) = revision.last_child {
                if child <= index
                    || child >= self.revisions.len()
                    || self.revisions[child].parent != index
                {
                    return Err(HistoryError::Graph);
                }
            }
        }
        Ok(())
    }

    pub fn validate_for(&self, text: &Rope) -> Result<(), HistoryError> {
        self.validate()?;
        if self.pending.is_some() {
            return Err(HistoryError::Graph);
        }
        let mut root = text.clone();
        let mut current = self.current;
        while current != 0 {
            apply(&mut root, self.revisions[current].undo.iter().rev())?;
            current = self.revisions[current].parent;
        }
        let mut states = Vec::with_capacity(self.revisions.len());
        states.push(root);
        for revision in self.revisions.iter().skip(1) {
            let parent = &states[revision.parent];
            let mut child = parent.clone();
            apply(&mut child, revision.redo.iter())?;
            let mut restored = child.clone();
            apply(&mut restored, revision.undo.iter().rev())?;
            if &restored != parent {
                return Err(HistoryError::TextMismatch);
            }
            states.push(child);
        }
        if states[self.current] != *text {
            return Err(HistoryError::TextMismatch);
        }
        Ok(())
    }
}

fn boundary(text: &Rope, byte: usize) -> bool {
    byte <= text.len_bytes() && text.char_to_byte(text.byte_to_char(byte)) == byte
}

fn apply<'a>(text: &mut Rope, edits: impl Iterator<Item = &'a Edit>) -> Result<(), HistoryError> {
    for edit in edits {
        if !boundary(text, edit.at) {
            return Err(HistoryError::Range);
        }
        match edit.kind {
            EditKind::Insert => text.insert(text.byte_to_char(edit.at), &edit.text),
            EditKind::Delete => {
                let end = edit
                    .at
                    .checked_add(edit.text.len())
                    .ok_or(HistoryError::Range)?;
                if !boundary(text, end) {
                    return Err(HistoryError::Range);
                }
                if text.byte_slice(edit.at..end) != edit.text {
                    return Err(HistoryError::TextMismatch);
                }
                text.remove(text.byte_to_char(edit.at)..text.byte_to_char(end));
            }
        }
    }
    Ok(())
}
