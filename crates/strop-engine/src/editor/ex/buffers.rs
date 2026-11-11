//! `:b {n|name}`, `:bn`, `:bp`, `:bd [n|name]`. A buffer's number is its
//! slot in the document arena: stable for as long as the buffer lives.

use strop_core::id::DocumentId;

use super::ExCall;
use crate::editor::ex::*;
use crate::editor::Editor;

/// The listed buffers, in number order.
fn listed_buffers(editor: &Editor) -> Vec<DocumentId> {
    editor
        .docs
        .iter()
        .map(|(id, _)| id)
        .filter(|id| buffer_listed(editor, *id))
        .collect()
}

/// The buffer an argument names: a number, `#` (the alternate), or a
/// name — an exact name wins, otherwise the one buffer whose name
/// contains the text (Vim's unique-match rule).
fn find_buffer(editor: &Editor, arg: &str) -> Result<DocumentId, String> {
    if arg == "#" {
        let current = editor.current();
        return editor
            .mru
            .iter()
            .copied()
            .find(|id| *id != current && editor.docs.get(*id).is_some())
            .ok_or_else(|| "no alternate buffer".into());
    }
    if let Ok(number) = arg.parse::<usize>() {
        return editor
            .docs
            .iter()
            .map(|(id, _)| id)
            .find(|id| buffer_number(*id) == number)
            .ok_or_else(|| format!("buffer {number} does not exist"));
    }
    let names: Vec<(DocumentId, String)> = listed_buffers(editor)
        .into_iter()
        .map(|id| (id, editor.doc(id).label(&editor.cwd)))
        .collect();
    if let Some((id, _)) = names.iter().find(|(_, name)| name == arg) {
        return Ok(*id);
    }
    let mut matches = names.iter().filter(|(_, name)| name.contains(arg));
    match (matches.next(), matches.next()) {
        (Some((id, _)), None) => Ok(*id),
        (Some(_), Some(_)) => Err(format!("more than one match for {arg}")),
        (None, _) => Err(format!("no matching buffer for {arg}")),
    }
}

/// The buffer a command addresses: its argument, else its count
/// (`:3b`), else none.
fn buffer_operand(editor: &Editor, call: &ExCall<'_>) -> Result<Option<DocumentId>, String> {
    if !call.arg.is_empty() {
        return find_buffer(editor, call.arg).map(Some);
    }
    match call.range {
        Some(range) => find_buffer(editor, &range.last.to_string()).map(Some),
        None => Ok(None),
    }
}

fn show_buffer(editor: &mut Editor, id: DocumentId) {
    if id == editor.current() {
        return;
    }
    editor.push_jump();
    editor.switch_to(id);
    editor.clamp_cursor();
    editor.discover_git();
}

/// `:b[!] {n|name}`, `:{n}b`
pub(crate) fn ex_buffer(editor: &mut Editor, call: &ExCall<'_>) {
    match buffer_operand(editor, call) {
        Ok(Some(id)) => show_buffer(editor, id),
        Ok(None) => {}
        Err(error) => editor.message = error,
    }
}

/// `:[count]bn`, `:[count]bp` / `:bN`: step through the listed
/// buffers in number order, wrapping at the ends.
pub(crate) fn ex_buffer_step(editor: &mut Editor, call: &ExCall<'_>, forward: bool) {
    let listed = listed_buffers(editor);
    if listed.is_empty() {
        return;
    }
    let steps = match call.arg.parse::<usize>() {
        Ok(steps) => steps,
        Err(_) if call.arg.is_empty() => call.range.map_or(1, |range| range.last),
        Err(_) => {
            editor.message = format!("trailing characters: {}", call.arg);
            return;
        }
    } % listed.len();
    let current = editor.current();
    // From an unlisted buffer (help), "next" is the first listed one.
    let target = match listed.iter().position(|id| *id == current) {
        Some(at) if forward => listed[(at + steps) % listed.len()],
        Some(at) => listed[(at + listed.len() - steps) % listed.len()],
        None if forward => listed[0],
        None => listed[listed.len() - 1],
    };
    show_buffer(editor, target);
}

/// `:bd[!] [n|name]`: delete this buffer or the one named. Another
/// buffer is deleted in place — the window and cursor stay put.
pub(crate) fn ex_buffer_delete(editor: &mut Editor, call: &ExCall<'_>) {
    let target = match buffer_operand(editor, call) {
        Ok(target) => target.unwrap_or(editor.current()),
        Err(error) => {
            editor.message = error;
            return;
        }
    };
    if target == editor.current() {
        editor.delete_buffer(call.bang);
        return;
    }
    let pane = editor.view().clone();
    editor.switch_to(target);
    editor.delete_buffer(call.bang);
    if editor.should_quit || editor.docs.get(pane.doc).is_none() {
        return;
    }
    let refused = editor.docs.get(target).is_some();
    let message = std::mem::take(&mut editor.message);
    editor.switch_to(pane.doc);
    let active = editor.active_pane;
    editor.panes[active] = pane;
    if refused {
        // Say why, and leave the user looking at their own buffer.
        editor.message = message;
    }
}
