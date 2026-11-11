//! normal/execute.rs — the operator engine + editing entry points.

use strop_core::Range;
use strop_grammar::{self as grammar, Command, Op};

use crate::editor::{Editor, Mode};
use strop_core::frontend_input::Key;

use super::operators::{Operand, Operation};

impl Editor {
    /// ds" / cs"' / ysiw" (sandwich lineage). Returns Some when the
    /// command was a surround op and got handled.
    fn execute_surround(&mut self, cmd: &Command) -> Option<()> {
        if !matches!(
            cmd.target,
            grammar::Target::SurroundDelete(_)
                | grammar::Target::SurroundChange { .. }
                | grammar::Target::SurroundAdd { .. }
        ) {
            return None;
        }
        // resolve is typed (R5): a refused query surfaces its error —
        // surround never guesses past one
        let r = match self
            .resolved_many(cmd, &self.all_cursors())
            .map(|resolved| resolved.into_iter().next().flatten())
        {
            Ok(Some(resolved)) => resolved,
            Ok(None) => return None,
            Err(error) => {
                self.message = error.to_string();
                return None;
            }
        };
        let pair = |ch: char| match ch {
            'b' | '(' | ')' => ('(', ')'),
            'r' | '[' | ']' => ('[', ']'),
            'a' | '<' | '>' => ('<', '>'),
            q => (q, q),
        };
        self.tx_begin();
        match &cmd.target {
            grammar::Target::SurroundDelete(_) => {
                // close first so the open's offset stays valid
                self.buf_mut()
                    .delete(Range::charwise(r.range.end - 1, r.range.end));
                self.buf_mut()
                    .delete(Range::charwise(r.range.start, r.range.start + 1));
                self.set_head(r.range.start);
            }
            grammar::Target::SurroundChange { to, .. } => {
                let (o, c) = pair(*to);
                self.buf_mut()
                    .delete(Range::charwise(r.range.end - 1, r.range.end));
                self.buf_mut().insert(r.range.end - 1, &c.to_string());
                self.buf_mut()
                    .delete(Range::charwise(r.range.start, r.range.start + 1));
                self.buf_mut().insert(r.range.start, &o.to_string());
                self.set_head(r.range.start);
            }
            grammar::Target::SurroundAdd { ch, .. } => {
                let (o, c) = pair(*ch);
                self.buf_mut().insert(r.range.end, &c.to_string());
                self.buf_mut().insert(r.range.start, &o.to_string());
                self.set_head(r.range.start + 1);
            }
            _ => {
                self.tx_commit();
                return None;
            }
        }
        self.tx_commit();
        self.clamp_cursor();
        self.flash(Range::charwise(self.head(), self.head()));
        self.last_cmd_keys = cmd.keys.clone();
        self.last_insert = None;
        Some(())
    }

