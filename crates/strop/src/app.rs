//! The editor state machine: modes, pending keys, registers, flash.
//! One input path for TUI and headless — both call `feed`.

use std::time::{Duration, Instant};

use strop_core::{Buffer, Range};
use strop_grammar::{self as grammar, Command, Op, Parse, Resolved};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
}

impl Mode {
    pub fn chip(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Esc,
    Enter,
    Backspace,
}

pub const FLASH_FOR: Duration = Duration::from_millis(280);

pub struct Editor {
    pub buf: Buffer,
    pub mode: Mode,
    pub cursor: usize,
    pub pending: String,
    pub anchor: usize,
    pub register: String,
    pub register_linewise: bool,
    pub flash: Option<(Range, Instant)>,
    pub message: String,
    pub should_quit: bool,
    pub view_top: usize,
    /// Dot-repeat: the command keys, plus inserted text if it was a change.
    last_cmd_keys: String,
    last_insert: Option<String>,
    recording_insert: Option<String>,
}

impl Editor {
    pub fn new(buf: Buffer) -> Self {
        Self {
            buf,
            mode: Mode::Normal,
            cursor: 0,
            pending: String::new(),
            anchor: 0,
            register: String::new(),
            register_linewise: false,
            flash: None,
            message: String::new(),
            should_quit: false,
            view_top: 0,
            last_cmd_keys: String::new(),
            last_insert: None,
            recording_insert: None,
        }
    }

    pub fn feed_text(&mut self, s: &str) {
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            let key = match c {
                '\x1b' => Key::Esc,
                '\r' | '\n' => Key::Enter,
                '\x7f' => Key::Backspace,
                '<' => {
                    // token form: <esc> <cr> <bs>
                    let rest: String = chars.by_ref().take_while(|&c| c != '>').collect();
                    match rest.to_ascii_lowercase().as_str() {
                        "esc" => Key::Esc,
                        "cr" | "enter" => Key::Enter,
                        "bs" => Key::Backspace,
                        _ => {
                            self.feed(Key::Char('<'));
                            for c in rest.chars().chain(std::iter::once('>')) {
                                self.feed(Key::Char(c));
                            }
                            continue;
                        }
                    }
                }
                c => Key::Char(c),
            };
            self.feed(key);
        }
    }

    pub fn feed(&mut self, key: Key) {
        self.message.clear();
        match self.mode {
            Mode::Insert => self.feed_insert(key),
            Mode::Visual => self.feed_visual(key),
            Mode::Normal => self.feed_normal(key),
        }
    }

    // ---- normal -------------------------------------------------------

    fn feed_normal(&mut self, key: Key) {
        if !self.pending.is_empty() {
            return self.feed_pending(key);
        }
        match key {
            Key::Char(c) => match c {
                '1'..='9' => self.pending.push(c),
                '0' => self.run_motion("0"),
                'h' | 'j' | 'k' | 'l' | 'w' | 'b' | 'e' | '$' | 'G' => {
                    self.run_motion(&c.to_string())
                }
                'g' | 'd' | 'y' | 'c' | 'f' | 'F' | 't' | 'T' | '/' | ':' => self.pending.push(c),
                'i' => self.enter_insert(false),
                'a' => {
                    self.cursor =
                        (self.cursor + 1).min(self.buf.line_end(self.buf.line_of(self.cursor)));
                    self.enter_insert(false);
                }
                'A' => {
                    self.cursor = self.buf.line_end(self.buf.line_of(self.cursor));
                    self.enter_insert(false);
                }
                'o' => {
                    let end = self.buf.line_end(self.buf.line_of(self.cursor));
                    self.buf.insert(end, "\n");
                    self.cursor = end + 1;
                    self.enter_insert(false);
                }
                'O' => {
                    let start = self.buf.line_start(self.buf.line_of(self.cursor));
                    self.buf.insert(start, "\n");
                    self.cursor = start;
                    self.enter_insert(false);
                }
                'x' => {
                    let end =
                        (self.cursor + 1).min(self.buf.line_end(self.buf.line_of(self.cursor)));
                    if end > self.cursor {
                        let range = Range::charwise(self.cursor, end);
                        self.register = self.buf.delete(range);
                        self.register_linewise = false;
                        self.flash(range);
                        self.last_cmd_keys = "x".into();
                        self.last_insert = None;
                    }
                }
                'p' => self.paste(),
                'v' => {
                    self.mode = Mode::Visual;
                    self.anchor = self.cursor;
                }
                '.' => self.dot_repeat(),
                'u' => self.message = "undo arrives with the undo tree (M4)".into(),
                _ => {}
            },
            Key::Esc | Key::Enter | Key::Backspace => {}
        }
    }

