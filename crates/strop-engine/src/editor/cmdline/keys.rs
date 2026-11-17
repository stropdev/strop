//! The command line's keys (plan 0110 §3.2): one table from key to
//! action, read by the reducer and by `?` (`keymap/bindings/cmdline.rs`
//! has one row per action; a test pins both directions). Typed
//! characters insert; a key the table does not name does nothing.

use strop_core::frontend_input::Key;

use super::Line;

/// What a command-line key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Run the line (`Enter`, `Ctrl-J`, `Ctrl-M`).
    Run,
    /// Close the line; the view returns to where it was.
    Cancel,
    /// Delete before the cursor; on an empty line, cancel.
    Backspace,
    /// Delete under the cursor (at the end: before it).
    Delete,
    /// Delete the word before the cursor.
    DeleteWord,
    /// Delete everything before the cursor.
    DeleteToStart,
    Left,
    Right,
    Home,
    /// End of the line; with a completion selected, put back what was
    /// typed and close the menu.
    End,
    /// The next key names a register to insert (or `Ctrl-W`/`Ctrl-A`:
    /// the word/WORD under the cursor).
    Register,
    /// Next completion, written into the line.
    Complete,
    /// Previous completion.
    CompleteBack,
    /// Older history line, whatever it starts with; the previous
    /// completion while the menu is open.
    Older,
    /// Newer history line; the next completion while the menu is open.
    Newer,
    /// Older history line that starts with what was typed; the row above
    /// once a completion is selected.
    OlderPrefix,
    /// Newer history line that starts with what was typed.
    NewerPrefix,
    /// Keep the selected completion and close the menu.
    Accept,
    /// Open the menu without selecting.
    List,
    /// Open the command-line window on this line.
    Window,
    /// Repaint the terminal.
    Repaint,
}

/// Every key the line binds. `Ctrl-[`, `Ctrl-J`, `Ctrl-M` and `Ctrl-H`
/// arrive as `Esc`, `Enter` and `Backspace` (the terminal sends those).
pub(crate) const KEYS: &[(Key, Action)] = &[
    (Key::Enter, Action::Run),
    (Key::Esc, Action::Cancel),
    (Key::CtrlC, Action::Cancel),
    (Key::Backspace, Action::Backspace),
    (Key::Delete, Action::Delete),
    (Key::CtrlW, Action::DeleteWord),
    (Key::CtrlU, Action::DeleteToStart),
    (Key::Left, Action::Left),
    (Key::Right, Action::Right),
    (Key::CtrlB, Action::Home),
    (Key::Home, Action::Home),
    (Key::CtrlE, Action::End),
    (Key::End, Action::End),
    (Key::CtrlR, Action::Register),
    (Key::Tab, Action::Complete),
    (Key::Backtab, Action::CompleteBack),
    (Key::CtrlP, Action::Older),
    (Key::CtrlN, Action::Newer),
    (Key::Up, Action::OlderPrefix),
    (Key::Down, Action::NewerPrefix),
    (Key::CtrlY, Action::Accept),
    (Key::CtrlD, Action::List),
    (Key::CtrlF, Action::Window),
    (Key::CtrlL, Action::Repaint),
];

/// The action a key names on the line.
pub(crate) fn action(key: Key) -> Option<Action> {
    KEYS.iter()
        .find(|(bound, _)| *bound == key)
        .map(|(_, action)| *action)
}

/// Vim's word class for `Ctrl-W`: letters, digits, `_` and anything
/// past ASCII are one class, other non-blanks another.
fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || !c.is_ascii()
}

impl Line {
    /// Type one character at the caret (after a pending `Ctrl-R`, the
    /// caller reads the register instead).
    pub(crate) fn insert(&mut self, text: &str) {
        self.edit.insert_str(text);
    }

    /// The body is empty: only the sigil is left.
    pub(crate) fn is_empty(&self) -> bool {
        self.edit.text().len() == 1
    }

    pub(crate) fn backspace(&mut self) {
        if self.edit.cursor() > 1 {
            self.edit.backspace();
        }
    }

    pub(crate) fn delete(&mut self) {
        let text = self.edit.text();
        let at = self.edit.cursor();
        match text[at..].chars().next() {
            Some(c) => self.edit.delete_range(at, at + c.len_utf8()),
            None => self.backspace(),
        }
    }

    /// `Ctrl-W` (ex_getln.c): blanks before the cursor, then the run of
    /// one class — word characters or other non-blanks.
    pub(crate) fn delete_word(&mut self) {
        let text = self.edit.text();
        let end = self.edit.cursor();
        let mut start = end;
        let before = |at: usize| text[1..at].chars().next_back();
        while let Some(c) = before(start).filter(|c| c.is_whitespace()) {
            start -= c.len_utf8();
        }
        if let Some(class) = before(start).map(word_char) {
            while let Some(c) =
                before(start).filter(|c| !c.is_whitespace() && word_char(*c) == class)
            {
                start -= c.len_utf8();
            }
        }
        self.edit.delete_range(start, end);
        self.edit.set_cursor(start);
    }

    pub(crate) fn delete_to_start(&mut self) {
        let end = self.edit.cursor();
        self.edit.delete_range(1, end);
        self.edit.set_cursor(1);
    }

    pub(crate) fn left(&mut self) {
        if self.edit.cursor() > 1 {
            self.edit.move_left();
        }
    }

    pub(crate) fn right(&mut self) {
        self.edit.move_right();
    }

    pub(crate) fn home(&mut self) {
        self.edit.set_cursor(1);
    }

    pub(crate) fn end(&mut self) {
        self.edit.set_cursor(usize::MAX);
    }

    /// `Ctrl-R` waits for a register name.
    pub(crate) fn await_register(&mut self) {
        self.register = true;
    }

    /// Whether `Ctrl-R` is waiting, and stop waiting.
    pub(crate) fn take_register(&mut self) -> bool {
        std::mem::take(&mut self.register)
    }
}
