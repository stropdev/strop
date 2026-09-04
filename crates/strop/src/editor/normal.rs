//! Normal mode: the grammar's home. Operators, motions, counts,
//! registers, dot-repeat, the ex-line — and the live preview query.

use strop_core::Range;
use strop_grammar::{self as grammar, Command, Op, Parse, Resolved};

use super::{Editor, Key, Mode};

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
        let Key::Char(c) = key else {
            return;
        };
        match c {
            '1'..='9' => self.pending.push(c),
            '0' => self.run_motion("0"),
            'h' | 'j' | 'k' | 'l' | 'w' | 'b' | 'e' | 'W' | 'B' | 'E' | '$' | 'G' | '%' => {
                self.run_motion(&c.to_string())
            }
            'g' | 'd' | 'y' | 'c' | 'f' | 'F' | 't' | 'T' | '/' | '?' | ':' | '"' | 'r' | '>'
            | '<' | ' ' | '[' | ']' | 'm' | '\'' | '`' => self.pending.push(c),
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
                self.cursor =
                    (self.cursor + 1).min(self.buf().line_end(self.buf().line_of(self.cursor)));
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
                self.buf_mut().insert(start, &text);
                self.cursor = start + indent.len();
                self.enter_insert_from("O");
                // same as 'o': indent is derived, not recorded
            }
            'x' => {
                let end =
                    (self.cursor + 1).min(self.buf().line_end(self.buf().line_of(self.cursor)));
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
                self.mode = Mode::Visual;
                self.anchor = self.cursor;
            }
            'V' => {
                self.mode = Mode::VisualLine;
                self.anchor = self.cursor;
            }
            'J' => self.join_lines(),
            '.' => self.dot_repeat(),
            'u' => self.undo(),
            _ => {}
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
        let is_search = !is_ex && (self.pending.contains('/') || self.pending.contains('?'));
        match key {
            Key::Esc => self.pending.clear(),
            Key::Backspace => {
                self.pending.pop();
            }
            Key::Enter if is_ex => self.run_ex(),
            Key::Enter if is_search => {
                self.pending.push('\r');
                self.resolve_pending();
            }
            Key::Enter => self.pending.clear(),
            Key::CtrlR | Key::CtrlW | Key::CtrlX => {} // pending + window/undo keys: no-op
            Key::Up | Key::Down | Key::Tab | Key::Backtab => {}
            Key::Char(c) => {
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
                if self.pending == "r" {
                    self.pending.clear();
                    return self.replace_char(c);
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

    /// `/pat⏎` arms `n`/`N` (vim search repeat).
    fn note_search(&mut self, cmd: &Command) {
        match &cmd.target {
            strop_grammar::Target::Motion(strop_grammar::Motion::Search(p)) => {
                self.last_search = Some((p.clone(), false));
            }
            strop_grammar::Target::Motion(strop_grammar::Motion::SearchBackward(p)) => {
                self.last_search = Some((p.clone(), true));
            }
            _ => {}
        }
    }

    pub(crate) fn move_cursor(&mut self, cmd: &Command) {
        self.note_search(cmd);
        if let Some(r) = grammar::resolve(self.buf(), self.cursor, cmd) {
            self.cursor = grammar::cursor_after(self.buf(), self.cursor, cmd, &r);
            self.clamp_cursor();
        }
    }

    /// `n` / `N`: repeat the armed search, wrapping at the file edges.
    pub(crate) fn repeat_search(&mut self, invert: bool) {
        let Some((pat, backward)) = self.last_search.clone() else {
            self.message = "no previous search".into();
            return;
        };
        let backward = backward ^ invert;
        let hit = if backward {
            grammar::search_backward(self.buf(), self.cursor, &pat)
        } else {
            grammar::search_forward(self.buf(), self.cursor + 1, &pat)
        };
        // vim wraps around the file ends
        let hit = hit.or_else(|| {
            if backward {
                grammar::search_backward(self.buf(), self.buf().len_bytes(), &pat)
            } else {
                grammar::search_forward(self.buf(), 0, &pat)
            }
        });
        match hit {
            Some(h) => {
                self.cursor = self.buf().clamp_boundary(h);
                self.clamp_cursor();
                self.flash(Range::charwise(self.cursor, self.cursor));
            }
            None => self.message = format!("pattern not found: {pat}"),
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
        let Some(r) = grammar::resolve(self.buf(), self.cursor, cmd) else {
            self.message = "no target".into();
            return;
        };
        match cmd.op.unwrap() {
            Op::Yank => {
                let text = self.buf().slice_string(r.range);
                self.set_register(cmd.register, text, r.range.linewise);
                self.flash(r.range);
            }
            Op::Indent | Op::Dedent => {
                self.apply_indent(r.range, cmd.op.unwrap() == Op::Indent);
                self.flash(Range::charwise(self.cursor, self.cursor));
            }
            Op::Delete | Op::Change => {
                self.tx_begin();
                let text = self.buf_mut().delete(r.range);
                self.set_register(cmd.register, text, r.range.linewise);
                self.cursor = r.range.start;
                self.clamp_cursor();
                self.flash(Range::charwise(self.cursor, self.cursor));
                if cmd.op.unwrap() == Op::Change {
                    // no commit: the insert session closes the undo unit
                    self.enter_insert_from(&cmd.keys);
                } else {
                    self.tx_commit();
                }
            }
        }
        self.last_cmd_keys = cmd.keys.clone();
        self.last_insert = None;
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

    fn replace_char(&mut self, c: char) {
        let end = (self.cursor + 1).min(self.buf().line_end(self.buf().line_of(self.cursor)));
        if end <= self.cursor || c == '\n' {
            return;
        }
        let cursor = self.cursor;
        self.tx_begin();
        self.buf_mut().delete(Range::charwise(cursor, end));
        let mut tmp = [0u8; 4];
        self.buf_mut().insert(cursor, c.encode_utf8(&mut tmp));
        self.tx_commit();
        self.flash(Range::charwise(self.cursor, self.cursor + 1));
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

    /// > / < applied to every line a resolved range covers.
    fn apply_indent(&mut self, range: Range, right: bool) {
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
            "w" => match self.buf_mut().save() {
                Ok(()) => {
                    crate::session::save(self);
                    self.message = "written".into();
                }
                Err(e) => self.message = format!("write failed: {e}"),
            },
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