    fn feed_pending(&mut self, key: Key) {
        let is_ex = self.pending.starts_with(':');
        let is_search = !is_ex && self.pending.contains('/');
        match key {
            Key::Esc => {
                self.pending.clear();
            }
            Key::Backspace => {
                self.pending.pop();
            }
            Key::Enter if is_ex => self.run_ex(),
            Key::Enter if is_search => {
                self.pending.push('\r');
                self.resolve_pending();
            }
            Key::Enter => {
                self.pending.clear();
            }
            Key::Char(c) => {
                self.pending.push(c);
                if !is_ex {
                    self.resolve_pending();
                }
            }
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

    fn run_motion(&mut self, keys: &str) {
        if let Parse::Complete(cmd) = grammar::parse(keys) {
            self.move_cursor(&cmd);
        }
    }

    fn move_cursor(&mut self, cmd: &Command) {
        if let Some(r) = grammar::resolve(&self.buf, self.cursor, cmd) {
            self.cursor = grammar::cursor_after(&self.buf, self.cursor, cmd, &r);
            self.clamp_cursor();
        }
    }

    /// The live preview: what would the pending keys do right now?
    /// Same resolver the executor uses — the preview cannot lie.
    pub fn preview(&self) -> Option<Resolved> {
        if self.pending.is_empty() {
            return None;
        }
        match grammar::parse(&self.pending) {
            Parse::Complete(cmd) if cmd.op.is_some() => {
                grammar::resolve(&self.buf, self.cursor, &cmd)
            }
            _ => {
                // partial search: d/foo mid-typing previews cursor→first match
                if let Some(idx) = self.pending.find('/') {
                    let pat = &self.pending[idx + 1..];
                    if !pat.is_empty() {
                        if let Some(hit) = grammar::search_forward(&self.buf, self.cursor + 1, pat)
                        {
                            return Some(Resolved {
                                range: Range::charwise(self.cursor, hit),
                                inclusive: false,
                                spec: format!("search /{pat}"),
                            });
                        }
                    }
                }
                None
            }
        }
    }

    /// Pending f/F/t/T awaiting its char: the leap-style candidates.
    pub fn find_candidates(&self) -> Option<(u8, bool)> {
        let b = self.pending.as_bytes();
        let (&pfx, _) = b.split_last()?;
        let backward = matches!(pfx, b'F' | b'T');
        if !matches!(pfx, b'f' | b'F' | b't' | b'T') {
            return None;
        }
        Some((pfx, backward))
    }

    /// Pending search pattern (incsearch highlight), if any.
    pub fn search_pattern(&self) -> Option<&str> {
        self.pending
            .find('/')
            .map(|i| &self.pending[i + 1..])
            .filter(|p| !p.is_empty())
    }

    fn execute(&mut self, cmd: &Command) {
        let Some(r) = grammar::resolve(&self.buf, self.cursor, cmd) else {
            self.message = "no target".into();
            return;
        };
        match cmd.op.unwrap() {
            Op::Yank => {
                self.register = self.buf.slice_string(r.range);
                self.register_linewise = r.range.linewise;
                self.flash(r.range);
            }
            Op::Delete | Op::Change => {
                self.register = self.buf.delete(r.range);
                self.register_linewise = r.range.linewise;
                self.cursor = r.range.start;
                self.clamp_cursor();
                self.flash(Range::charwise(self.cursor, self.cursor));
                if cmd.op.unwrap() == Op::Change {
                    self.enter_insert(true);
                }
            }
        }
        self.last_cmd_keys = cmd.keys.clone();
        self.last_insert = None;
    }

    fn dot_repeat(&mut self) {
        if self.last_cmd_keys.is_empty() {
            return;
        }
        let keys = self.last_cmd_keys.clone();
        let insert = self.last_insert.clone();
        self.feed_text(&keys);
        if let Some(text) = insert {
            for c in text.chars() {
                self.feed(Key::Char(c));
            }
            self.feed(Key::Esc);
            self.message = "repeated".into();
        }
    }

    fn paste(&mut self) {
        if self.register.is_empty() {
            return;
        }
        if self.register_linewise {
            let at = self.buf.line_start(self.buf.line_of(self.cursor) + 1);
            self.buf
                .insert(at.min(self.buf.len_bytes()), &self.register);
            self.cursor = at.min(self.buf.len_bytes());
        } else {
            let at = (self.cursor + 1).min(self.buf.len_bytes());
            self.buf.insert(at, &self.register);
            self.cursor = at;
        }
        self.clamp_cursor();
    }

    fn run_ex(&mut self) {
        let cmdline = self
            .pending
            .trim_start_matches(':')
            .trim_end_matches('\r')
            .to_string();
        self.pending.clear();
        match cmdline.as_str() {
            "w" => match self.buf.save() {
                Ok(()) => self.message = "written".into(),
                Err(e) => self.message = format!("write failed: {e}"),
            },
            "q" => self.should_quit = true,
            "wq" => {
                let _ = self.buf.save();
                self.should_quit = true;
            }
            other => self.message = format!("unknown ex: :{other}"),
        }
    }

    // ---- visual -------------------------------------------------------

    fn feed_visual(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.pending.clear();
            }
            Key::Char('d') | Key::Char('y') | Key::Char('c') if self.pending.is_empty() => {
                let op = match key {
                    Key::Char('d') => Op::Delete,
                    Key::Char('y') => Op::Yank,
                    _ => Op::Change,
                };
                let (s, e) = (
                    self.anchor.min(self.cursor),
                    self.anchor.max(self.cursor) + 1,
                );
                let range = Range::charwise(s, e.min(self.buf.len_bytes()));
                self.register = if op == Op::Yank {
                    self.buf.slice_string(range)
                } else {
                    self.buf.delete(range)
                };
                self.register_linewise = false;
                self.cursor = range.start;
                self.mode = Mode::Normal;
                self.clamp_cursor();
                self.flash(Range::charwise(self.cursor, self.cursor));
                if op == Op::Change {
                    self.enter_insert(true);
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
        if self.mode != Mode::Visual {
            return None;
        }
        let (s, e) = (
            self.anchor.min(self.cursor),
            self.anchor.max(self.cursor) + 1,
        );
        Some(Range::charwise(s, e.min(self.buf.len_bytes())))
    }

    // ---- insert -------------------------------------------------------

    fn enter_insert(&mut self, record: bool) {
        self.mode = Mode::Insert;
        if record {
            self.recording_insert = Some(String::new());
        }
    }

    fn feed_insert(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.cursor = self.cursor.saturating_sub(1);
                self.clamp_cursor();
                if let Some(rec) = self.recording_insert.take() {
                    self.last_insert = Some(rec);
                }
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    let prev = self.cursor - 1;
                    self.buf.delete(Range::charwise(prev, self.cursor));
                    self.cursor = prev;
                    if let Some(rec) = &mut self.recording_insert {
                        rec.pop();
                    }
                }
            }
            Key::Enter => {
                self.buf.insert(self.cursor, "\n");
                self.cursor += 1;
                if let Some(rec) = &mut self.recording_insert {
                    rec.push('\n');
                }
            }
            Key::Char(c) => {
                let mut tmp = [0u8; 4];
                self.buf.insert(self.cursor, c.encode_utf8(&mut tmp));
                self.cursor += c.len_utf8();
                if let Some(rec) = &mut self.recording_insert {
                    rec.push(c);
                }
            }
        }
    }

    // ---- shared --------------------------------------------------------

    fn flash(&mut self, range: Range) {
        self.flash = Some((range, Instant::now()));
    }

    pub fn flash_range(&self) -> Option<Range> {
        self.flash
            .and_then(|(r, at)| (at.elapsed() < FLASH_FOR).then_some(r))
    }

    pub fn clamp_cursor(&mut self) {
        let line = self.buf.line_of(self.cursor);
        let start = self.buf.line_start(line);
        let end = self.buf.line_end(line);
        let max = if self.mode == Mode::Insert {
            end
        } else {
            end.max(start + 1) - 1
        };
        self.cursor = self
            .buf
            .clamp_boundary(self.cursor.clamp(start, max.max(start)));
    }

    /// Keep the cursor on screen; `rows` = text area height.
    pub fn scroll_to_cursor(&mut self, rows: usize) {
        let line = self.buf.line_of(self.cursor);
        if line < self.view_top {
            self.view_top = line;
        } else if line >= self.view_top + rows {
            self.view_top = line + 1 - rows;
        }
    }
}
