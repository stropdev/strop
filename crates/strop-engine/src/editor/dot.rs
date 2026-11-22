//! `.`: the last change, held by one owner. Every change records itself
//! here as one typed value; `repeat` is a `match` on it, so no
//! branch order decides what repeats.

use strop_grammar::{Command, Op};

use super::normal::edits::Put;
use super::visual::repeat::VisualRepeat;
use super::{Editor, Mode};
use crate::keymap::VisualVerb;
use strop_core::frontend_input::Key;

/// What `.` does again.
#[derive(Debug, Clone)]
pub(crate) enum Change {
    /// A grammar command, resolved afresh from the cursor (0014): `.`
    /// never replays a stale key string through a changed keymap.
    Command(Command),
    /// A direct command's keys (`J`, `gp`, `R`, an Insert entry, …).
    Keys(String),
    /// A Visual verb over the same extent from the cursor.
    Visual(VisualRepeat),
    /// A Normal put, with its register and count.
    Put(Put),
}

impl Change {
    /// A change that opens Insert: when it finds nothing to change,
    /// there is no Insert to type the recorded text into.
    fn opens_insert(&self) -> bool {
        match self {
            Change::Command(command) => command.op == Some(Op::Change),
            Change::Visual(repeat) => matches!(
                repeat.verb,
                VisualVerb::Operator(Op::Change) | VisualVerb::Lines(Op::Change)
            ),
            Change::Keys(_) | Change::Put(_) => false,
        }
    }
}

/// The last change and the text its Insert session typed.
#[derive(Debug, Clone, Default)]
pub(crate) struct Dot {
    change: Option<Change>,
    /// What the last Insert session typed: replayed after the change,
    /// and the `".` register.
    pub(crate) insert: Option<String>,
}

impl Dot {
    /// A new change: an earlier Insert's text is not part of it.
    pub(crate) fn record(&mut self, change: Change) {
        self.change = Some(change);
        self.insert = None;
    }

    /// An Insert entry (`i`, `A`, `o`, …): Esc adds the typed text.
    pub(crate) fn enter(&mut self, change: Change) {
        self.change = Some(change);
    }
}

/// `.`: the last change again. A count replaces the count a Normal
/// change was made with, and sticks for later repeats; a Visual repeat
/// keeps its own count and extent, and a numbered put takes the next
/// register, as Vim's do.
pub(crate) fn repeat(editor: &mut Editor, count: Option<usize>) {
    let insert = editor.dot.insert.clone();
    let change = editor.dot.change.clone();
    if change.is_none() && insert.is_none() {
        return;
    }
    let replay = editor.last_insert_replay.clone();
    match &change {
        Some(Change::Command(command)) => {
            let mut command = command.clone();
            if let Some(count) = count {
                command.count = Some(count);
                editor.dot.change = Some(Change::Command(command.clone()));
            }
            if editor.defer_resolution(
                &command,
                editor.all_cursors(),
                super::resolution::ResolutionPurpose::DotRepeat,
            ) {
                return;
            }
            // the same change, resolved fresh from here (0014 §input)
            editor.execute(&command);
        }
        Some(Change::Keys(keys)) => {
            // Replay the requested count without recording it first. A
            // refused direct command must leave the previous repeat intact.
            let keys = match count {
                Some(count) => {
                    let keys = keys.trim_start_matches(|c: char| c.is_ascii_digit());
                    format!("{count}{keys}")
                }
                None => keys.clone(),
            };
            editor.feed_text(&keys);
        }
        Some(Change::Visual(repeat)) => super::visual::repeat::replay(editor, repeat),
        Some(Change::Put(put)) => put.again(count).run(editor),
        None => {}
    }
    if change.as_ref().is_some_and(Change::opens_insert) && editor.mode != Mode::Insert {
        // the change found no target here
        return;
    }
    if let Some(text) = insert {
        let was_insert = editor.mode == Mode::Insert;
        if !was_insert && !editor.resolution.blocked() {
            editor.enter_insert_from("i");
        }
        if replay.is_empty() {
            for character in text.chars() {
                editor.replay_insert_key(Key::Char(character));
            }
        } else {
            for step in replay {
                editor.replay_insert_step(step);
            }
        }
        editor.replay_insert_key(Key::Esc);
        editor.message = "repeated".into();
    }
}
