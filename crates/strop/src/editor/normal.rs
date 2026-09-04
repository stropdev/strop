//! Normal mode: the grammar's home. Operators, motions, counts,
//! registers, dot-repeat, the ex-line — and the live preview query.

use strop_core::Range;
use strop_grammar::{self as grammar, Command, Op, Parse, Resolved};

use super::{Editor, Key, Mode};

/// The ex vocabulary (completion + `run_ex` dispatch reads the same
/// list — one table, no drift).
pub(crate) const EX_COMMANDS: &[(&str, &str)] = &[
    ("w", "write"),
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
];

impl Editor {
    pub(crate) fn feed_normal(&mut self, key: Key) {
        // readonly surfaces (git browser/blame/etc.): q closes, Enter
        // dives, motions and yank fall through, edits refuse (0001 §3)
        if self.buf().readonly {
            return self.feed_readonly(key);
        }
        if key == Key::CtrlR {
            return self.redo();
        }
        if key == Key::CtrlW {
            self.pending = "\x17".into(); // ctrl-w prefix
            return;
        }
        if !self.pending.is_empty() {
            return self.feed_pending(key);
        }
        // Enter with the blame gutter on dives into the line's commit
        // (0011 §3); with it off, Enter stays inert in normal mode
        if key == Key::Enter {
            return self.dive_from_blame();
        }
        // ctrl-o / ctrl-i (Tab) walk the jumplist (vim; terminal ctrl-i
        // *is* Tab)
        if key == Key::CtrlO {
            return self.jump_back();
        }
        if key == Key::Tab {
            return self.jump_forward();
        }
        // Esc in normal mode: collapse to the primary cursor (0013 §3)
        if key == Key::Esc {
            self.collapse_cursors();
            return;
        }
        // arrows speak hjkl (never dropped at the translation layer)
        match key {
            Key::Up => return self.run_motion("k"),
            Key::Down => return self.run_motion("j"),
            Key::Left => return self.run_motion("h"),
            Key::Right => return self.run_motion("l"),
            _ => {}
        }
        let Key::Char(c) = key else {
            return;
        };
        match c {
            '1'..='9' => self.pending.push(c),
            // 0 starts a count only as its first digit (vim: 30j works)
            '0' if self.pending.chars().all(|c| c.is_ascii_digit()) && !self.pending.is_empty() => {
                self.pending.push('0')
            }
            // bare 0 is the line-start motion (vim); only a digit after
            // a count continues the count
            '0' => self.run_motion("0"),
            'h' | 'j' | 'k' | 'l' | 'w' | 'b' | 'e' | 'W' | 'B' | 'E' | '$' | 'G' | '%' | '^' => {
                self.run_motion(&c.to_string())
            }
            'g' | 'd' | 'y' | 'c' | 'f' | 'F' | 't' | 'T' | '/' | '?' | ':' | '"' | 'r' | '>'
            | '<' | ' ' | '[' | ']' | 'm' | '\'' | '`' | '|' => self.pending.push(c),
            // n/N replay the last search (vim; N inverts direction)
            'n' => self.repeat_search(false),
            'N' => self.repeat_search(true),
            // aliases — dot-repeat replays the alias key itself
            'D' => self.alias("D", "d$"),
            'C' => self.alias("C", "c$"),
            'Y' => self.alias("Y", "yy"),
            's' => self.alias("s", "cl"),
            'X' => self.alias("X", "dh"),
            'i' => self.enter_insert_from("i"),
            'a' => {
                // append after the char under the cursor — its end, not
                // one raw byte in (multibyte)
                self.cursor = self
                    .buf()
                    .ceil_boundary(self.cursor + 1)
                    .min(self.buf().line_end(self.buf().line_of(self.cursor)));
                self.enter_insert_from("a");
            }
            'A' => {
                self.cursor = self.buf().line_end(self.buf().line_of(self.cursor));
                self.enter_insert_from("A");
            }
            'o' => {
                let indent = self.auto_indent_full_line();
                let end = self.buf().line_end(self.buf().line_of(self.cursor));
                let text = format!("\n{indent}");
                self.insert_open = Some(text.clone());
                self.buf_mut().insert(end, &text);
                self.cursor = end + text.len();
                self.enter_insert_from("o");
                // dot-repeat replays 'o', which re-derives the indent —
                // recording it too would double it
            }
            'O' => {
                let indent = self.auto_indent_full_line();
                let start = self.buf().line_start(self.buf().line_of(self.cursor));
                let text = format!("{indent}\n");
                self.insert_open = Some(text.clone());
                self.buf_mut().insert(start, &text);
                self.cursor = start + indent.len();
                self.enter_insert_from("O");
                // same as 'o': indent is derived, not recorded
            }
            'x' => {
                // the whole char under the cursor — a bare +1 splits
                // multibyte chars and panics ropey's byte_slice
                let end = self
                    .buf()
                    .ceil_boundary(self.cursor + 1)
                    .min(self.buf().line_end(self.buf().line_of(self.cursor)));
                if end > self.cursor {
                    self.tx_begin();
                    let range = Range::charwise(self.cursor, end);
                    let text = self.buf_mut().delete(range);
                    self.tx_commit();
                    self.set_register(None, text, false);
                    self.flash(range);
                    self.last_cmd_keys = "x".into();
                    self.last_insert = None;
                }
            }
            'p' => {
                self.paste(None, false);
                self.last_cmd_keys = "p".into();
                self.last_insert = None;
            }
            'P' => {
                self.paste(None, true);
                self.last_cmd_keys = "P".into();
                self.last_insert = None;
            }
            'v' => {
                // v1: visual mode is primary-only — extras collapse (0013)
                self.extra_cursors.clear();
                self.mode = Mode::Visual;
                self.anchor = self.cursor;
            }
            'V' => {
                self.extra_cursors.clear();
                self.mode = Mode::VisualLine;
                self.anchor = self.cursor;
            }
            'Q' => self.toggle_cursor(),
            'J' => self.join_lines(),
            '.' => self.dot_repeat(),
            'u' => self.undo(),
            ';' => self.repeat_find(false),
            ',' => self.repeat_find(true),
            '*' => self.search_word_under_cursor(false),
            '#' => self.search_word_under_cursor(true),
            '~' => self.toggle_case(),
            'S' => self.alias("S", "cc"),
            'I' => self.alias("I", "^i"),
            // unknown keys say so (the `gI` path) — never silent
            _ => self.message = format!("not an editor command: {c}"),
        }
    }

