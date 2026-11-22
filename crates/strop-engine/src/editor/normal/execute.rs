//! normal/execute.rs — the operator engine + editing entry points.

use strop_core::Range;
use strop_grammar::{self as grammar, Command, Op};

use crate::editor::Editor;

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
        super::edits::record_keys(self, cmd.keys.clone());
        Some(())
    }

    /// Run an operator command. `typed` is the command as the keys spelled
    /// it: dot-repeat keeps that form, so `.` binds marks and the last
    /// search afresh.
    pub(crate) fn execute(&mut self, typed: &Command) {
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
                } else if let Some(head) = grammar::cursor_after_refusal(self.buf(), cmd) {
                    self.set_head(head);
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
        if !matches!(op, Op::Yank | Op::Filter) && !op.missing() {
            self.dot
                .record(crate::editor::dot::Change::Command(typed.clone()));
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
            ) => column.map(|_| strop_core::id::DisplayColumn::new(usize::MAX)),
            _ => column,
        };
        crate::editor::normal::operators::apply_operator(
            self,
            Operation {
                op,
                register: cmd.register,
                shift_levels: 1,
                operands: &operands,
                reg_one: crate::editor::registers::reg_one_motion(&cmd.target),
                keys: &cmd.keys,
                column,
            },
        );
    }

    /// `[count]r<c>` replaces exactly that many characters on this line.
    pub(super) fn replace_char_n(&mut self, c: char, count: usize) {
        let cursor = self.head();
        let line_end = self.buf().line_end(self.buf().line_of(cursor));
        if cursor >= line_end || count == 0 || c == '\n' {
            return;
        }
        let rest = self.buf().text().byte_slice(cursor..line_end);
        let Some(end) = strop_core::layout::RopeGraphemes::new(rest, self.cur_indent().width)
            .nth(count - 1)
            .map(|(span, cluster)| cursor + span.byte + cluster.len())
        else {
            return;
        };
        self.tx_begin();
        self.buf_mut().delete(Range::charwise(cursor, end));
        let text = c.encode_utf8(&mut [0; 4]).repeat(count);
        self.buf_mut().insert(cursor, &text);
        self.tx_commit();
        self.set_head(cursor + text.len() - c.len_utf8()); // last replaced char
        self.flash(Range::charwise(self.head(), self.head()));
        super::edits::record_keys(self, format!("{count}r{c}"));
    }

    /// > / < applied to every line a resolved range covers.
    pub(crate) fn apply_indent(&mut self, range: Range, right: bool, levels: usize) {
        let line = self.buf().line_of(range.start);
        let last = self.buf().line_of(range.end.saturating_sub(1)) + 1;
        let width = self.cur_indent().width;
        let indent = right.then(|| self.cur_indent().units(levels));
        for l in line..last {
            let start = self.buf().line_start(l);
            if let Some(indent) = &indent {
                // vim: an empty line gets no indent
                if self.buf().line_end(l) > start {
                    self.buf_mut().insert(start, indent);
                }
            } else {
                let end = self.buf().line_end(l);
                let mut strip = 0;
                for _ in 0..levels {
                    let from = start + strip;
                    match self.buf().byte_at(from) {
                        Some(b' ') => {
                            let limit = end.min(from + width);
                            strip += (from..limit)
                                .take_while(|&at| self.buf().byte(at) == b' ')
                                .count();
                        }
                        Some(b'\t') => strip += 1,
                        _ => break,
                    }
                }
                if strip > 0 {
                    self.buf_mut().delete(Range::charwise(start, start + strip));
                }
            }
        }
        // A shift sets 'modified' even at the margin, but does not detach
        // the saved history node: undoing a shift back to disk is clean.
        self.buf_mut().mark_modified();
        self.set_head(self.buf().line_start(line));
        self.clamp_cursor();
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
