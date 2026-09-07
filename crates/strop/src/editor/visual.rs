//! Visual mode (charwise `v` and linewise `V`): motions extend the
//! selection, operators consume it. Structural composition shares the
//! Walker; text prompts (`space |`) route through the shared pending
//! owner — visual has no input state of its own (R7).

use strop_core::Range;
use strop_grammar::{self as grammar, Op};

use super::input::Action;
use super::{Editor, Key, Mode};

impl Editor {
    pub(crate) fn feed_visual(&mut self, key: Key) {
        // gv's memory: the live visual range, refreshed per key — any
        // exit path (Esc, operator, yank) leaves the last range behind
        let p = self.sels().primary();
        self.last_visual = Some((p.anchor, p.head));
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.walker.clear();
            }
            Key::Up if self.walker.is_ground() => self.run_motion("k"),
            Key::Down if self.walker.is_ground() => self.run_motion("j"),
            Key::Left if self.walker.is_ground() => self.run_motion("h"),
            Key::Right if self.walker.is_ground() => self.run_motion("l"),
            Key::Char('>') | Key::Char('<') if self.walker.is_ground() => {
                // visual indent: apply to every selected line, one undo
                // unit, back to normal (vim re-selects with gv)
                let Some(range) = self.visual_range() else {
                    return;
                };
                if self.buf().readonly {
                    self.message = "readonly buffer".into();
                    self.mode = Mode::Normal;
                    return;
                }
                let right = key == Key::Char('>');
                self.tx_begin();
                self.apply_indent(range, right);
                self.tx_commit();
                self.mode = Mode::Normal;
                self.set_head(range.start.get());
                self.clamp_cursor();
                self.flash(Range::charwise(self.head(), self.head()));
                self.last_cmd_keys = if right { "V>" } else { "V<" }.into();
                self.last_insert = None;
            }
            Key::Char('d') | Key::Char('y') | Key::Char('c') | Key::Char('x')
                if self.walker.is_ground() && self.mode == Mode::VisualBlock =>
            {
                match key {
                    Key::Char('y') => self.block_yank(),
                    Key::Char('c') => self.block_change(),
                    _ => self.block_delete(),
                }
            }
            Key::Char('I') if self.mode == Mode::VisualBlock && self.walker.is_ground() => {
                self.block_insert(false)
            }
            Key::Char('A') if self.mode == Mode::VisualBlock && self.walker.is_ground() => {
                self.block_insert(true)
            }
            Key::Char('d') | Key::Char('y') | Key::Char('c') | Key::Char('x')
                if self.walker.is_ground() =>
            {
                let op = match key {
                    Key::Char('d') | Key::Char('x') => Op::Delete,
                    Key::Char('y') => Op::Yank,
                    _ => Op::Change,
                };
                if self.buf().readonly && op != Op::Yank {
                    self.message = "readonly buffer".into();
                    self.mode = Mode::Normal;
                    return;
                }
                let Some(range) = self.visual_range() else {
                    return;
                };
                let linewise = self.mode == Mode::VisualLine;
                if op == Op::Yank {
                    let text = self.buf().slice_string(range);
                    self.set_register(
                        None,
                        if linewise {
                            super::Register::linewise(text)
                        } else {
                            super::Register::characterwise(text)
                        },
                    );
                    self.flash(range);
                } else {
                    let text = self.buf().slice_string(range);
                    let changes = crate::editor::transact::ChangeSet {
                        edits: vec![strop_core::Replacement::new(range, String::new())],
                        undo_open: false,
                    };
                    if let Err(error) = self.apply(self.current(), self.buf().revision(), changes) {
                        self.message = error.to_string();
                        return;
                    }
                    self.set_register(
                        None,
                        if linewise {
                            super::Register::linewise(text)
                        } else {
                            super::Register::characterwise(text)
                        },
                    );
                    self.set_head(range.start.get());
                    self.flash(Range::charwise(self.head(), self.head()));
                }
                self.mode = Mode::Normal;
                self.clamp_cursor();
                if op == Op::Change {
                    self.enter_insert_from(if linewise { "V..." } else { "v..." });
                }
            }
            _ => {
                let action = self.walker.feed_visual(key);
                self.dispatch_visual_action(action);
            }
        }
    }

    /// One typed visual action: motions extend, objects select, the
    /// table's leader rows (Space y, Space g h) act on the selection,
    /// `S<c>` wraps it.
    fn dispatch_visual_action(&mut self, action: Action) {
        match action {
            Action::Pending => {}
            Action::Invalid(keys) => self.message = format!("not an editor command: {keys}"),
            Action::QueryError(error) => self.message = error.to_string(),
            Action::EnterText { sigil, state } => self.begin_text_line(sigil, state),
            Action::Grammar(command) if command.op.is_none() => {
                if let grammar::Target::Object { .. } = command.target {
                    match grammar::resolve(self.buf(), self.head(), &command) {
                        Ok(Some(resolved)) => {
                            // objects select (vi[, va"): the anchor jumps
                            // to the range start, the cursor to its end —
                            // inclusive, vim semantics (0001 §5.5)
                            let head = self.head();
                            self.sels_mut()
                                .stretch_primary(resolved.range.start.get(), head);
                            self.set_head(
                                self.buf()
                                    .clamp_boundary(resolved.range.end.get().saturating_sub(1)),
                            );
                        }
                        Ok(None) => {}
                        Err(error) => self.message = error.to_string(),
                    }
                } else {
                    self.move_cursor(&command);
                }
            }
            Action::Grammar(_) => self.message = "operator needs a visual selection".into(),
            Action::Row { row, .. } if row.id == "git-file-history" => {
                // visual Space g h: history of the selected lines (0014 §4)
                let a = self.buf().line_of(self.anchor()) + 1;
                let b = self.buf().line_of(self.head()) + 1;
                self.open_line_history(a.min(b), a.max(b));
            }
            Action::Row { row, .. } if row.id == "clip-yank" => {
                // visual Space y: yank the selection to the clipboard
                if let Some(range) = self.visual_range() {
                    let linewise = self.mode == Mode::VisualLine;
                    let text = self.buf().slice_string(range);
                    self.set_register(
                        Some('+'),
                        if linewise {
                            super::Register::linewise(text)
                        } else {
                            super::Register::characterwise(text)
                        },
                    );
                    self.flash(range);
                }
                self.mode = Mode::Normal;
                self.clamp_cursor();
            }
            Action::Row { .. } => self.message = "command unavailable in visual mode".into(),
            Action::VisualSurround(c) => {
                if self.buf().readonly {
                    self.message = "readonly buffer".into();
                    return;
                }
                let Some(range) = self.visual_range() else {
                    return;
                };
                let pair = match c {
                    'b' | '(' | ')' => ('(', ')'),
                    'B' | '{' | '}' => ('{', '}'),
                    'r' | '[' | ']' => ('[', ']'),
                    'a' | '<' | '>' => ('<', '>'),
                    q => (q, q),
                };
                // two distinct boundary insertions into pre-edit
                // coordinates — one validated ChangeSet, no ordering
                // ambiguity (R8)
                let changes = crate::editor::transact::ChangeSet {
                    edits: vec![
                        strop_core::Replacement::new(
                            Range::charwise(range.start, range.start),
                            pair.0.to_string(),
                        ),
                        strop_core::Replacement::new(
                            Range::charwise(range.end, range.end),
                            pair.1.to_string(),
                        ),
                    ],
                    undo_open: false,
                };
                if let Err(error) = self.apply(self.current(), self.buf().revision(), changes) {
                    self.message = error.to_string();
                    return;
                }
                self.mode = Mode::Normal;
                self.flash(Range::charwise(
                    range.start,
                    range.end.get() + pair.0.len_utf8() + pair.1.len_utf8(),
                ));
                self.last_cmd_keys = format!("vS{c}"); // replay is visual-mode replay; approximated
                self.last_insert = None;
            }
        }
    }

    pub fn visual_range(&self) -> Option<Range> {
        match self.mode {
            Mode::Visual => {
                // charwise-inclusive spans whole CHARS (0020 §10): the
                // +1 from a multibyte lead landed mid-char and panicked
                let (s, e) = (
                    self.anchor().min(self.head()),
                    self.buf().ceil_boundary(self.anchor().max(self.head()) + 1),
                );
                Some(Range::charwise(s, e.min(self.buf().len_bytes())))
            }
            Mode::VisualLine => {
                let (a, b) = (
                    self.buf().line_of(self.anchor()),
                    self.buf().line_of(self.head()),
                );
                let (a, b) = (a.min(b), a.max(b));
                let start = self.buf().line_start(a);
                let end = if b + 1 >= self.buf().len_lines() {
                    self.buf().len_bytes()
                } else {
                    self.buf().line_start(b + 1)
                };
                Some(Range::linewise(start, end))
            }
            _ => None,
        }
    }
}