    /// Run an operator command. `typed` is the command as the keys spelled
    /// it: dot-repeat keeps that form, so `.` binds marks and the last
    /// search afresh.
    pub(super) fn execute(&mut self, typed: &Command) {
        let bound = match crate::editor::normal::bind::bind_command(self, typed) {
            Ok(bound) => bound.into_owned(),
            Err(message) => {
                self.message = message;
                return;
            }
        };
        let cmd = &bound;
        if self.defer_resolution(
            cmd,
            self.all_cursors(),
            super::super::resolution::ResolutionPurpose::Execute,
        ) {
            return;
        }
        let Some(op) = cmd.op else {
            return;
        };
        if !self.register_writable(cmd.register) {
            return;
        }
        self.note_search(cmd);
        // surround targets execute as pair edits, not operator ranges
        if let Some(()) = self.execute_surround(cmd) {
            return;
        }
        // the cascade (0013 §3) IS the plan (0014 §3): preview renders
        // these same targets — one object, no preview/execute drift
        let plan = match self.resolved_plan(cmd, &self.all_cursors()) {
            Ok(Some(plan)) => plan,
            // vim: a motion that fails cancels the operator, quietly
            Ok(None) => {
                if matches!(
                    cmd.target,
                    grammar::Target::Motion(
                        grammar::Motion::ScreenDown { .. } | grammar::Motion::ScreenUp { .. }
                    )
                ) {
                    // … but gj/gk have moved as far as they could by then
                    let mut motion = cmd.clone();
                    motion.op = None;
                    self.move_cursor(&motion);
                }
                return;
            }
            // a refused query is typed (R5) — it surfaces, never
            // masquerades as "no target"
            Err(error) => {
                self.message = error.to_string();
                return;
            }
        };
        // semantic dot-repeat (0014): `.` re-resolves this command from
        // the new position — it never replays a stale key string through
        // a changed keymap. Only a command that found its target is kept.
        if op != Op::Yank && op != Op::Filter && !op.missing() {
            self.last_change = Some(typed.clone());
            self.last_cmd_keys.clear();
            self.last_insert = None;
        }
        let operands: Vec<Operand> = plan
            .targets
            .iter()
            .map(|target| Operand {
                cursor: target.cursor,
                range: target.range,
                origin: target.origin,
                end_adjusted: target.end_adjusted,
            })
            .collect();
        let column = operands
            .iter()
            .find(|operand| operand.cursor == self.head())
            .or(operands.first())
            .and_then(|operand| crate::editor::normal::operators::kept_column(self, op, operand));
        // `>$`, `>g_`: the cursor wanted the line's end and gets it
        let column = match (op, &cmd.target) {
            (
                Op::Indent | Op::Dedent,
                grammar::Target::Motion(grammar::Motion::LineEnd | grammar::Motion::LastNonBlank),
            ) => column.map(|_| strop_core::id::DisplayColumn::new(usize::MAX / 2)),
            _ => column,
        };
        crate::editor::normal::operators::apply_operator(
            self,
            Operation {
                op,
                register: cmd.register,
                operands: &operands,
                reg_one: crate::editor::registers::reg_one_motion(&cmd.target),
                keys: &cmd.keys,
                column,
            },
        );
    }

    /// `.`: the last change again. A count replaces the count the change
    /// was made with, and sticks for later repeats.
    pub(crate) fn dot_repeat(&mut self, count: Option<usize>) {
        if self.last_change.is_none() && self.last_cmd_keys.is_empty() && self.last_insert.is_none()
        {
            return;
        }
        if let Some(count) = count {
            if let Some(change) = self.last_change.as_mut() {
                change.count = Some(count);
            } else {
                let keys = self
                    .last_cmd_keys
                    .trim_start_matches(|c: char| c.is_ascii_digit());
                self.last_cmd_keys = format!("{count}{keys}");
            }
        }
        if let Some(command) = self.last_change.clone() {
            if self.defer_resolution(
                &command,
                self.all_cursors(),
                super::super::resolution::ResolutionPurpose::DotRepeat,
            ) {
                return;
            }
        }
        let insert = self.last_insert.clone();
        let replay = self.last_insert_replay.clone();
        if let Some(cmd) = self.last_change.clone() {
            // the same change, resolved fresh from here (0014 §input)
            self.execute(&cmd);
            if cmd.op == Some(Op::Change) && self.mode != Mode::Insert && insert.is_some() {
                // the change found no target here: there is no Insert
                // to type the text into
                return;
            }
        } else {
            // direct (non-grammar) commands replay their keys
            let keys = self.last_cmd_keys.clone();
            if !keys.is_empty() {
                self.feed_text(&keys);
            }
        }
        if let Some(text) = insert {
            let was_insert = self.mode == Mode::Insert;
            if !was_insert && !self.resolution.blocked() {
                self.enter_insert_from("i");
            }
            if replay.is_empty() {
                for character in text.chars() {
                    self.replay_insert_key(Key::Char(character));
                }
            } else {
                for step in replay {
                    self.replay_insert_step(step);
                }
            }
            self.replay_insert_key(Key::Esc);
            self.message = "repeated".into();
        }
    }

    /// vim `3rx` replaces three chars (clamped to the line end).
    pub(super) fn replace_char_n(&mut self, c: char, count: usize) {
        // count is CHARS (0017) — a byte count splits multibyte text
        let line = self.buf().line_of(self.head());
        let (s, e) = (self.buf().line_start(line), self.buf().line_end(line));
        let text_line = self.buf().text().byte_slice(s..e).to_string();
        let col = self.head().saturating_sub(s);
        let end = text_line
            .trim_end_matches('\n')
            .get(col..)
            .map(|t| {
                t.char_indices()
                    .nth(count)
                    .map(|(i, _)| s + col + i)
                    .unwrap_or(e)
            })
            .unwrap_or(e);
        if end <= self.head() || c == '\n' {
            return;
        }
        let cursor = self.head();
        let n_chars = self
            .buf()
            .text()
            .byte_slice(cursor..end)
            .to_string()
            .chars()
            .count();
        self.tx_begin();
        self.buf_mut().delete(Range::charwise(cursor, end));
        let text: String = std::iter::repeat_n(c, n_chars).collect();
        self.buf_mut().insert(cursor, &text);
        self.tx_commit();
        self.set_head(cursor + text.len() - c.len_utf8()); // last replaced char
        self.flash(Range::charwise(self.head(), self.head()));
        self.last_cmd_keys = format!("r{c}");
        self.last_change = None;
        self.last_insert = None;
    }

