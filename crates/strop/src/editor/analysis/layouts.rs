//! Native workers retain long-line indexes across cursor and viewport requests.
//! Only immutable sparse checkpoints are published to the editor.
use super::AnalysisTarget;
use std::collections::HashMap;
use std::sync::Arc;
use strop_core::{
    id::{BufferRevision, LineIndex},
    layout::{LineLayoutIndex, PreparedLineLayout},
    Buffer,
};

#[derive(Default)]
pub(crate) struct LayoutCache {
    owner: Option<(AnalysisTarget, BufferRevision, usize)>,
    lines: HashMap<LineIndex, Arc<LineLayoutIndex>>,
}
impl LayoutCache {
    pub fn prepare(
        &mut self,
        buffer: &Buffer,
        target: &AnalysisTarget,
        revision: BufferRevision,
        tab: usize,
        lines: impl IntoIterator<Item = LineIndex>,
        cancelled: impl Fn() -> bool,
    ) -> Option<Vec<PreparedLineLayout>> {
        let owner = (target.clone(), revision, tab);
        if self.owner.as_ref() != Some(&owner) {
            self.lines.clear();
            self.owner = Some(owner);
        }
        let mut result = Vec::new();
        for line in lines {
            if cancelled() {
                return None;
            }
            if let Some(index) = self.lines.get(&line) {
                result.push(PreparedLineLayout {
                    line,
                    index: index.clone(),
                });
            } else if let Some(prepared) = buffer.prepare_line_layout(line, tab, &cancelled) {
                if self.lines.len() == 256 {
                    self.lines.clear();
                }
                self.lines.insert(line, prepared.index.clone());
                result.push(prepared);
            }
        }
        (!cancelled()).then_some(result)
    }
}
