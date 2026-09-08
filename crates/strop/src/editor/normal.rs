//! Normal mode (0016): the machine walks keys to typed actions; this
//! module dispatches them. Siblings by responsibility: `execute` (the
//! operator engine + editing entries), `search`, `preview`, `pending`
//! (the prompt effects), `ex` (the `:` line), `motions`.

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
    ("tail", "remote tail: :tail [BYTES] URI"),
    ("range", "remote range: :range START BYTES URI"),
    ("follow", "follow remote EOF: :follow [URI]"),
    ("unfollow", "stop remote following"),
    ("browse", "browse remote directory: :browse [URI]"),
    ("filter", "filter remote directory by literal filename"),
    (
        "remote",
        "connections: connect/disconnect ssh://HOST, clear, list",
    ),
];

impl Editor {
    pub(crate) fn feed_normal(&mut self, key: Key) {
        // readonly surfaces (git browser/blame/etc.) share the Walker;
        // their surface-specific keys live in feed_readonly (0001 §3)
        if self.buf().readonly {
            return self.feed_readonly(key);
        }
        // Esc is a mode-level key: collapse to the primary cursor and
        // ground the machine (0013 §3) — it never walks the trie
        if key == Key::Esc {
            self.collapse_cursors();
            self.walker.clear();
            return;
        }
        // every other key event walks the one machine (0016)
        self.feed_command(key);
    }

    /// Shared Walker dispatch: normal mode, readonly surfaces, and any
    /// surface mid-composition all reduce typed actions here.
    pub(crate) fn feed_command(&mut self, key: Key) {
        let action = self.walker.feed(key);
        match action {
            super::input::Action::Pending => {}
            super::input::Action::Invalid(keys) => {
                self.message = format!("not an editor command: {keys}")
            }
            super::input::Action::QueryError(error) => self.message = error.to_string(),
            super::input::Action::EnterText { sigil, state } => self.begin_text_line(sigil, state),
            super::input::Action::Grammar(command) => self.dispatch_grammar(&command),
            super::input::Action::Row {
                row,
                count,
                register,
                arg,
                key,
            } => self.dispatch_row(row, count, register, arg, key),
            super::input::Action::VisualSurround(_) => {
                unreachable!("normal Walker cannot emit visual actions")
            }
        }
    }

    /// A grammar command: motions move, yanks survive readonly, edits
    /// refuse there. The one grammar entrypoint — prompts, aliases and
    /// the Walker all land here.
    pub(crate) fn dispatch_grammar(&mut self, command: &grammar::Command) {
        match command.op {
            None => self.move_cursor(command),
            Some(grammar::Op::Yank) if self.buf().readonly => self.yank_only(command),
            Some(_) if self.buf().readonly => self.message = "readonly buffer".into(),
            Some(_) => self.execute(command),
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
        if self.buf().readonly {
            // dispatch capability check on existing stable command ids,
            // not a second grammar: git handlers act on their stamped
            // source targets; navigation/yank/visual stay shared
            let allowed = match row.handler {
                Handler::Alias(_) => true, // resolved typed op is checked by dispatch_grammar
                Handler::Leaf(_) => {
                    row.section == "git"
                        || matches!(
                            row.id,
                            "search-next"
                                | "search-prev"
                                | "visual-enter"
                                | "visual-block"
                                | "pane-nav"
                                | "pane-split"
                                | "pane-close"
                                | "hunk-nav"
                                | "clip-yank"
                                | "redraw"
                                | "view-place"
                                | "visible-jumps"
                                | "scroll-pages"
                                | "enter"
                                | "jumplist"
                                | "jump-forward"
                                | "goto-definition"
                                | "switch-source-header"
                                | "references"
                                | "implementation"
                                | "type-definition"
                                | "declaration"
                                | "diagnostic-jumps"
                                | "hover"
                        )
                }
                Handler::AbsorbChar(
                    crate::keymap::AbsorbKind::Find
                    | crate::keymap::AbsorbKind::MarkSet
                    | crate::keymap::AbsorbKind::MarkJump,
                ) => true,
                _ => false,
            };
            if !allowed {
                self.message = "readonly buffer".into();
                return;
            }
        }
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
                    "paste" => self.paste_named(register, n, last == 'P'),
                    "scroll-pages" => self.scroll_counted(last, n),
                    // Space y: the clipboard yank is typed operator
                    // state in the Walker — `2 yw` yanks two words to
                    // `+`, no synthetic grammar text (R7)
                    "clip-yank" => self
                        .walker
                        .begin_operator(grammar::Op::Yank, Some('+'), count),
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
            Handler::Alias(expansion) => match grammar::parse(expansion) {
                Parse::Complete(mut cmd) => {
                    cmd.count = Some(n.saturating_mul(cmd.count.unwrap_or(1)));
                    if register.is_some() {
                        cmd.register = register;
                    }
                    self.dispatch_grammar(&cmd);
                }
                Parse::QueryError(error) => self.message = error.to_string(),
                Parse::Incomplete | Parse::Invalid => {
                    self.message = format!("not an editor command: {key}");
                }
            },
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
    pub(crate) fn paste_named_pub(&mut self, name: Option<char>, count: usize, before: bool) {
        self.paste_named(name, count, before);
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
            'I' => {
                self.run_motion("^");
                self.enter_insert_from("I");
            }
            _ => {}
        }
    }
    /// "v V" row.
    pub(crate) fn enter_visual_pub(&mut self, key: char) {
        self.enter_visual(key == 'V');
    }
}
