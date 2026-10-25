//! Parser state belongs to the existing resolution CPU owner. Journal updates
//! are ordered with requests; a missed revision makes Highlighter discard its
//! tree rather than pretending stale syntax describes the new snapshot.
use super::super::structural_indent::{self, Batch, Edit};
use std::collections::HashMap;
use strop_core::id::DocumentId;
use strop_syntax::{Highlighter, IndentContext};

#[derive(Default)]
pub(super) struct Cache {
    slots: HashMap<DocumentId, Slot>,
}
struct Slot {
    path: std::path::PathBuf,
    highlighter: Highlighter,
}
impl Cache {
    pub(super) fn edits(&mut self, document: DocumentId, changes: &[strop_core::Change]) {
        if let Some(slot) = self.slots.get_mut(&document) {
            for change in changes {
                slot.highlighter
                    .apply_edits(&[change.edit], change.revision);
            }
        }
    }

    pub(super) fn resolve(
        &mut self,
        batch: &Batch,
        ropes: &[ropey::Rope],
        cancelled: impl Fn() -> bool,
    ) -> Result<Vec<Edit>, String> {
        if batch.sources.len() != ropes.len() {
            return Err("indentation source snapshot missing".into());
        }
        let mut edits = Vec::new();
        for (source, rope) in batch.sources.iter().zip(ropes) {
            if cancelled() {
                return Err("indentation cancelled".into());
            }
            let buffer = strop_core::Buffer::from_snapshot(rope.clone());
            let cursors = source.cursors();
            let positions = structural_indent::probes(&buffer, &cursors, batch.trigger);
            let syntax_path = source
                .request
                .syntax_path
                .as_ref()
                .filter(|_| batch.trigger != structural_indent::Trigger::Above);
            let contexts = if let Some(path) = syntax_path {
                if !self
                    .slots
                    .get(&source.document)
                    .is_some_and(|slot| slot.path == *path)
                {
                    let highlighter = Highlighter::for_path(path, rope)
                        .ok_or_else(|| "indentation language unavailable".to_string())?;
                    // Keep ordinary document switching warm without retaining
                    // one parser forever for every document visited.
                    if self.slots.len() >= 16 {
                        self.slots.clear();
                    }
                    self.slots.insert(
                        source.document,
                        Slot {
                            path: path.clone(),
                            highlighter,
                        },
                    );
                }
                self.slots
                    .get_mut(&source.document)
                    .ok_or_else(|| "indentation parser slot missing".to_string())?
                    .highlighter
                    .indentation_contexts(rope, source.revision, &positions, &cancelled)
                    .map_err(|error| error.to_string())?
            } else {
                positions
                    .iter()
                    .map(|&cursor| IndentContext {
                        cursor,
                        code: false,
                        opener: None,
                    })
                    .collect()
            };
            edits.extend(source.project(structural_indent::plan(
                &buffer,
                &cursors,
                &source.request,
                &contexts,
            ))?);
        }
        Batch::order(&mut edits)?;
        Ok(edits)
    }
}
