//! normal/preview.rs — the pending-operation preview (0001 pillar 2).
//! The same typed command and saved origin feed preview and execution:
//! a search prompt composes with its operator exactly as typed.

use strop_core::Range;
use strop_grammar::{self as grammar, Parse};

use crate::editor::Editor;

impl Editor {
    /// The live preview: what would the pending keys do right now?
    /// The plan the executor would apply — every cursor's range (0014
    /// wave 3): the preview cannot lie. Query errors surface as `Err`
    /// (the modeline shows them); no operator or no origin means no
    /// preview at all.
    pub fn preview(&self) -> Result<Option<(Vec<Range>, String)>, String> {
        if let Some(state) = self.substitute_preview() {
            return match state {
                crate::editor::SubstitutePreviewState::Ready(preview) => {
                    Ok(Some((preview.ranges().collect(), preview.summary())))
                }
                crate::editor::SubstitutePreviewState::Invalid(error)
                | crate::editor::SubstitutePreviewState::Unavailable(error) => Err(error),
                crate::editor::SubstitutePreviewState::Resolving
                | crate::editor::SubstitutePreviewState::NoTarget
                | crate::editor::SubstitutePreviewState::Incomplete(_) => Ok(None),
            };
        }
        let Some((command, cursors)) = self.preview_command()? else {
            return Ok(None);
        };
        if command.op.is_none()
            || (self.resolution_is_large(&command) && !self.resolution_ready(&command, &cursors))
        {
            return Ok(None);
        }
        let resolved = self.resolved_many(&command, &cursors)?;
        let Some(Some(primary)) = resolved.first() else {
            return Ok(None);
        };
        let Some(plan) = self.resolved_plan(&command, &cursors)? else {
            return Ok(None);
        };
        Ok(Some((
            plan.targets.iter().map(|target| target.range).collect(),
            primary.spec.clone(),
        )))
    }

    pub fn preview_is_resolving(&self) -> bool {
        let Ok(Some((command, cursors))) = self.preview_command() else {
            return false;
        };
        command.op.is_some()
            && self.resolution_is_large(&command)
            && !self.resolution_ready(&command, &cursors)
    }

    pub(crate) fn prepare_resolution_preview(&mut self) {
        if self.resolution.blocked()
            || self
                .pending
                .prompt()
                .is_some_and(|prompt| prompt.search().is_some())
        {
            return;
        }
        if let Some(prompt) = self.pending.prompt() {
            if let crate::editor::pending::PromptContext::Ex(origin) = prompt.context() {
                if !self.pending_origin_valid(origin) {
                    self.resolution.cancel_preview();
                    return;
                }
                match crate::editor::ex::parse_substitute_line(
                    self,
                    prompt.body(),
                    origin.pane.sels.primary().head,
                ) {
                    Ok(Some(spec)) => {
                        self.prepare_substitute_resolution(spec, origin.pane.sels.heads());
                    }
                    Ok(None) | Err(_) => self.resolution.cancel_preview(),
                }
                return;
            }
        }
        match self.preview_command() {
            Ok(Some((command, cursors))) => {
                self.defer_resolution(
                    &command,
                    cursors,
                    super::super::resolution::ResolutionPurpose::Preview,
                );
            }
            Ok(None) => self.resolution.cancel_preview(),
            Err(error) => {
                self.resolution.cancel_preview();
                self.message = error;
            }
        }
    }

    fn preview_command(&self) -> Result<Option<(grammar::Command, Vec<usize>)>, String> {
        if let Some(prompt) = self.pending.prompt() {
            let Some((origin, state)) = prompt.search() else {
                return Ok(None);
            };
            if state.op.is_none() || !self.pending_origin_valid(origin) {
                return Ok(None);
            }
            let Some(command) = self.search_prompt_command(prompt, false)? else {
                return Ok(None);
            };
            Ok(Some((command, origin.pane.sels.heads())))
        } else {
            let Some((operator, motion)) = self.walker.op_motion() else {
                return Ok(None);
            };
            let keys = operator.key().to_string() + motion;
            let mut command = match grammar::parse(&keys) {
                Parse::Complete(command) => command,
                Parse::QueryError(error) => return Err(error.to_string()),
                Parse::Incomplete | Parse::Invalid => return Ok(None),
            };
            command.count = self.walker.state.count();
            command.register = self.walker.state.register;
            Ok(Some((command, self.all_cursors())))
        }
    }
}