    /// > / < applied to every line a resolved range covers.
    pub(crate) fn apply_indent(&mut self, range: Range, right: bool) {
        let line = self.buf().line_of(range.start);
        let last = self.buf().line_of(range.end.saturating_sub(1)) + 1;
        for l in line..last {
            let start = self.buf().line_start(l);
            if right {
                // vim: an empty line gets no indent
                if self.buf().line_end(l) > start {
                    let indent = self.cur_indent().unit();
                    self.buf_mut().insert(start, &indent);
                }
            } else {
                let end = self.buf().line_end(l);
                let width = self.cur_indent().width;
                let mut strip = 0;
                while strip < width && start + strip < end && self.buf().byte(start + strip) == b' '
                {
                    strip += 1;
                }
                if strip == 0 && self.buf().byte_at(start) == Some(b'\t') {
                    strip = 1;
                }
                if strip > 0 {
                    self.buf_mut().delete(Range::charwise(start, start + strip));
                }
            }
        }
        // Neovim: a shift sets 'modified' even when no line changed
        self.buf_mut().mark_dirty();
        self.set_head(self.buf().line_start(line));
        self.clamp_cursor();
    }

    /// `p` / `P` (dot-repeatable): paste the register after/before
    /// (vim `2p` lands it count times — a block repeats horizontally).
    pub(crate) fn paste_named(&mut self, name: Option<char>, count: usize, before: bool) {
        self.paste(name, count, before);
        self.last_cmd_keys = if before { "P" } else { "p" }.into();
        self.last_change = None;
        self.last_insert = None;
    }
    /// `a`: append after the char under the cursor (multibyte-honest).
    pub(crate) fn append(&mut self) {
        self.set_head(
            self.buf()
                .ceil_boundary(self.head() + 1)
                .min(self.buf().line_end(self.buf().line_of(self.head()))),
        );
        self.enter_insert_from("a");
    }

    /// `A`: append at line end.
    pub(crate) fn append_eol(&mut self) {
        self.set_head(self.buf().line_end(self.buf().line_of(self.head())));
        self.enter_insert_from("A");
    }

    /// `o`: open a line below, auto-indented. Dot-repeat replays `o`
    /// itself, which re-derives the indent — recording it would double it.
    pub(crate) fn open_below(&mut self) {
        let Some(edits) = self.resolve_indentation(super::super::structural_indent::Trigger::Below)
        else {
            return;
        };
        self.insert_open = edits
            .iter()
            .find(|edit| edit.cursor.get() == self.head())
            .map(|edit| edit.text.clone());
        self.enter_insert_from("o");
        self.apply_indentation(&edits);
    }

    /// `O`: open above, inheriting the current line rather than deepening it.
    pub(crate) fn open_above(&mut self) {
        let Some(edits) = self.resolve_indentation(super::super::structural_indent::Trigger::Above)
        else {
            return;
        };
        self.insert_open = edits
            .iter()
            .find(|edit| edit.cursor.get() == self.head())
            .map(|edit| edit.text.clone());
        self.enter_insert_from("O");
        self.apply_indentation(&edits);
    }
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use strop_core::Buffer;

    #[test]
    fn dot_repeats_x_instead_of_an_earlier_linewise_delete() {
        let mut editor = Editor::new(Buffer::from_text("one\ntwo\nthree\nfour\n"));
        editor.feed_text("ddx");
        assert_eq!(editor.buf().text().to_string(), "wo\nthree\nfour\n");

        editor.feed_text(".");
        assert_eq!(editor.buf().text().to_string(), "o\nthree\nfour\n");
        assert_eq!(editor.head(), 0);
        assert_eq!(editor.register(None).text, "w");
    }
}
