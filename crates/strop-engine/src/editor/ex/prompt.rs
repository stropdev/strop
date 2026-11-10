//! The `:` field itself: completion from the command table and the
//! field's modal state.

use super::table::COMMANDS;
use crate::editor::Editor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingFieldMode {
    Insert,
    Normal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingFieldEscape {
    EnterNormal,
    CancelPrompt,
}
/// Modal ownership of an Ex/search field, including an operator outside it.
pub struct PendingFieldState {
    pub mode: PendingFieldMode,
    pub outer_operator: Option<strop_grammar::Op>,
    pub next_escape: PendingFieldEscape,
}

/// Ex-completion candidates for the pending prefix (name, doc): the
/// table's full names, in table order.
pub(crate) fn ex_candidates(editor: &Editor) -> Vec<(&'static str, &'static str)> {
    let Some(prefix) = editor.pending.text().strip_prefix(':') else {
        return Vec::new();
    };
    if !prefix.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
        return Vec::new();
    }
    COMMANDS
        .iter()
        .filter(|command| command.name.starts_with(prefix))
        .map(|command| (command.name, command.doc))
        .collect()
}

/// Tab on the ex line: cycle the completion candidates — command
/// names for a bare prefix; remote hosts/paths for a file
/// command's `ssh://` operand (editor/remote_completion.rs).
pub(crate) fn ex_tab_complete(editor: &mut Editor) {
    if editor.remote_completion_tab() {
        return;
    }
    let cands = ex_candidates(editor);
    if cands.is_empty() {
        return;
    }
    let prefix = editor.pending.text().strip_prefix(':').unwrap_or("");
    let next = cands
        .iter()
        .position(|(name, _)| *name == prefix)
        .map_or(cands[0].0, |i| cands[(i + 1) % cands.len()].0);
    editor.feed_pending_event(crate::editor::pending::PendingEvent::CompleteEx(
        next.to_owned(),
    ));
}

/// Field ownership is independent of the outer walker's operator grammar.
pub(crate) fn pending_field_state(editor: &Editor) -> Option<PendingFieldState> {
    let prompt = editor.pending.prompt()?;
    if !matches!(
        prompt.context(),
        crate::editor::pending::PromptContext::Ex(_)
            | crate::editor::pending::PromptContext::Search { .. }
    ) {
        return None;
    }
    let outer_operator = match prompt.context() {
        crate::editor::pending::PromptContext::Search { state, .. } => state.op,
        _ => editor.walker.state.op,
    };
    let normal = prompt.normal();
    Some(PendingFieldState {
        mode: if normal {
            PendingFieldMode::Normal
        } else {
            PendingFieldMode::Insert
        },
        outer_operator,
        next_escape: if normal {
            PendingFieldEscape::CancelPrompt
        } else {
            PendingFieldEscape::EnterNormal
        },
    })
}
