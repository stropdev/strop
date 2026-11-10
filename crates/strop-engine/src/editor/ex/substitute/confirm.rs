//! `:s///c`: ask at each match. The matches come from the one plan the
//! preview showed; an accepted one is applied at once, so the text under
//! the question is the text as it will be. As in Vim, each line's
//! replacements are one undo step.

use std::collections::VecDeque;

use strop_core::frontend_input::Key;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::{Range, Replacement};

use super::SubstitutionPlan;
use crate::editor::ex::*;
use crate::editor::transact::ChangeSet;
use crate::editor::Editor;

/// An open confirmation: the matches not yet answered, in the
/// coordinates of the buffer as it was when the plan was made.
#[derive(Debug)]
pub(crate) struct Confirm {
    document: DocumentId,
    /// The revision the next answer applies to; anything else moved the
    /// text under the plan.
    revision: BufferRevision,
    remaining: VecDeque<Replacement>,
    /// How far accepted replacements have shifted what follows.
    delta: isize,
    replaced: usize,
    /// Where the last accepted replacement starts, as the text is now.
    last: Option<usize>,
}

/// How a confirmation ended; it decides where the cursor rests.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// `a`: like an unconfirmed `:s`, the last changed line's first
    /// non-blank.
    All,
    /// `l`, or `y` at the last match: the changed line's first column.
    Replaced,
    /// `q`, Esc, or `n` at the last match: where the question left it.
    Stopped,
}

impl Confirm {
    /// The match being asked about, where it is now.
    fn current(&self) -> Option<Range> {
        let edit = self.remaining.front()?;
        Some(Range::charwise(
            edit.range.start.get().saturating_add_signed(self.delta),
            edit.range.end.get().saturating_add_signed(self.delta),
        ))
    }
}

pub(crate) fn begin_confirm(editor: &mut Editor, plan: SubstitutionPlan) {
    editor.push_jump();
    editor.pending.ex.confirm = Some(Confirm {
        document: editor.current(),
        revision: editor.buf().revision(),
        remaining: plan.edits.into(),
        delta: 0,
        replaced: 0,
        last: None,
    });
    present_confirm(editor);
}

/// The match a `:s///c` question is about (painted while it waits).
pub(crate) fn confirm_range(editor: &Editor) -> Option<Range> {
    let confirm = editor.pending.ex.confirm.as_ref()?;
    (confirm.document == editor.current())
        .then(|| confirm.current())
        .flatten()
}

pub(crate) fn confirm_active(editor: &Editor) -> bool {
    editor.pending.ex.confirm.is_some()
}

/// Put the cursor on the match in front and ask.
fn present_confirm(editor: &mut Editor) {
    let Some(confirm) = editor.pending.ex.confirm.as_ref() else {
        return;
    };
    let (Some(range), Some(edit)) = (confirm.current(), confirm.remaining.front()) else {
        return;
    };
    let question = format!("replace with {} (y/n/a/q/l)?", edit.text);
    editor.set_head(range.start.get());
    editor.scroll_to_cursor(editor.view_rows());
    editor.message = question;
}

/// Replace the match in front. False when the text moved under the
/// plan.
fn confirm_replace(editor: &mut Editor) -> bool {
    let Some(confirm) = editor.pending.ex.confirm.as_mut() else {
        return false;
    };
    let (Some(range), Some(edit)) = (confirm.current(), confirm.remaining.pop_front()) else {
        return false;
    };
    let (document, revision) = (confirm.document, confirm.revision);
    let grown = edit.text.len() as isize - edit.range.len() as isize;
    let applied = editor.apply(
        document,
        revision,
        ChangeSet {
            edits: vec![Replacement::new(range, edit.text)],
            undo_open: true,
        },
    );
    let (Ok(committed), Some(confirm)) = (applied, editor.pending.ex.confirm.as_mut()) else {
        return false;
    };
    confirm.revision = committed.revision;
    confirm.delta += grown;
    confirm.replaced += 1;
    confirm.last = Some(range.start.get());
    // The next match on another line starts a new undo step.
    let line = editor.buf().line_of(range.start.get());
    let next = confirm_range(editor).map(|next| editor.buf().line_of(next.start.get()));
    if next != Some(line) {
        editor.tx_commit();
    }
    true
}

fn finish_confirm(editor: &mut Editor, ending: Ending) {
    let Some(confirm) = editor.pending.ex.confirm.take() else {
        return;
    };
    if editor.current() == confirm.document {
        editor.tx_commit();
        match (ending, confirm.last) {
            (Ending::All, Some(last)) => ex_land(editor, editor.buf().line_of(last)),
            (Ending::Replaced, Some(last)) => {
                editor.set_head(editor.buf().line_start(editor.buf().line_of(last)));
            }
            _ => {}
        }
        editor.clamp_cursor();
    }
    editor.message = format!(
        "{} substitution{}",
        confirm.replaced,
        if confirm.replaced == 1 { "" } else { "s" }
    );
}

/// One answer: `y` replace, `n` skip, `a` all the rest, `l` this one
/// and stop, `q` / Esc stop. Any other key asks again.
pub(crate) fn feed_confirm(editor: &mut Editor, key: Key) {
    let valid = editor.pending.ex.confirm.as_ref().is_some_and(|confirm| {
        confirm.document == editor.current() && confirm.revision == editor.buf().revision()
    });
    if !valid {
        editor.pending.ex.confirm = None;
        editor.message = "substitute stopped: the buffer changed".into();
        return;
    }
    // `Some(ending)` when this answer ends the confirmation.
    let (ok, ending) = match key {
        Key::Char('y') => {
            let ok = confirm_replace(editor);
            (
                ok,
                confirm_range(editor).is_none().then_some(Ending::Replaced),
            )
        }
        Key::Char('n') => {
            if let Some(confirm) = editor.pending.ex.confirm.as_mut() {
                confirm.remaining.pop_front();
            }
            (
                true,
                confirm_range(editor).is_none().then_some(Ending::Stopped),
            )
        }
        Key::Char('a') => {
            let mut ok = true;
            while ok && confirm_range(editor).is_some() {
                ok = confirm_replace(editor);
            }
            (ok, Some(Ending::All))
        }
        Key::Char('l') => (confirm_replace(editor), Some(Ending::Replaced)),
        Key::Char('q') | Key::Esc => (true, Some(Ending::Stopped)),
        _ => (true, None),
    };
    if !ok {
        finish_confirm(editor, Ending::Stopped);
        editor.message = "substitute stopped: the buffer changed".into();
        return;
    }
    match ending {
        Some(ending) => finish_confirm(editor, ending),
        None => present_confirm(editor),
    }
}
