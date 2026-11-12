//! Normal mode (0016): the machine walks keys to typed actions; this
//! module dispatches them. Siblings by responsibility: `execute` (the
//! operator engine + editing entries), `search`, `preview`, `pending`
//! (the prompt effects), `motions`. The `:` line is `editor/ex/`.

pub(crate) mod bind;
pub(crate) mod edits;
mod execute;
mod motions;
pub(crate) mod operators;
mod pending;
mod preview;
mod search;

use strop_grammar::{self as grammar, Parse};

use super::Editor;
use strop_core::frontend_input::Key;

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
            self.cancel_remote_write(self.current());
            if self.review.preparing.is_some() {
                self.review_cancel_pub();
            }
            self.collapse_cursors();
            self.walker.clear();
            return;
        }
        // A collection's context keys: `+`/`-` widen and narrow the excerpt
        // under the caret (they are line motions everywhere else).
        if self.walker.is_ground() && self.collections.contains_key(&self.current()) {
            self.collection_command_boundary(self.current());
            match key {
                Key::Char('+') => return self.collection_context_step(true),
                Key::Char('-') => return self.collection_context_step(false),
                _ => {}
            }
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
            super::input::Action::Unbound(keys) => self.message = format!("{keys} is not bound"),
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
        }
    }

    pub(crate) fn refuse_readonly(&mut self) {
        self.message = if self.terminal_document(self.current()).is_some() {
            "terminal buffer is read-only — i enters TERMINAL mode".into()
        } else if self.review.live(self.current()) {
            "a review is read-only — Enter applies and writes, u reverts, q closes".into()
        } else {
            "readonly buffer".into()
        };
    }

    /// A grammar command: motions move, yanks survive readonly, edits
    /// refuse there. The one grammar entrypoint — prompts, aliases and
    /// the Walker all land here.
    pub(crate) fn dispatch_grammar(&mut self, command: &grammar::Command) {
        match command.op {
            None if matches!(command.target, grammar::Target::Match { .. }) => {
                self.select_match(command)
            }
            None => self.move_cursor(command),
            Some(grammar::Op::Yank) if self.buf().readonly => self.yank_only(command),
            Some(_) if self.buf().readonly => self.refuse_readonly(),
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
                    row.sections.contains(&"git")
                        || matches!(
                            row.id,
                            "search-next"
                                | "search-prev"
                                | "visual-enter"
                                | "visual-block"
                                | "pane-nav"
                                | "pane-split"
                                | "pane-close"
                                | "hunk-next"
                                | "hunk-prev"
                                | "clip-yank"
                                | "redraw"
                                | "view-place"
                                | "visible-jumps"
                                | "scroll-pages"
                                | "scroll-lines"
                                | "enter"
                                | "collection-file-next"
                                | "collection-file-prev"
                                | "quickfix-next"
                                | "quickfix-prev"
                                | "jumplist"
                                | "jump-forward"
                                | "goto-definition"
                                | "switch-source-header"
                                | "references"
                                | "implementation"
                                | "type-definition"
                                | "declaration"
                                | "diagnostic-next"
                                | "diagnostic-prev"
                                | "hover"
                                | "remote-open"
                                | "files"
                                | "directory-reveal"
                                | "buffers"
                                | "search"
                                | "replace-global"
                                | "help"
                                | "alternate-buffer"
                                | "word-search"
                                | "document-symbols"
                                | "diagnostics"
                                | "code-actions"
                                | "code-actions-key"
                                | "jumplist-picker"
                                | "undo-tree"
                        )
                }
                Handler::AbsorbChar(
                    crate::keymap::AbsorbKind::Find
                    | crate::keymap::AbsorbKind::MarkSet
                    | crate::keymap::AbsorbKind::MarkJump
                    | crate::keymap::AbsorbKind::MarkLine,
                ) => true,
                _ => false,
            };
            if !allowed {
                self.refuse_readonly();
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
                    "paste-after" => {
                        crate::editor::normal::edits::paste_and_pass(self, register, n, last == 'P')
                    }
                    "paste-indent-after" => {
                        crate::editor::normal::edits::paste_indented(self, register, n, false)
                    }
                    "paste-indent-before" => {
                        crate::editor::normal::edits::paste_indented(self, register, n, true)
                    }
                    "join" => crate::editor::normal::edits::join_lines(self, count, true),
                    "join-raw" => crate::editor::normal::edits::join_lines(self, count, false),
                    "repeat" => self.dot_repeat(count),
                    "toggle-case" => crate::editor::normal::edits::toggle_case(self, n),
                    "number-add" => crate::editor::normal::edits::add_to_number(self, n as i64),
                    "number-subtract" => {
                        crate::editor::normal::edits::add_to_number(self, -(n as i64))
                    }
                    "scroll-pages" => self.scroll_counted(last, n),
                    "scroll-lines" => self.scroll_lines(n, last == 'e'),
                    // Space y: the clipboard yank is typed operator
                    // state in the Walker — `2 yw` yanks two words to
                    // `+`, no synthetic grammar text (R7)
                    "clip-yank" => self
                        .walker
                        .begin_operator(grammar::Op::Yank, Some('+'), count),
                    _ => self.run_counted_leaf(f, last, n),
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
                    AbsorbKind::MarkJump => self.jump_mark(c, true),
                    AbsorbKind::MarkLine => self.jump_mark(c, false),
                    AbsorbKind::Find => {} // grammar resolved f<c> in the machine
                    AbsorbKind::MacroRecord => self.macro_toggle(c),
                    AbsorbKind::MacroPlay if c == '@' => self.macro_again(n),
                    AbsorbKind::MacroPlay if c == ':' => self.repeat_command_line(n),
                    AbsorbKind::MacroPlay => self.macro_play(c, n),
                }
            }
            Handler::Prefix
            | Handler::Motion
            | Handler::Operator
            | Handler::ObjectPrefix
            | Handler::TextLine
            | Handler::AbsorbRegister
            | Handler::Visual(_)
            | Handler::Soon
            | Handler::Contextual => {}
        }
    }

    // ---- handler shims for the command table (0008 stage 2) ---------

    pub(crate) fn repeat_search_pub(&mut self, invert: bool) {
        self.repeat_search(invert);
    }
    pub(crate) fn jump_hunk_pub(&mut self, forward: bool) {
        self.jump_hunk(forward);
    }
    pub(crate) fn repeat_find_pub(&mut self, reverse: bool) {
        self.repeat_find(reverse);
    }
    /// "p P" row: the completing key picks before/after.
    pub(crate) fn paste_named_pub(&mut self, name: Option<char>, count: usize, before: bool) {
        self.paste_named(name, count, before);
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
        self.enter_visual_mode(if key == 'V' {
            super::Mode::VisualLine
        } else {
            super::Mode::Visual
        });
    }
}
