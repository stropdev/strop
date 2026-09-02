//! Visual mode (charwise `v` and linewise `V`): motions extend the
//! selection, operators consume it. The escape hatch of 0001 §2.2.

use strop_core::Range;
use strop_grammar::{self as grammar, Op, Parse};

use super::{Editor, Key, Mode};

impl Editor {
    pub(crate) fn feed_visual(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.pending.clear();
            }
            Key::Char('d') | Key::Char('y') | Key::Char('c') | Key::Char('x')
                if self.pending.is_empty() =>
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
                    self.set_register(None, text, linewise);
                    self.flash(range);
                } else {
                    let text = self.buf_mut().delete(range);
                    self.set_register(None, text, linewise);
                    self.cursor = range.start;
                    self.flash(Range::charwise(self.cursor, self.cursor));
                }
                self.mode = Mode::Normal;
                self.clamp_cursor();
                if op == Op::Change {
                    self.enter_insert_from(if linewise { "V..." } else { "v..." });
                }
            }
            Key::Char(c) => {
                self.pending.push(c);
                if let Parse::Complete(cmd) = grammar::parse(&self.pending) {
                    if cmd.op.is_none() {
                        self.pending.clear();
                        self.move_cursor(&cmd);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn visual_range(&self) -> Option<Range> {
        match self.mode {
            Mode::Visual => {
                let (s, e) = (
                    self.anchor.min(self.cursor),
                    self.anchor.max(self.cursor) + 1,
                );
                Some(Range::charwise(s, e.min(self.buf().len_bytes())))
            }
            Mode::VisualLine => {
                let (a, b) = (
                    self.buf().line_of(self.anchor),
                    self.buf().line_of(self.cursor),
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
