//! normal/pending.rs — the modal text lines (: / ? |) — 0003 §1.

use strop_grammar::{self as grammar, Parse};

use crate::editor::{Editor, Key};

impl Editor {
    pub(super) fn feed_pending(&mut self, key: Key) {
        let is_ex = self.pending.starts_with(':');
        let is_pipe = self.pending.starts_with('|');
        let is_search = !is_ex && (self.pending.contains('/') || self.pending.contains('?'));
        match key {
            Key::Esc => {
                // rootle's boxes: Esc enters normal mode on the line,
                // Esc again clears it
                if self.pending_normal {
                    self.pending.clear();
                    self.pending_normal = false;
                } else {
                    self.pending_normal = true;
                    self.pending_cursor = self.pending.len();
                }
            }
            Key::Backspace => {
                if self.pending_normal {
                    self.pending_normal_key('h'); // vim: bs in normal = h
                } else {
                    self.pending.pop();
                    self.pending_cursor = self.pending.len();
                }
            }
            Key::Enter if is_ex => self.run_ex(),
            Key::Enter if is_search => {
                // vim: an empty / repeats the last search in its
                // direction; an empty ? reverses it
                let pat = &self.pending[1..];
                if pat.is_empty() {
                    let reversed = self.pending.starts_with('?');
                    self.pending.clear();
                    return self.repeat_search(reversed);
                }
                self.pending.push('\r');
                self.resolve_pending();
            }
            Key::Enter if is_pipe => self.pipe_current_line(),
            Key::Tab if is_ex => self.ex_tab_complete(),
            Key::Enter => self.pending.clear(),
            Key::CtrlR | Key::CtrlW | Key::CtrlX | Key::CtrlD | Key::CtrlO => {} // pending + window/undo keys: no-op
            Key::CtrlU | Key::CtrlF | Key::CtrlB | Key::CtrlV | Key::CtrlCaret => {}
            Key::Up | Key::Down | Key::Left | Key::Right | Key::Tab | Key::Backtab => {}
            Key::Char(c) => {
                // modal editing on the input line (0003 §1)
                if self.pending_normal {
                    self.pending_normal_key(c);
                    return;
                }
                self.pending.push(c);
                if !is_ex && !is_pipe {
                    self.resolve_pending();
                }
            }
        }
    }

    /// One normal-mode key on the pending input line — the picker and
    /// the ex line share strop_picker::LineEdit for this (0003 §1).
    fn pending_normal_key(&mut self, c: char) {
        let mut le = strop_picker::LineEdit::new(std::mem::take(&mut self.pending));
        le.cursor = self.pending_cursor;
        le.normal = true;
        le.normal_key(c);
        self.pending_normal = le.normal;
        self.pending_cursor = le.cursor;
        self.pending = le.text;
    }

    fn resolve_pending(&mut self) {
        match grammar::parse(&self.pending) {
            Parse::Incomplete => {}
            Parse::Invalid => {
                self.message = format!("not an editor command: {}", self.pending);
                self.pending.clear();
            }
            Parse::Complete(cmd) => {
                self.pending.clear();
                match cmd.op {
                    None => self.move_cursor(&cmd),
                    Some(_) => self.execute(&cmd),
                }
            }
        }
    }

    /// vim Enter: [count] lines down, first non-blank. With the blame
    /// gutter on, Enter dives into the line's commit instead (0011 §3).
    pub fn enter_pub(&mut self) {
        if self.dive_from_blame() {
            return;
        }
        let n = self.walker.state.count1.unwrap_or(1);
        let line = (self.buf().line_of(self.head()) + n).min(self.buf().last_content_line());
        let s = self.buf().line_start(line);
        let e = self.buf().line_end(line);
        let mut p = s;
        while p < e
            && self
                .buf()
                .byte_at(p)
                .is_some_and(|b| b == b' ' || b == b'\t')
        {
            p += 1;
        }
        self.set_head(p.min(e));
        self.clamp_cursor();
    }

    pub(crate) fn run_motion(&mut self, keys: &str) {
        if let Parse::Complete(cmd) = grammar::parse(keys) {
            self.move_cursor(&cmd);
        }
    }

    /// `|cmd` in normal mode: pipe the current line through cmd
    /// (helix's pipe — the better `!`).
    fn pipe_current_line(&mut self) {
        let cmd = self.pending[1..].to_string();
        self.pending.clear();
        let line = self.buf().line_of(self.head());
        let start = self.buf().line_start(line);
        let end = self.buf().line_start(line + 1).min(self.buf().len_bytes());
        self.pipe_run(start, end, &cmd);
    }
}
