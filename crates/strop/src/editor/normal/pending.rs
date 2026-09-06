//! normal/pending.rs — the modal text lines (: / ? |) — 0003 §1.

use strop_grammar::{self as grammar, Parse};

use crate::editor::{Editor, Key};

enum LineInput {
    Insert(char),
    Backspace,
}

impl Editor {
    pub(super) fn feed_pending(&mut self, key: Key) {
        let sigil = self.pending_sigil();
        // synthetic compositions (`"+y` + motion, set by Space y/p)
        // resolve per keystroke like the old free lines did — only
        // the : and | sigils opt out
        let is_ex = sigil == Some(':');
        let is_pipe = sigil == Some('|');
        // a search under composition: a / or ? sigil line, or a
        // synthetic line whose motion is a search (` y/foo⏎)
        let is_search = !is_ex && !is_pipe && self.pending.contains(['/', '?']);
        match key {
            Key::Esc => {
                // rootle's boxes: Esc enters normal mode on the line,
                // Esc again clears it — and clearing a search aborts
                // it: the cursor returns to where `/` opened (vim)
                if self.pending_normal {
                    self.abort_search_line();
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
                    if is_search {
                        self.incsearch_jump();
                    }
                } else {
                    self.pending_insert_key(LineInput::Backspace);
                    if self.pending.is_empty() {
                        self.abort_search_line();
                    }
                    if is_search {
                        self.incsearch_jump();
                    }
                }
            }
            Key::Enter if sigil == Some(':') => self.run_ex(),
            Key::Enter if is_search => {
                // vim: an empty / repeats the last search in its
                // direction; an empty ? reverses it
                if matches!(sigil, Some('/' | '?')) && self.pending.len() == 1 {
                    let reversed = sigil == Some('?');
                    self.abort_search_line();
                    self.pending.clear();
                    return self.repeat_search(reversed);
                }
                // Enter commits from the origin: incsearch already
                // sits on this match — resolving from the jumped
                // cursor would skip to the next one
                self.abort_search_line();
                self.pending.push('\r');
                self.resolve_pending();
            }
            Key::Enter if sigil == Some('|') => self.pipe_current_line(),
            Key::Tab if sigil == Some(':') => self.ex_tab_complete(),
            Key::Enter => self.pending.clear(),
            Key::CtrlL => self.needs_repaint = true, // desync recovery
            Key::CtrlR | Key::CtrlW | Key::CtrlX | Key::CtrlD | Key::CtrlO => {} // pending + window/undo keys: no-op
            Key::CtrlU | Key::CtrlF | Key::CtrlB | Key::CtrlV | Key::CtrlCaret => {}
            Key::Up | Key::Down | Key::Left | Key::Right | Key::Tab | Key::Backtab => {}
            Key::Char(c) => {
                // modal editing on the input line (0003 §1)
                if self.pending_normal {
                    self.pending_normal_key(c);
                    if is_search {
                        self.incsearch_jump();
                    }
                    return;
                }
                self.pending_insert_key(LineInput::Insert(c));
                if is_search {
                    // live incsearch (vim parity): the cursor tracks
                    // the pattern's first match from the fixed origin
                    // — typing AND deleting re-resolve it
                    self.incsearch_jump();
                } else if !is_ex && !is_pipe {
                    // synthetic composition (`"+y + motion): the
                    // grammar completes as the motion types
                    self.resolve_pending();
                }
            }
        }
    }

    /// Apply an insert-phase edit using the same caret mechanics as picker fields.
    fn pending_insert_key(&mut self, key: LineInput) {
        let sigil = self.pending_sigil();
        let mut line = strop_picker::LineEdit::new(std::mem::take(&mut self.pending));
        line.cursor = if sigil.is_some() {
            self.pending_cursor.min(line.text.len())
        } else {
            line.text.len()
        };
        match key {
            LineInput::Insert(character) => line.insert_char(character),
            LineInput::Backspace => line.backspace(),
        }
        self.pending_cursor = line.cursor;
        self.pending = line.text;
        if sigil.is_some() && self.pending_sigil() != sigil {
            self.pending.clear();
            self.pending_cursor = 0;
            self.abort_search_line();
        }
    }

    fn pending_normal_key(&mut self, c: char) {
        let sigil = self.pending_sigil();
        let mut le = strop_picker::LineEdit::new(std::mem::take(&mut self.pending));
        le.cursor = self.pending_cursor;
        le.normal = true;
        le.normal_key(c);
        self.pending_normal = le.normal;
        self.pending_cursor = le.cursor;
        self.pending = le.text;
        if self.pending.is_empty() || (sigil.is_some() && self.pending_sigil() != sigil) {
            self.pending.clear();
            self.pending_cursor = 0;
            self.abort_search_line();
        }
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
