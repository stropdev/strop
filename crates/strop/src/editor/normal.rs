//! Normal mode (0016): the machine walks keys to typed actions; this
//! module dispatches them. Siblings by responsibility: `execute` (the
//! operator engine + editing entries), `search`, `preview`, `pending`
//! (the modal text lines), `ex` (the `:` line), `motions`.

mod ex;
mod execute;
mod motions;
mod pending;
mod preview;
mod search;

use strop_grammar::{self as grammar, Parse};

use super::{Editor, Key};

/// The ex vocabulary (completion + `run_ex` dispatch reads the same
/// list — one table, no drift).
pub(crate) const EX_COMMANDS: &[(&str, &str)] = &[
    ("w", "write"),
    ("w!", "write, force (file changed on disk)"),
    ("wq!", "write forced + quit"),
    ("q", "quit"),
    ("q!", "quit, force"),
    ("wq", "write + quit"),
    ("e", "edit file"),
    ("e!", "edit file, force"),
    ("vs", "split vertical"),
    ("vsplit", "split vertical"),
    ("sp", "split horizontal"),
    ("split", "split horizontal"),
    ("help", "help buffer"),
    ("h", "help buffer"),
    ("!", "run shell command"),
    ("view", "open readonly"),
];

impl Editor {
    pub(crate) fn feed_normal(&mut self, key: Key) {
        // readonly surfaces (git browser/blame/etc.): q closes, Enter
        // dives, motions and yank fall through, edits refuse (0001 §3)
        if self.buf().readonly {
            return self.feed_readonly(key);
        }
        if !self.pending.is_empty() {
            return self.feed_pending(key);
        }
        // Esc is a mode-level key: collapse to the primary cursor and
        // ground the machine (0013 §3) — it never walks the trie
        if key == Key::Esc {
            self.collapse_cursors();
            self.walker.clear();
            return;
        }
        // every other key event walks the one machine (0016)
        match self.walker.feed(key) {
            super::input::Action::Pending => {}
            super::input::Action::Invalid(keys) => {
                self.message = format!("not an editor command: {keys}")
            }
            super::input::Action::EnterText(c) => {
                self.pending = c.to_string();
                self.pending_normal = false;
                self.pending_cursor = self.pending.len();
            }
            super::input::Action::Grammar(cmd) => match cmd.op {
                None => self.move_cursor(&cmd),
                Some(_) => self.execute(&cmd),
            },
            super::input::Action::Row {
                row,
                count,
                register,
                arg,
                key,
            } => self.dispatch_row(row, count, register, arg, key),
        }
    }

    /// A table row dispatch: typed count/register/arg ride the Action
    /// (0016 — no string inspection). Count semantics by id:
    /// most leaves repeat, insert entries carry it into the session,
    /// visible jumps treat it as a line offset, replace multiplies.
    fn dispatch_row(
        &mut self,
        row: &'static crate::keymap::Binding,
        count: Option<usize>,
        register: Option<char>,
        arg: Option<char>,
        key: char,
    ) {
        use crate::keymap::Handler;
        let n = count.unwrap_or(1);
        match row.handler {
            Handler::Leaf(f) => {
                let last = key;
                match row.id {
                    "visible-jumps" => self.jump_visible(last, n),
                    "insert-entries" => {
                        self.insert_count = n;
                        f(self, last);
                    }
                    "paste" if register.is_some() => self.paste_named(register, last == 'P'),
                    "paste" => self.paste_n(n, last == 'P'),
                    "scroll-pages" => self.scroll_counted(last, n),
                    _ => {
                        for _ in 0..n {
                            f(self, last);
                        }
                    }
                }
            }
            // aliases are semantic (0016): the expansion parses ONCE
            // into a grammar Command; the walker's count/register merge
            // in — nothing replays through input
            Handler::Alias(expansion) => {
                if let Parse::Complete(mut cmd) = grammar::parse(expansion) {
                    cmd.count = Some(n * cmd.count.unwrap_or(1));
                    if register.is_some() {
                        cmd.register = register;
                    }
                    match cmd.op {
                        None => self.move_cursor(&cmd),
                        Some(_) => self.execute(&cmd),
                    }
                }
            }
            Handler::AbsorbChar(kind) => {
                let c = arg.unwrap_or('\0');
                use crate::keymap::AbsorbKind;
                match kind {
                    AbsorbKind::Replace => self.replace_char_n(c, n),
                    AbsorbKind::MarkSet => self.set_mark(c),
                    AbsorbKind::MarkJump => self.jump_mark(c),
                    AbsorbKind::Find => {} // grammar resolved f<c> in the machine
                    AbsorbKind::MacroRecord => self.macro_toggle(c),
                    AbsorbKind::MacroPlay if c == '@' => self.macro_again(n),
                    AbsorbKind::MacroPlay => self.macro_play(c, n),
                }
            }
            Handler::Prefix
            | Handler::Motion
            | Handler::Operator
            | Handler::ObjectPrefix
            | Handler::TextLine
            | Handler::AbsorbRegister
            | Handler::Soon => {}
        }
    }

    // ---- handler shims for the command table (0008 stage 2) ---------

    pub(crate) fn repeat_search_pub(&mut self, invert: bool) {
        self.repeat_search(invert);
    }
    pub(crate) fn jump_hunk_pub(&mut self, forward: bool) {
        self.jump_hunk(forward);
    }
    pub(crate) fn search_word_under_cursor_pub(&mut self, backward: bool) {
        self.search_word_under_cursor(backward);
    }
    pub(crate) fn repeat_find_pub(&mut self, reverse: bool) {
        self.repeat_find(reverse);
    }
    /// "p P" row: the completing key picks before/after.
    pub(crate) fn paste_named_pub(&mut self, name: Option<char>, before: bool) {
        self.paste_named(name, before);
    }
    /// "J ." row: the completing key picks the command.
    pub(crate) fn join_or_repeat(&mut self, key: char) {
        if key == 'J' {
            self.join_lines_pub();
        } else {
            self.dot_repeat_pub();
        }
    }
    /// "i a A o O I" row: insert entries by key.
    pub(crate) fn insert_entry_pub(&mut self, key: char) {
        match key {
            'i' => self.enter_insert_from("i"),
            'a' => self.append(),
            'A' => self.append_eol(),
            'o' => self.open_below(),
            'O' => self.open_above(),
            'I' => self.alias("I", "^i"),
            _ => {}
        }
    }
    /// "v V" row.
    pub(crate) fn enter_visual_pub(&mut self, key: char) {
        self.enter_visual(key == 'V');
    }
}

mod dbg3 {
    #[test]
    fn editor_3x() {
        let mut e = crate::editor::Editor::new(strop_core::Buffer::from_text("abcde\n"));
        e.feed_text("3x");
        eprintln!("text {:?} msg {:?}", e.buf().rope.to_string(), e.message);
    }
}
