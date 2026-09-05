//! normal/preview.rs — the pending-operation preview (0001 pillar 2).

use strop_core::Range;
use strop_grammar::{self as grammar, Parse};

use crate::editor::Editor;

impl Editor {
    /// The live preview: what would the pending keys do right now?
    /// The plan the executor would apply — every cursor's range (0014
    /// wave 3): the preview cannot lie, and multicursor previews too.
    pub fn preview(&self) -> Option<(Vec<Range>, String)> {
        if self.pending.is_empty() {
            return None;
        }
        match grammar::parse(&self.pending) {
            Parse::Complete(cmd) if cmd.op.is_some() => {
                let spec = grammar::resolve(self.buf(), self.head(), &cmd)?.spec;
                let plan = grammar::plan(self.buf(), &self.all_cursors(), &cmd)?;
                Some((plan.targets.iter().map(|t| t.range).collect(), spec))
            }
            _ => {
                // partial backward search: d?foo mid-typing previews match→cursor
                if let Some(idx) = self.pending.find('?') {
                    let pat = &self.pending[idx + 1..];
                    if !pat.is_empty() && !pat.contains('\r') {
                        if let Some(hit) = grammar::search_backward(self.buf(), self.head(), pat) {
                            return Some((
                                vec![Range::charwise(hit, self.head())],
                                format!("search ?{pat}"),
                            ));
                        }
                    }
                }

                // partial search: d/foo mid-typing previews cursor→first match
                if let Some(idx) = self.pending.find('/') {
                    let pat = &self.pending[idx + 1..];
                    if !pat.is_empty() {
                        if let Some(hit) = grammar::search_forward(self.buf(), self.head() + 1, pat)
                        {
                            return Some((
                                vec![Range::charwise(self.head(), hit)],
                                format!("search /{pat}"),
                            ));
                        }
                    }
                }

                None
            }
        }
    }
}
