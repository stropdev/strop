//! normal/preview.rs — the pending-operation preview (0001 pillar 2).

use strop_core::Range;
use strop_grammar::{self as grammar, Parse};

use crate::editor::Editor;

impl Editor {
    /// The live preview: what would the pending keys do right now?
    /// The plan the executor would apply — every cursor's range (0014
    /// wave 3): the preview cannot lie, and multicursor previews too.
    pub fn preview(&self) -> Option<(Vec<Range>, String)> {
        let mut keys = if self.pending.is_empty() {
            let (operator, motion) = self.walker.op_motion()?;
            operator.key().to_string() + motion
        } else {
            // Plain searches and text commands have no affected edit range.
            if self.pending.starts_with(['/', '?', ':', '|']) {
                return None;
            }
            self.pending.clone()
        };
        if keys.contains(['/', '?']) && !keys.ends_with('\r') {
            keys.push('\r');
        }
        let Parse::Complete(mut command) = grammar::parse(&keys) else {
            return None;
        };
        command.op?;
        if self.pending.is_empty() {
            command.count = self.walker.state.count();
            command.register = self.walker.state.register;
        }
        let resolved = grammar::resolve(self.buf(), self.head(), &command)?;
        let plan = grammar::plan(self.buf(), &self.all_cursors(), &command)?;
        Some((
            plan.targets.iter().map(|target| target.range).collect(),
            resolved.spec,
        ))
    }
}
