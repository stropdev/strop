//! Visual mode (charwise `v` and linewise `V`): motions extend the
//! selection, operators consume it. The escape hatch of 0001 §2.2.

use strop_core::Range;
use strop_grammar::{self as grammar, Op, Parse};

use super::{Editor, Key, Mode};

impl Editor {
    pub(crate) fn feed_visual(&mut self, key: Key) {
        // gv's memory: the live visual range, refreshed per key — any
        // exit path (Esc, operator, yank) leaves the last range behind
        let p = self.sels().primary();
        self.last_visual = Some((p.anchor, p.head));
        // the pipe input line: `|sort<cr>` pipes the selection through
        // the command (helix's pipe) — its own tiny input state
        if self.pending.starts_with('|') {
            match key {
                Key::Esc => self.pending.clear(),
                Key::Backspace => {
                    self.pending.pop();
                    if self.pending.len() <= 1 {
                        self.pending.clear();
                    }
                }
                Key::Enter => {
                    let cmd = self.pending[1..].to_string();
                    self.pending.clear();
                    if let Some(r) = self.visual_range() {
                        self.pipe_run(r.start, r.end, &cmd);
                    }
                    self.mode = Mode::Normal;
                }
                Key::Char(c) => self.pending.push(c),
                _ => {}
            }
            return;
        }
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.pending.clear();
            }
            Key::Up => self.run_motion("k"),
            Key::Down => self.run_motion("j"),
            Key::Left => self.run_motion("h"),
            Key::Right => self.run_motion("l"),
            Key::Char('>') | Key::Char('<') if self.pending.is_empty() => {
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
                self.set_head(range.start);
                self.clamp_cursor();
                self.flash(Range::charwise(self.head(), self.head()));
                self.last_cmd_keys = if right { "V>" } else { "V<" }.into();
                self.last_insert = None;
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
                    self.tx_begin();
                    let text = self.buf_mut().delete(range);
                    self.tx_commit();
                    self.set_register(None, text, linewise);
                    self.set_head(range.start);
                    self.flash(Range::charwise(self.head(), self.head()));
                }
                self.mode = Mode::Normal;
                self.clamp_cursor();
                if op == Op::Change {
                    self.enter_insert_from(if linewise { "V..." } else { "v..." });
                }
            }
            // visual Space g h: history of the selected lines (0014 §4)
            Key::Char('g') if self.pending == " " => {
                self.pending = " g".into();
            }
            Key::Char('h') if self.pending == " g" => {
                self.pending.clear();
                let (a, b) = (
                    self.buf().line_of(self.anchor()) + 1,
                    self.buf().line_of(self.head()) + 1,
                );
                self.open_line_history(a.min(b), a.max(b));
            }
            Key::Char('y') if self.pending == " " => {
                // Space y: yank the selection to the system clipboard
                self.pending.clear();
                if let Some(range) = self.visual_range() {
                    let linewise = self.mode == Mode::VisualLine;
                    let text = self.buf().slice_string(range);
                    self.set_register(Some('+'), text, linewise);
                    self.flash(range);
                }
                self.mode = Mode::Normal;
                self.clamp_cursor();
            }
            Key::Char(c) => {
                // pipe a selection through a shell command: Space |
                // (bare | is vim's column motion, restored in 0014)
                if c == '|' && self.pending == " " {
                    self.pending = "|".into();
                    return;
                }
                if self.pending == "S" {
                    // visual S<char>: wrap the selection (sandwich)
                    self.pending.clear();
                    if let Some(range) = self.visual_range() {
                        let pair = match c {
                            'b' | '(' | ')' => ('(', ')'),
                            'B' | '{' | '}' => ('{', '}'),
                            'r' | '[' | ']' => ('[', ']'),
                            'a' | '<' | '>' => ('<', '>'),
                            q => (q, q),
                        };
                        self.tx_begin();
                        self.buf_mut().insert(range.end, &pair.1.to_string());
                        self.buf_mut().insert(range.start, &pair.0.to_string());
                        self.tx_commit();
                        self.mode = Mode::Normal;
                        self.flash(Range::charwise(range.start, range.end + 2));
                        self.last_cmd_keys = format!("vS{c}"); // replay is visual-mode replay; approximated
                        self.last_insert = None;
                    }
                    return;
                }
                self.pending.push(c);
                if self.pending == "S" || self.pending == " " || self.pending == " g" {
                    return; // surround/leader/git await their second key
                }
                if matches!(grammar::parse(&self.pending), Parse::Invalid) {
                    // invalid keys never squat in pending (visual had no
                    // clear — `>x` used to swallow every later key)
                    self.message = format!("not an editor command: {}", self.pending);
                    self.pending.clear();
                    return;
                }
                if let Parse::Complete(cmd) = grammar::parse(&self.pending) {
                    if cmd.op.is_none() {
                        self.pending.clear();
                        // objects in visual mode select the object (vi[, va"):
                        // the anchor jumps to the range start, the cursor to
                        // its end — inclusive, vim semantics (0001 §5.5)
                        if let grammar::Target::Object { .. } = cmd.target {
                            if let Some(r) = grammar::resolve(self.buf(), self.head(), &cmd) {
                                let h = self.head();
                                self.sels_mut().stretch_primary(r.range.start, h);
                                self.set_head(r.range.end.saturating_sub(1));
                            }
                        } else {
                            self.move_cursor(&cmd);
                        }
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
                    self.anchor().min(self.head()),
                    self.anchor().max(self.head()) + 1,
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