    /// ds" / cs"' / ysiw" (sandwich lineage). Returns Some when the
    /// command was a surround op and got handled.
    fn execute_surround(&mut self, cmd: &Command) -> Option<()> {
        let r = grammar::resolve(self.buf(), self.cursor, cmd)?;
        let pair = |ch: u8| match ch {
            b'b' | b'(' | b')' => (b'(', b')'),
            b'B' | b'{' | b'}' => (b'{', b'}'),
            b'r' | b'[' | b']' => (b'[', b']'),
            b'a' | b'<' | b'>' => (b'<', b'>'),
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
                self.cursor = r.range.start;
            }
            grammar::Target::SurroundChange { to, .. } => {
                let (o, c) = pair(*to);
                self.buf_mut()
                    .delete(Range::charwise(r.range.end - 1, r.range.end));
                self.buf_mut()
                    .insert(r.range.end - 1, &(c as char).to_string());
                self.buf_mut()
                    .delete(Range::charwise(r.range.start, r.range.start + 1));
                self.buf_mut()
                    .insert(r.range.start, &(o as char).to_string());
                self.cursor = r.range.start;
            }
            grammar::Target::SurroundAdd { ch, .. } => {
                let (o, c) = pair(*ch);
                self.buf_mut().insert(r.range.end, &(c as char).to_string());
                self.buf_mut()
                    .insert(r.range.start, &(o as char).to_string());
                self.cursor = r.range.start + 1;
            }
            _ => {
                self.tx_commit();
                return None;
            }
        }
        self.tx_commit();
        self.clamp_cursor();
        self.flash(Range::charwise(self.cursor, self.cursor));
        self.last_cmd_keys = cmd.keys.clone();
        self.last_insert = None;
        Some(())
    }

    /// Alias keys (D → d$, …): execute the expansion, remember the alias
    /// so dot-repeat replays through the same path.
    fn alias(&mut self, alias_key: &str, expansion: &str) {
        self.feed_text(expansion);
        self.last_cmd_keys = alias_key.into();
    }

    fn feed_pending(&mut self, key: Key) {
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
            Key::Up | Key::Down | Key::Left | Key::Right | Key::Tab | Key::Backtab => {}
            Key::Char(c) => {
                // modal editing on the input line (0003 §1)
                if self.pending_normal {
                    self.pending_normal_key(c);
                    return;
                }
                // count + non-operator command: vim multiplies (2x, 3p,
                // 2u, 4.…). The count never aborts the command.
                if self.pending.bytes().all(|b| b.is_ascii_digit()) {
                    let count: usize = self.pending.parse().unwrap_or(1);
                    match c {
                        'p' | 'P' => {
                            // vim 2p: the register lands twice at one
                            // spot, not paste-advance-paste
                            self.pending.clear();
                            return self.paste_n(count, c == 'P');
                        }
                        'x' | 'X' | 'J' | 's' | '~' | '^' | 'u' | '.' | 'n' | 'N' => {
                            self.pending.clear();
                            for _ in 0..count {
                                self.feed_normal(Key::Char(c));
                            }
                            return;
                        }
                        'i' | 'a' | 'A' | 'o' | 'O' | 'I' => {
                            // vim: the inserted text repeats <count> times
                            self.pending.clear();
                            self.insert_count = count;
                            return self.feed_normal(Key::Char(c));
                        }
                        'r' => {
                            self.pending.push('r'); // count rides to the char
                            return;
                        }
                        'D' | 'C' | 'Y' => {
                            // the alias once — count on these is rare and
                            // d$/c$/yy re-runs are inert at line end
                            self.pending.clear();
                            return self.feed_normal(Key::Char(c));
                        }
                        _ => {} // operators/motions: grammar resolves counts
                    }
                }
                // window commands (C-w): h l j k w move, v s split
                if self.pending == "\x17" {
                    self.pending.clear();
                    return match c {
                        'h' | 'l' | 'j' | 'k' | 'w' => self.pane_move(c),
                        'v' => self.split(true, None),
                        's' => self.split(false, None),
                        'q' => self.close_pane_or_buffer(false),
                        _ => self.message = "C-w: h l j k w move · v s split · q close".into(),
                    };
                }
                // Space leader (0003 §2): one namespace, which-key overlay
                if self.pending == " " {
                    self.pending.clear();
                    return match c {
                        'c' => self.add_cursor_next_line(),
                        'f' => self.open_picker(strop_picker::Kind::Files),
                        'b' => self.open_picker(strop_picker::Kind::Buffers),
                        '/' => self.open_picker(strop_picker::Kind::Grep),
                        'R' => self.open_picker(strop_picker::Kind::Replace),
                        'g' => {
                            self.pending = " g".into();
                        }
                        '?' => self.open_help(),
                        'u' => self.open_undo_tree(),
                        'd' => self.open_diagnostics_picker(),
                        'k' => self.lsp_hover(),
                        // system clipboard via the `+` register (helix's
                        // space y/p, vim's "+ machinery underneath)
                        'y' => {
                            self.pending = "\"+y".into();
                        }
                        'p' => self.clipboard_paste(false),
                        'P' => self.clipboard_paste(true),
                        _ => {
                            self.message =
                                "Space: f files · b buffers · / grep · y/p clipboard · g git".into()
                        }
                    };
                }
                // git namespace (0003 §4): working-surface verbs (M2)
                if self.pending == " g" {
                    return self.feed_git_pending(c);
                }
                // gd: goto definition (LSP)
                if self.pending == "g" && c == 'd' {
                    self.pending.clear();
                    return self.lsp_goto_definition();
                }
                // gs: switch source ↔ header (clangd extension)
                if self.pending == "g" && c == 's' {
                    self.pending.clear();
                    return self.lsp_switch_source_header();
                }
                // hunk motions (0001 pillar 3.1)
                if (self.pending == "]" || self.pending == "[") && c == 'c' {
                    let forward = self.pending == "]";
                    self.pending.clear();
                    return self.jump_hunk(forward);
                }
                // marks: m<a> sets, '<a> jumps
                if self.pending == "m" && c.is_ascii_lowercase() {
                    self.pending.clear();
                    return self.set_mark(c);
                }
                if (self.pending == "'" || self.pending == "`") && c.is_ascii_lowercase() {
                    self.pending.clear();
                    return self.jump_mark(c);
                }
                // r<char>: replace the char under the cursor, stay normal
                let r_prefix = self.pending.strip_suffix('r').unwrap_or("");
                if (self.pending == "r" || r_prefix.bytes().all(|b| b.is_ascii_digit()))
                    && self.pending.ends_with('r')
                {
                    let count: usize = r_prefix.parse().unwrap_or(1);
                    self.pending.clear();
                    return self.replace_char_n(c, count);
                }
                // "xp / "xP: paste from a named register
                if self.pending.len() == 2
                    && self.pending.starts_with('"')
                    && (c == 'p' || c == 'P')
                {
                    let reg = self.pending.chars().nth(1);
                    self.pending.clear();
                    self.paste(reg, c == 'P');
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
        let line = self.buf().line_of(self.cursor);
        let start = self.buf().line_start(line);
        let end = self.buf().line_start(line + 1).min(self.buf().len_bytes());
        self.pipe_run(start, end, &cmd);
    }
    fn note_search(&mut self, cmd: &Command) {
        match &cmd.target {
            strop_grammar::Target::Motion(strop_grammar::Motion::Search(p)) => {
                self.last_search = Some(super::LastSearch {
                    pattern: p.clone(),
                    backward: false,
                    whole_word: false,
                });
            }
            strop_grammar::Target::Motion(strop_grammar::Motion::SearchBackward(p)) => {
                self.last_search = Some(super::LastSearch {
                    pattern: p.clone(),
                    backward: true,
                    whole_word: false,
                });
            }
            strop_grammar::Target::Motion(strop_grammar::Motion::FindChar {
                ch,
                till,
                backward,
            }) => {
                self.last_find = Some((*ch, *backward, *till));
            }
            _ => {}
        }
    }

    pub(crate) fn move_cursor(&mut self, cmd: &Command) {
        self.note_search(cmd);
        // jump-class motions record jumplist entries (vim: gg G % / ?)
        if matches!(
            cmd.target,
            strop_grammar::Target::Motion(
                strop_grammar::Motion::FirstLine
                    | strop_grammar::Motion::LastLine
                    | strop_grammar::Motion::MatchPair
                    | strop_grammar::Motion::Search(_)
                    | strop_grammar::Motion::SearchBackward(_)
            )
        ) {
            self.push_jump();
        }
        // the cascade (0013 §3): one scalar resolver, mapped over every
        // cursor — secondary cursors run the exact same motion
        let primary_hit = grammar::resolve(self.buf(), self.cursor, cmd);
        if let Some(r) = &primary_hit {
            self.cursor = grammar::cursor_after(self.buf(), self.cursor, cmd, r);
        }
        // take/compute/put-back: the resolver borrows self immutably
        let mut extras = std::mem::take(&mut self.extra_cursors);
        for c in &mut extras {
            if let Some(r) = grammar::resolve(self.buf(), *c, cmd) {
                *c = grammar::cursor_after(self.buf(), *c, cmd, &r);
            }
            *c = self.clamp_pos(*c);
        }
        self.extra_cursors = extras;
        self.clamp_cursor();
        self.normalize_cursors();
        // vim says so when a search finds nothing
        if matches!(
            cmd.target,
            strop_grammar::Target::Motion(
                strop_grammar::Motion::Search(_) | strop_grammar::Motion::SearchBackward(_)
            )
        ) && primary_hit.is_none()
        {
            self.message = "pattern not found".into();
        }
    }

    /// `;` / `,`: replay the last f/F/t/T (vim: `,` inverts direction),
    /// line-local like the original find, cascading over cursors.
    pub(crate) fn repeat_find(&mut self, reverse: bool) {
        let Some((ch, backward, till)) = self.last_find else {
            self.message = "no previous find".into();
            return;
        };
        let backward = backward ^ reverse;
        let seek = |buf: &strop_core::Buffer, c: usize| -> Option<usize> {
            let line = buf.line_of(c);
            let (ls, le) = (buf.line_start(line), buf.line_end(line));
            if backward {
                let mut pos = c.min(le);
                if pos == ls {
                    return None;
                }
                pos -= 1;
                loop {
                    if buf.byte(pos) == ch {
                        return Some(if till { (pos + 1).min(le) } else { pos });
                    }
                    if pos == ls {
                        return None;
                    }
                    pos -= 1;
                }
            }
            let mut pos = c + 1;
            while pos < le {
                if buf.byte(pos) == ch {
                    return Some(if till {
                        pos.saturating_sub(1).max(ls)
                    } else {
                        pos
                    });
                }
                pos += 1;
            }
            None
        };
        let mut extras = std::mem::take(&mut self.extra_cursors);
        for c in &mut extras {
            if let Some(h) = seek(self.buf(), *c) {
                *c = h;
            }
        }
        self.extra_cursors = extras;
        match seek(self.buf(), self.cursor) {
            Some(h) => {
                self.cursor = h;
                self.flash(Range::charwise(self.cursor, self.cursor));
            }
            None => self.message = "find: no more matches".into(),
        }
        self.normalize_cursors();
    }

    /// `n` / `N`: repeat the armed search, wrapping at the file edges.
    /// Cascades: every cursor seeks from its own position (0013 §3).
    pub(crate) fn repeat_search(&mut self, invert: bool) {
        let Some(ls) = self.last_search.clone() else {
            self.message = "no previous search".into();
            return;
        };
        let backward = ls.backward ^ invert;
        self.push_jump(); // n/N are jumplist entries
                          // a whole-word match has non-word bytes (or the edge) on both
                          // flanks — vim's \< \> without the regex layer
        let word_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let boundary_ok = |buf: &strop_core::Buffer, at: usize, len: usize| {
            let before_ok = at == 0 || !word_char(buf.byte(at - 1));
            let after_ok = at + len >= buf.len_bytes() || !word_char(buf.byte(at + len));
            !ls.whole_word || (before_ok && after_ok)
        };
        let seek = |buf: &strop_core::Buffer, from: usize| {
            let len = ls.pattern.len();
            let mut hit = if backward {
                grammar::search_backward(buf, from, &ls.pattern)
            } else {
                grammar::search_forward(buf, from + 1, &ls.pattern)
            };
            // skip boundary-mismatched hits (whole-word searches)
            let mut guard = 0;
            while hit.is_some_and(|h| !boundary_ok(buf, h, len)) && guard < 64 {
                let h = hit;
                hit = if backward {
                    grammar::search_backward(buf, h.unwrap_or(0), &ls.pattern)
                } else {
                    grammar::search_forward(buf, h.map(|x| x + 1).unwrap_or(0), &ls.pattern)
                };
                guard += 1;
            }
            // vim wraps around the file ends
            hit.or_else(|| {
                let mut h = if backward {
                    grammar::search_backward(buf, buf.len_bytes(), &ls.pattern)
                } else {
                    grammar::search_forward(buf, 0, &ls.pattern)
                };
                let mut guard = 0;
                while h.is_some_and(|x| !boundary_ok(buf, x, len)) && guard < 64 {
                    let cur = h;
                    h = if backward {
                        grammar::search_backward(buf, cur.unwrap_or(0), &ls.pattern)
                    } else {
                        grammar::search_forward(buf, cur.map(|x| x + 1).unwrap_or(0), &ls.pattern)
                    };
                    guard += 1;
                }
                h
            })
        };
        let mut extras = std::mem::take(&mut self.extra_cursors);
        for c in &mut extras {
            if let Some(h) = seek(self.buf(), *c) {
                *c = self.buf().clamp_boundary(h);
            }
        }
        self.extra_cursors = extras;
        match seek(self.buf(), self.cursor) {
            Some(h) => {
                self.cursor = self.buf().clamp_boundary(h);
                self.clamp_cursor();
                self.flash(Range::charwise(self.cursor, self.cursor));
            }
            None => self.message = format!("pattern not found: {}", ls.pattern),
        }
        self.normalize_cursors();
    }

    /// `*` / `#` (vim): search the word under the cursor — whole-word,
    /// forward / backward, wrapping. `n`/`N` keep the same anchors.
    pub(crate) fn search_word_under_cursor(&mut self, backward: bool) {
        let word_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let buf_len = self.buf().len_bytes();
        if self.cursor >= buf_len || !word_char(self.buf().byte(self.cursor)) {
            self.message = "no word under cursor".into();
            return;
        }
        let mut start = self.cursor;
        while start > 0 && word_char(self.buf().byte(start - 1)) {
            start -= 1;
        }
        let mut end = self.cursor;
        while end < buf_len && word_char(self.buf().byte(end)) {
            end += 1;
        }
        let pattern = self.buf().rope.byte_slice(start..end).to_string();
        self.last_search = Some(super::LastSearch {
            pattern,
            backward,
            whole_word: true,
        });
        // `#` seeks from the word's start so the current word isn't its
        // own "previous" match (vim semantics)
        if backward {
            self.cursor = start;
        }
        self.repeat_search(false);
    }

    /// The live preview: what would the pending keys do right now?
    /// Same resolver the executor uses — the preview cannot lie.
    pub fn preview(&self) -> Option<Resolved> {
        if self.pending.is_empty() {
            return None;
        }
        match grammar::parse(&self.pending) {
            Parse::Complete(cmd) if cmd.op.is_some() => {
                grammar::resolve(self.buf(), self.cursor, &cmd)
            }
            _ => {
                // partial backward search: d?foo mid-typing previews match→cursor
                if let Some(idx) = self.pending.find('?') {
                    let pat = &self.pending[idx + 1..];
                    if !pat.is_empty() && !pat.contains('\r') {
                        if let Some(hit) = grammar::search_backward(self.buf(), self.cursor, pat) {
                            return Some(Resolved {
                                range: Range::charwise(hit, self.cursor),
                                inclusive: false,
                                spec: format!("search ?{pat}"),
                            });
                        }
                    }
                }

                // partial search: d/foo mid-typing previews cursor→first match
                if let Some(idx) = self.pending.find('/') {
                    let pat = &self.pending[idx + 1..];
                    if !pat.is_empty() {
                        if let Some(hit) = grammar::search_forward(self.buf(), self.cursor + 1, pat)
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

    /// Ex-completion candidates for the pending prefix (name, doc).
    pub(crate) fn ex_candidates(&self) -> Vec<(&'static str, &'static str)> {
        let Some(prefix) = self.pending.strip_prefix(':') else {
            return Vec::new();
        };
        if prefix.contains(' ') {
            return Vec::new();
        }
        EX_COMMANDS
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .copied()
            .collect()
    }

    /// Tab on the ex line: cycle the completion candidates.
    fn ex_tab_complete(&mut self) {
        let cands = self.ex_candidates();
        if cands.is_empty() {
            return;
        }
        let prefix = self.pending.strip_prefix(':').unwrap_or("");
        let next = cands
            .iter()
            .position(|(name, _)| *name == prefix)
            .map_or(cands[0].0, |i| cands[(i + 1) % cands.len()].0);
        self.pending = format!(":{next}");
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
        self.note_search(cmd);
        // surround targets execute as pair edits, not operator ranges
        if let Some(()) = self.execute_surround(cmd) {
            return;
        }
        // the cascade (0013 §3): resolve per cursor, then apply bottom-up
        // so earlier ranges never shift under us
        let mut targets: Vec<(usize, Range, bool)> = self
            .all_cursors()
            .into_iter()
            .filter_map(|c| {
                grammar::resolve(self.buf(), c, cmd).map(|r| (c, r.range, r.range.linewise))
            })
            .collect();
        if targets.is_empty() {
            self.message = "no target".into();
            return;
        }
        // dedupe identical ranges (two cursors, same word), then drop
        // ranges that overlap an already-kept one below them
        targets.sort_by_key(|t| t.1.start);
        targets.dedup_by_key(|t| (t.1.start, t.1.end));
        let mut kept: Vec<(usize, Range, bool)> = Vec::new();
        for t in targets {
            if kept.last().is_none_or(|k| t.1.start >= k.1.end) {
                kept.push(t);
            }
        }
        match cmd.op.unwrap() {
            Op::Yank => {
                // one register, parts joined with newlines (helix rule)
                let texts: Vec<String> = kept
                    .iter()
                    .map(|(_, r, _)| self.buf().slice_string(*r))
                    .collect();
                let linewise = kept.first().is_some_and(|t| t.2);
                self.set_register(cmd.register, texts.join("\n"), linewise);
                self.flash(kept[0].1);
            }
            Op::Indent | Op::Dedent => {
                self.tx_begin();
                for (_, r, _) in kept.iter().rev() {
                    self.apply_indent(*r, cmd.op.unwrap() == Op::Indent);
                }
                self.tx_commit();
                self.normalize_cursors();
                self.flash(Range::charwise(self.cursor, self.cursor));
            }
            Op::Delete | Op::Change => {
                if cmd.op.unwrap() == Op::Change && kept.first().is_some_and(|t| t.2) {
                    // vim cc/S: clear content, keep the line — never
                    // merge with the next one
                    self.change_lines(cmd, &kept);
                    self.last_cmd_keys = cmd.keys.clone();
                    self.last_insert = None;
                    return;
                }
                // yank text reads top-down before any delete lands
                let texts: Vec<String> = kept
                    .iter()
                    .map(|(_, r, _)| self.buf().slice_string(*r))
                    .collect();
                let linewise = kept.first().is_some_and(|t| t.2);
                self.tx_begin();
                for (_, r, _) in kept.iter().rev() {
                    self.buf_mut().delete(*r);
                }
                self.set_register(cmd.register, texts.join("\n"), linewise);
                // landings: each range start minus what lower deletes
                // already removed (deletes applied bottom-up above)
                let mut shift = 0usize;
                let mut landings: Vec<(bool, usize)> = Vec::with_capacity(kept.len());
                for (c, r, _) in &kept {
                    landings.push((*c == self.cursor, r.start - shift));
                    shift += r.end - r.start;
                }
                self.cursor = landings
                    .iter()
                    .find(|(p, _)| *p)
                    .map(|(_, s)| *s)
                    .unwrap_or(self.cursor);
                let mut starts: Vec<usize> = landings.iter().map(|(_, s)| *s).collect();
                starts.sort_unstable();
                self.extra_cursors = starts.into_iter().filter(|&s| s != self.cursor).collect();
                self.normalize_cursors();
                if cmd.op.unwrap() == Op::Change {
                    // no commit: the insert session closes the undo unit
                    self.enter_insert_from(&cmd.keys);
                    // clamp AFTER the mode switch — insert mode allows
                    // the end-of-line cursor C just earned (0001 §5.5)
                    self.clamp_cursor();
                } else {
                    self.clamp_cursor();
                    self.tx_commit();
                }
                self.flash(Range::charwise(self.cursor, self.cursor));
            }
        }
        self.last_cmd_keys = cmd.keys.clone();
        self.last_insert = None;
    }

    /// vim `cc` / `S` (and counted `2cc`): clear each line's content,
    /// keep the line and its indent, open insert at the indent. The
    /// register gets the full lines (with newlines), linewise.
    fn change_lines(&mut self, cmd: &Command, kept: &[(usize, strop_core::Range, bool)]) {
        // texts + indents read top-down before any edit lands
        let texts: Vec<String> = kept
            .iter()
            .map(|(_, r, _)| {
                let first = self.buf().line_of(r.start);
                let last = self.buf().line_of(r.end.saturating_sub(1));
                let s = self.buf().line_start(first);
                let e = self.buf().line_start(last + 1).min(self.buf().len_bytes());
                self.buf().rope.byte_slice(s..e).to_string()
            })
            .collect();
        let indents: Vec<String> = kept
            .iter()
            .map(|(_, r, _)| {
                let line = self.buf().line_of(r.start);
                self.buf()
                    .line_text(line)
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect()
            })
            .collect();
        self.tx_begin();
        // apply bottom-up: each entry's range is still valid when it
        // applies; record (primary?, landing, net byte delta) per entry
        // and shift higher landings by the nets of the lower ones
        let mut entries: Vec<(bool, usize, isize)> = Vec::with_capacity(kept.len());
        for ((c, r, _), indent) in kept.iter().zip(indents.iter()).rev() {
            let first = self.buf().line_of(r.start);
            let last = self.buf().line_of(r.end.saturating_sub(1));
            let start = self.buf().line_start(first);
            let before = self.buf().len_bytes() as isize;
            if first == last {
                // one line: clear content, keep the newline
                let end = self.buf().line_end(first);
                self.buf_mut().delete(Range::charwise(start, end));
                self.buf_mut().insert(start, indent);
            } else {
                // N lines collapse into one fresh line
                let end = self.buf().line_start(last + 1).min(self.buf().len_bytes());
                self.buf_mut().delete(Range::charwise(start, end));
                self.buf_mut().insert(start, &format!("{indent}\n"));
            }
            let net = self.buf().len_bytes() as isize - before;
            entries.push((*c == self.cursor, start + indent.len(), net));
        }
        let landings: Vec<(bool, usize)> = entries
            .iter()
            .enumerate()
            .map(|(i, (p, at, _))| {
                // entries after i applied at lower positions → they
                // shift this landing
                let shift: isize = entries[i + 1..].iter().map(|(_, _, n)| n).sum();
                (*p, (*at as isize + shift).max(0) as usize)
            })
            .collect();
        self.set_register(cmd.register, texts.join(""), true);
        self.cursor = landings
            .iter()
            .find(|(p, _)| *p)
            .map(|(_, s)| *s)
            .unwrap_or(self.cursor);
        let mut starts: Vec<usize> = landings.iter().map(|(_, s)| *s).collect();
        starts.sort_unstable();
        self.extra_cursors = starts.into_iter().filter(|&s| s != self.cursor).collect();
        self.normalize_cursors();
        self.enter_insert_from(&cmd.keys);
        self.clamp_cursor();
        self.flash(Range::charwise(self.cursor, self.cursor));
    }

    fn dot_repeat(&mut self) {
        if self.last_cmd_keys.is_empty() && self.last_insert.is_none() {
            return;
        }
        let keys = self.last_cmd_keys.clone();
        let insert = self.last_insert.clone();
        if !keys.is_empty() {
            self.feed_text(&keys);
        }
        if let Some(text) = insert {
            let was_insert = self.mode == Mode::Insert;
            if !was_insert {
                self.enter_insert_from("i");
            }
            for c in text.chars() {
                self.feed(Key::Char(c));
            }
            self.feed(Key::Esc);
            self.message = "repeated".into();
        }
    }

    /// vim `3rx` replaces three chars (clamped to the line end).
    fn replace_char_n(&mut self, c: char, count: usize) {
        let line_end = self.buf().line_end(self.buf().line_of(self.cursor));
        let end = (self.cursor + count).min(line_end);
        if end <= self.cursor || c == '\n' {
            return;
        }
        let cursor = self.cursor;
        self.tx_begin();
        self.buf_mut().delete(Range::charwise(cursor, end));
        let text: String = std::iter::repeat_n(c, end - cursor).collect();
        self.buf_mut().insert(cursor, &text);
        self.tx_commit();
        self.cursor = cursor + text.len() - 1; // last replaced char
        self.flash(Range::charwise(self.cursor, self.cursor));
        self.last_cmd_keys = format!("r{c}");
        self.last_insert = None;
    }

    fn join_lines(&mut self) {
        self.tx_begin();
        let line = self.buf().line_of(self.cursor);
        if line + 1 >= self.buf().len_lines() {
            self.tx_commit();
            return;
        }
        let eol = self.buf().line_end(line);
        let next_start = self.buf().line_start(line + 1);
        let next_end = self.buf().line_end(line + 1);
        // delete newline + leading whitespace of the next line, add one space
        let mut join_at = next_start;
        while join_at < next_end
            && self.buf().byte(join_at).is_ascii_whitespace()
            && self.buf().byte(join_at) != b'\n'
        {
            join_at += 1;
        }
        self.buf_mut().delete(Range::charwise(eol, join_at));
        if join_at < next_end {
            self.buf_mut().insert(eol, " ");
        }
        self.cursor = eol;
        self.tx_commit();
        self.clamp_cursor();
        self.flash(Range::charwise(eol, (eol + 1).min(self.buf().len_bytes())));
        self.last_cmd_keys = "J".into();
        self.last_insert = None;
    }

    /// `~`: toggle the case of the char under the cursor and advance
    /// (non-letters just advance; EOL stays put).
    fn toggle_case(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        let line_end = self.buf().line_end(self.buf().line_of(self.cursor));
        if self.cursor >= line_end {
            return;
        }
        let b = self.buf().byte(self.cursor);
        let c = b as char;
        if c.is_ascii_alphabetic() {
            let flipped = if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            };
            let cursor = self.cursor;
            self.tx_begin();
            self.buf_mut().delete(Range::charwise(cursor, cursor + 1));
            self.buf_mut().insert(cursor, &flipped.to_string());
            self.tx_commit();
        }
        // advance one char, not one byte — a multibyte char would
        // otherwise park the cursor mid-char for the next edit
        self.cursor = self.buf().ceil_boundary(self.cursor + 1);
        self.clamp_cursor();
        self.last_cmd_keys = "~".into();
        self.last_insert = None;
    }

    /// > / < applied to every line a resolved range covers.
    pub(crate) fn apply_indent(&mut self, range: Range, right: bool) {
        let line = self.buf().line_of(range.start);
        let last = self.buf().line_of(range.end.saturating_sub(1)) + 1;
        for l in line..last {
            let start = self.buf().line_start(l);
            if right {
                let indent = self.config.indent();
                self.buf_mut().insert(start, &indent);
            } else {
                let end = self.buf().line_end(l);
                let width = self.config.tab_size;
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
        self.cursor = self.buf().line_start(line);
        self.clamp_cursor();
    }

    pub(crate) fn run_ex(&mut self) {
        let cmdline = self
            .pending
            .trim_start_matches(':')
            .trim_end_matches('\r')
            .to_string();
        self.pending.clear();
        let (cmd, arg) = cmdline.split_once(' ').unwrap_or((cmdline.as_str(), ""));
        match cmd {
            _ if cmdline.starts_with('!') => self.shell_run(&cmdline[1..]),
            "w" => match self.buf_mut().save() {
                Ok(()) => {
                    crate::session::save(self);
                    self.message = "written".into();
                }
                Err(e) => self.message = format!("write failed: {e}"),
            },
            "view" => {
                // vim view: edit readonly — no arg marks the current
                // buffer readonly
                if arg.is_empty() {
                    self.buf_mut().readonly = true;
                    self.message = "readonly".into();
                } else if let Err(e) = self.open_buffer(arg) {
                    self.message = format!("view {arg}: {e}");
                } else {
                    self.buf_mut().readonly = true;
                }
            }
            "q" => {
                self.close_pane_or_buffer(false);
            }
            "q!" => {
                self.close_pane_or_buffer(true);
            }
            "wq" => {
                let _ = self.buf_mut().save();
                self.close_buffer(true);
            }
            "noh" => {
                // nohlsearch: the persistent highlight drops (0001 §5.8)
                self.last_search = None;
            }
            _ if cmdline.bytes().all(|b| b.is_ascii_digit()) && !cmdline.is_empty() => {
                // :30 jumps to line 30 (vim); past EOF clamps to the last
                // content line, never the phantom past a trailing newline
                let n: usize = cmdline.parse().unwrap_or(1);
                let mut last = self.buf().len_lines().saturating_sub(1);
                if self.buf().line_start(last) >= self.buf().len_bytes() {
                    last = last.saturating_sub(1);
                }
                self.push_jump(); // :N is a jump — record before moving
                self.cursor = self.buf().line_start(n.saturating_sub(1).min(last));
                self.clamp_cursor();
            }
            "vs" | "vsplit" => self.split(true, if arg.is_empty() { None } else { Some(arg) }),
            "sp" | "split" => self.split(false, if arg.is_empty() { None } else { Some(arg) }),
            "help" | "h" => self.open_help(),
            "e" | "e!" => {
                if arg.is_empty() {
                    self.message = ":e needs a path".into();
                } else if self.buf().dirty && cmd == "e" {
                    self.message = "unsaved changes — :e! to force".into();
                } else if let Err(e) = self.open_buffer(arg) {
                    self.message = format!("open {arg}: {e}");
                }
            }
            other => self.message = format!("unknown ex: :{other}"),
        }
    }
}
