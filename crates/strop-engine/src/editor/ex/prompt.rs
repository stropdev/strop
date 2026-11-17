//! `:` completion from the command table.

use super::table::COMMANDS;
use crate::editor::Editor;

/// Ex-completion candidates for the typed prefix (name, doc): the
/// table's names, in table order.
pub(crate) fn ex_candidates(editor: &Editor) -> Vec<(&'static str, &'static str)> {
    let Some(prefix) = editor.cmdline.text().strip_prefix(':') else {
        return Vec::new();
    };
    if !prefix.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
        return Vec::new();
    }
    COMMANDS
        .iter()
        .flat_map(|command| {
            command
                .names
                .iter()
                .map(move |name| (name.name, command.doc))
        })
        .filter(|(name, _)| name.starts_with(prefix))
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
    let prefix = editor.cmdline.text().strip_prefix(':').unwrap_or("");
    let next = cands
        .iter()
        .position(|(name, _)| *name == prefix)
        .map_or(cands[0].0, |i| cands[(i + 1) % cands.len()].0);
    if let Some(line) = editor.cmdline.line_mut() {
        line.set_body(next);
    }
}
