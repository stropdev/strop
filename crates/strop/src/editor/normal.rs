//! Normal mode: the grammar's home. Operators, motions, counts,
//! registers, dot-repeat, the ex-line — and the live preview query.

use strop_core::Range;
use strop_grammar::{self as grammar, Command, Op, Parse};

use super::{Editor, Key, Mode};

/// The ex vocabulary (completion + `run_ex` dispatch reads the same
/// list — one table, no drift).
pub(crate) const EX_COMMANDS: &[(&str, &str)] = &[
    ("w", "write"),
    ("w!", "write, force (file changed on disk)"),
    ("wq!", "write forced + quit"),
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
        if !self.pending.is_empty() {
            return self.feed_pending(key);
        }
        // Esc is a mode-level key: collapse to the primary cursor and
        // ground the machine (0013 §3) — it never walks the trie
        if key == Key::Esc {
            self.collapse_cursors();
            self.walker.clear();
            return;
        }
        // every other key event walks the one machine (0016)
        match self.walker.feed(key) {
            super::input::Action::Pending => {}
            super::input::Action::Invalid(keys) => {
                self.message = format!("not an editor command: {keys}")
            }
            super::input::Action::EnterText(c) => {
                self.pending = c.to_string();
                self.pending_normal = false;
                self.pending_cursor = self.pending.len();
            }
            super::input::Action::Grammar(cmd) => match cmd.op {
                None => self.move_cursor(&cmd),
                Some(_) => self.execute(&cmd),
            },
            super::input::Action::Row {
                row,
                count,
                register,
                arg,
                key,
            } => self.dispatch_row(row, count, register, arg, key),
        }
    }

    /// A table row dispatch: typed count/register/arg ride the Action
    /// (0016 — no string inspection). Count semantics by id:
    /// most leaves repeat, insert entries carry it into the session,
    /// visible jumps treat it as a line offset, replace multiplies.
    fn dispatch_row(
        &mut self,
        row: &'static crate::keymap::Binding,
        count: Option<usize>,
        register: Option<char>,
        arg: Option<char>,
        key: char,
    ) {
        use crate::keymap::Handler;
        let n = count.unwrap_or(1);
        match row.handler {
            Handler::Leaf(f) => {
                let last = key;
                match row.id {
                    "visible-jumps" => self.jump_visible(last, n),
                    "insert-entries" => {
                        self.insert_count = n;
                        f(self, last);
                    }
                    "paste" if register.is_some() => self.paste_named(register, last == 'P'),
                    "paste" => self.paste_n(n, last == 'P'),
                    "scroll-pages" => self.scroll_counted(last, n),
                    _ => {
                        for _ in 0..n {
                            f(self, last);
                        }
                    }
                }
            }
            // aliases are semantic (0016): the expansion parses ONCE
            // into a grammar Command; the walker's count/register merge
            // in — nothing replays through input
            Handler::Alias(expansion) => {
                if let Parse::Complete(mut cmd) = grammar::parse(expansion) {
                    cmd.count = Some(n * cmd.count.unwrap_or(1));
                    if register.is_some() {
                        cmd.register = register;
                    }
                    match cmd.op {
                        None => self.move_cursor(&cmd),
                        Some(_) => self.execute(&cmd),
                    }
                }
            }
            Handler::AbsorbChar(kind) => {
                let c = arg.unwrap_or('\0');
                use crate::keymap::AbsorbKind;
                match kind {
                    AbsorbKind::Replace => self.replace_char_n(c, n),
                    AbsorbKind::MarkSet => self.set_mark(c),
                    AbsorbKind::MarkJump => self.jump_mark(c),
                    AbsorbKind::Find => {} // grammar resolved f<c> in the machine
                    AbsorbKind::MacroRecord => self.macro_toggle(c),
                    AbsorbKind::MacroPlay if c == '@' => self.macro_again(n),
                    AbsorbKind::MacroPlay => self.macro_play(c, n),
                }
            }
            Handler::Prefix
            | Handler::Motion
            | Handler::Operator
            | Handler::ObjectPrefix
            | Handler::TextLine
            | Handler::AbsorbRegister
            | Handler::Soon => {}
        }
    }

    /// `x`: delete the whole char under the cursor — a bare +1 splits
    /// `x`: delete the whole char under the cursor — a bare +1 splits
    /// multibyte chars and panics ropey's byte_slice.
    pub(crate) fn delete_char(&mut self) {
        let end = self
            .buf()
            .ceil_boundary(self.head() + 1)
            .min(self.buf().line_end(self.buf().line_of(self.head())));
        if end > self.head() {
            self.tx_begin();
            let range = Range::charwise(self.head(), end);
            let text = self.buf_mut().delete(range);
            self.tx_commit();
            self.set_register(None, text, false);
            self.flash(range);
            self.last_cmd_keys = "x".into();
            self.last_insert = None;
        }
    }

    /// `p` / `P`: paste the register after/before (dot-repeatable).
    pub(crate) fn paste_named(&mut self, name: Option<char>, before: bool) {
        self.paste(name, before);
        self.last_cmd_keys = if before { "P".into() } else { "p".into() };
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
        let indent = self.auto_indent_full_line();
        let end = self.buf().line_end(self.buf().line_of(self.head()));
        let text = format!("\n{indent}");
        self.insert_open = Some(text.clone());
        self.buf_mut().insert(end, &text);
        self.set_head(end + text.len());
        self.enter_insert_from("o");
    }

    /// `O`: open above — same indent derivation as `o`.
    pub(crate) fn open_above(&mut self) {
        let indent = self.auto_indent_full_line();
        let start = self.buf().line_start(self.buf().line_of(self.head()));
        let text = format!("{indent}\n");
        self.insert_open = Some(text.clone());
        self.buf_mut().insert(start, &text);
        self.set_head(start + indent.len());
        self.enter_insert_from("O");
    }

    /// `v` / `V`: visual mode is primary-only — extras collapse (0013).
    pub(crate) fn enter_visual(&mut self, linewise: bool) {
        self.sels_mut().collapse_extras();
        self.mode = if linewise {
            Mode::VisualLine
        } else {
            Mode::Visual
        };
        let h = self.head();
        self.sels_mut().stretch_primary(h, h);
    }

    /// `.` — see dot_repeat (semantic; 0014).
    pub(crate) fn dot_repeat_pub(&mut self) {
        self.dot_repeat();
    }

    /// `~` — see toggle_case.
    pub(crate) fn toggle_case_pub(&mut self) {
        self.toggle_case();
    }

    /// `J` — see join_lines.
    pub(crate) fn join_lines_pub(&mut self) {
        self.join_lines();
    }

    /// ds" / cs"' / ysiw" (sandwich lineage). Returns Some when the
    /// command was a surround op and got handled.
    fn execute_surround(&mut self, cmd: &Command) -> Option<()> {
        let r = grammar::resolve(self.buf(), self.head(), cmd)?;
        let pair = |ch: char| match ch {
            'b' | '(' | ')' => ('(', ')'),
            'B' | '{' | '}' => ('{', '}'),
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

    /// Alias keys (D → d$, …): execute the expansion, remember the alias
    /// so dot-repeat replays through the same path.
    pub(crate) fn alias(&mut self, alias_key: &str, expansion: &str) {
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
        let primary_hit = grammar::resolve(self.buf(), self.head(), cmd);
        if let Some(r) = &primary_hit {
            let land = grammar::cursor_after(self.buf(), self.head(), cmd, r);
            self.set_head(land);
        }
        // take/compute/replant: the resolver borrows self immutably
        let extras: Vec<usize> = self
            .extra_selections()
            .iter()
            .map(|s| {
                let c = match grammar::resolve(self.buf(), s.head, cmd) {
                    Some(r) => grammar::cursor_after(self.buf(), s.head, cmd, &r),
                    None => s.head,
                };
                self.clamp_pos(c)
            })
            .collect();
        self.sels_mut().set_extras(extras);
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
        // char-honest: ; , on f é must land on é (0014)
        let seek = |buf: &strop_core::Buffer, c: usize| -> Option<usize> {
            let line = buf.line_of(c);
            let (ls, le) = (buf.line_start(line), buf.line_end(line));
            let text = buf.line_text(line);
            if backward {
                for (off, t) in text.char_indices().rev() {
                    let pos = ls + off;
                    if pos >= c.min(le) {
                        continue;
                    }
                    if t == ch {
                        return Some(if till { (pos + 1).min(le) } else { pos });
                    }
                }
                return None;
            }
            for (off, t) in text.char_indices() {
                let pos = ls + off;
                if pos <= c {
                    continue;
                }
                if pos >= le {
                    break;
                }
                if t == ch {
                    return Some(if till {
                        pos.saturating_sub(1).max(ls)
                    } else {
                        pos
                    });
                }
            }
            None
        };
        let extras: Vec<usize> = self
            .extra_selections()
            .iter()
            .map(|s| seek(self.buf(), s.head).unwrap_or(s.head))
            .collect();
        self.sels_mut().set_extras(extras);
        match seek(self.buf(), self.head()) {
            Some(h) => {
                self.set_head(h);
                self.flash(Range::charwise(self.head(), self.head()));
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
        let extras: Vec<usize> = self
            .extra_selections()
            .iter()
            .map(|s| {
                seek(self.buf(), s.head)
                    .map(|h| self.buf().clamp_boundary(h))
                    .unwrap_or(s.head)
            })
            .collect();
        self.sels_mut().set_extras(extras);
        match seek(self.buf(), self.head()) {
            Some(h) => {
                self.set_head(self.buf().clamp_boundary(h));
                self.clamp_cursor();
                self.flash(Range::charwise(self.head(), self.head()));
            }
            None => self.message = format!("pattern not found: {}", ls.pattern),
        }
        self.normalize_cursors();
    }

    /// `*` / `#` (vim): search the word under the cursor — whole-word,
    /// forward / backward, wrapping. `n`/`N` keep the same anchors.
    pub(crate) fn search_word_under_cursor(&mut self, backward: bool) {
        // char-classified (0017): identifiers in every script count —
        // é/fün/変数 are words. Walk CHAR boundaries via the line text.
        let word_char = |c: char| c.is_alphanumeric() || c == '_';
        let buf_len = self.buf().len_bytes();
        let head = self.buf().clamp_boundary(self.head());
        if head >= buf_len {
            self.message = "no word under cursor".into();
            return;
        }
        let char_at = |p: usize| -> Option<char> {
            let line = self.buf().line_of(p);
            let (s, e) = (self.buf().line_start(line), self.buf().line_end(line));
            self.buf()
                .rope
                .byte_slice(s..e)
                .to_string()
                .trim_end_matches('\n')
                .get(p - s..)
                .and_then(|t| t.chars().next())
        };
        if !char_at(head).is_some_and(word_char) {
            self.message = "no word under cursor".into();
            return;
        }
        let mut start = head;
        while start > 0 {
            let prev = self.buf().clamp_boundary(start.saturating_sub(4));
            let prev = if prev == start { start - 1 } else { prev };
            if char_at(prev).is_some_and(word_char) {
                start = prev;
            } else {
                break;
            }
        }
        let mut end = head;
        while end < buf_len {
            let line = self.buf().line_of(end);
            let (_, le) = (self.buf().line_start(line), self.buf().line_end(line));
            let next = char_at(end).map(|c| end + c.len_utf8());
            match next {
                Some(n) if n <= le && char_at(end).is_some_and(word_char) => end = n,
                _ => break,
            }
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
            self.set_head(start);
        }
        self.repeat_search(false);
    }

    /// The live preview: what would the pending keys do right now?
    /// The plan the executor would apply — every cursor's range (0014
    /// wave 3): the preview cannot lie, and multicursor previews too.
    pub fn preview(&self) -> Option<(Vec<Range>, String)> {
        if self.pending.is_empty() {
            return None;
        }
        match grammar::parse(&self.pending) {
            Parse::Complete(cmd) if cmd.op.is_some() => {
                let spec = grammar::resolve(self.buf(), self.head(), &cmd)?.spec;
                let plan = grammar::plan(self.buf(), &self.all_cursors(), &cmd)?;
                Some((plan.targets.iter().map(|t| t.range).collect(), spec))
            }
            _ => {
                // partial backward search: d?foo mid-typing previews match→cursor
                if let Some(idx) = self.pending.find('?') {
                    let pat = &self.pending[idx + 1..];
                    if !pat.is_empty() && !pat.contains('\r') {
                        if let Some(hit) = grammar::search_backward(self.buf(), self.head(), pat) {
                            return Some((
                                vec![Range::charwise(hit, self.head())],
                                format!("search ?{pat}"),
                            ));
                        }
                    }
                }

                // partial search: d/foo mid-typing previews cursor→first match
                if let Some(idx) = self.pending.find('/') {
                    let pat = &self.pending[idx + 1..];
                    if !pat.is_empty() {
                        if let Some(hit) = grammar::search_forward(self.buf(), self.head() + 1, pat)
                        {
                            return Some((
                                vec![Range::charwise(self.head(), hit)],
                                format!("search /{pat}"),
                            ));
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
        // semantic dot-repeat (0014): `.` re-resolves this command from
        // the new position — it never replays a stale key string through
        // a changed keymap
        self.last_change = Some(cmd.clone());
        self.last_cmd_keys.clear();
        self.note_search(cmd);
        // surround targets execute as pair edits, not operator ranges
        if let Some(()) = self.execute_surround(cmd) {
            return;
        }
        // the cascade (0013 §3) IS the plan (0014 §3): preview renders
        // these same targets — one object, no preview/execute drift
        let Some(plan) = grammar::plan(self.buf(), &self.all_cursors(), cmd) else {
            self.message = "no target".into();
            return;
        };
        let kept: Vec<(usize, Range, bool)> = plan
            .targets
            .iter()
            .map(|t| (t.cursor, t.range, t.range.is_linewise()))
            .collect();
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
                self.flash(Range::charwise(self.head(), self.head()));
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
                    landings.push((*c == self.head(), r.start - shift));
                    shift += r.end - r.start;
                }
                self.set_head(
                    landings
                        .iter()
                        .find(|(p, _)| *p)
                        .map(|(_, s)| *s)
                        .unwrap_or(self.head()),
                );
                // extras are the SECONDARY landings — the primary must
                // never stack with itself (0015: dw leaves 1 cursor)
                let mut starts: Vec<usize> = landings
                    .iter()
                    .filter(|(p, _)| !*p)
                    .map(|(_, s)| *s)
                    .collect();
                starts.sort_unstable();
                self.sels_mut().set_extras(starts);
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
                self.flash(Range::charwise(self.head(), self.head()));
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
            entries.push((*c == self.head(), start + indent.len(), net));
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
        self.set_head(
            landings
                .iter()
                .find(|(p, _)| *p)
                .map(|(_, s)| *s)
                .unwrap_or(self.head()),
        );
        let mut starts: Vec<usize> = landings.iter().map(|(_, s)| *s).collect();
        starts.sort_unstable();
        self.sels_mut().set_extras(starts);
        self.enter_insert_from(&cmd.keys);
        self.clamp_cursor();
        self.flash(Range::charwise(self.head(), self.head()));
    }

    fn dot_repeat(&mut self) {
        if self.last_change.is_none() && self.last_cmd_keys.is_empty() && self.last_insert.is_none()
        {
            return;
        }
        let insert = self.last_insert.clone();
        if let Some(cmd) = self.last_change.clone() {
            // the same change, resolved fresh from here (0014 §input)
            self.execute(&cmd);
        } else {
            // direct (non-grammar) commands replay their keys
            let keys = self.last_cmd_keys.clone();
            if !keys.is_empty() {
                self.feed_text(&keys);
            }
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
        // count is CHARS (0017) — a byte count splits multibyte text
        let line = self.buf().line_of(self.head());
        let (s, e) = (self.buf().line_start(line), self.buf().line_end(line));
        let text_line = self.buf().rope.byte_slice(s..e).to_string();
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
            .rope
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
        self.last_insert = None;
    }

    fn join_lines(&mut self) {
        self.tx_begin();
        let line = self.buf().line_of(self.head());
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
        self.set_head(eol);
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
        let line_end = self.buf().line_end(self.buf().line_of(self.head()));
        if self.head() >= line_end {
            return;
        }
        let b = self.buf().byte(self.head());
        let c = b as char;
        if c.is_ascii_alphabetic() {
            let flipped = if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            };
            let cursor = self.head();
            self.tx_begin();
            self.buf_mut().delete(Range::charwise(cursor, cursor + 1));
            self.buf_mut().insert(cursor, &flipped.to_string());
            self.tx_commit();
        }
        // advance one char, not one byte — a multibyte char would
        // otherwise park the cursor mid-char for the next edit
        self.set_head(self.buf().ceil_boundary(self.head() + 1));
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
        self.set_head(self.buf().line_start(line));
        self.clamp_cursor();
    }

    /// Parse a leading ex range: `%`, `.`, `$`, `N`, `N,M`, with
    /// +/- offsets. Returns 0-indexed inclusive line bounds + the
    /// remaining command text, or None when no range leads.
    fn parse_ex_range<'a>(&self, cmdline: &'a str) -> (Option<(usize, usize)>, &'a str) {
        let buf = self.buf();
        let last = buf.last_content_line();
        let cur = buf.line_of(self.head());
        let addr = |tok: &str| -> Option<(usize, usize)> {
            // one address + the bytes it consumed
            match tok.as_bytes().first()? {
                b'%' => Some((0, 1)),
                b'.' => Some((cur, 1)),
                b'$' => Some((last, 1)),
                b if b.is_ascii_digit() => {
                    let n: usize = tok
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse()
                        .ok()?;
                    Some((n.saturating_sub(1).min(last), n.to_string().len()))
                }
                _ => None,
            }
        };
        let (mut first, mut used) = match addr(cmdline) {
            Some(v) => v,
            None => return (None, cmdline),
        };
        let mut second = None;
        if cmdline.as_bytes().get(used) == Some(&b',') {
            match addr(&cmdline[used + 1..]) {
                Some((l2, u2)) => {
                    second = Some(l2);
                    used += 1 + u2;
                }
                None => return (None, cmdline),
            }
        }
        // +/- offsets trail an address (:+3, :-2, :.-1,$-1)
        let whole = cmdline[..used].to_string();
        let mut tail = &cmdline[used..];
        let apply_off = |line: usize, tail: &str| -> (usize, usize) {
            let b = tail.as_bytes();
            let mut i = 0;
            let mut line = line;
            while i < tail.len() && (b[i] == b'+' || b[i] == b'-') {
                let neg = b[i] == b'-';
                i += 1;
                let digits: String = tail[i..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                let n: usize = if digits.is_empty() {
                    1
                } else {
                    digits.parse().unwrap_or(1)
                };
                i += digits.len();
                line = if neg {
                    line.saturating_sub(n)
                } else {
                    (line + n).min(last)
                };
            }
            (line, i)
        };
        let (f, fu) = apply_off(first, tail);
        first = f;
        tail = &tail[fu..];
        let mut last_line = second.unwrap_or(first);
        if second.is_some() {
            let (l, lu) = apply_off(last_line, tail);
            last_line = l;
            tail = &tail[lu..];
        }
        if whole == "%" {
            return (Some((0, last)), tail);
        }
        if first > last_line {
            return (None, cmdline); // backwards range: vim errors
        }
        (Some((first.min(last), last_line.min(last))), tail)
    }

    /// Ranged commands: `:N` alone jumps; `d`/`y` delete/yank the
    /// lines; `s/a/b/[g]` substitutes (LITERAL pattern — vim's regex
    /// substitute is a documented deviation until 0016's grammar work).
    fn run_ranged_ex(&mut self, range: (usize, usize), rest: &str) {
        let (lo, hi) = range;
        if rest.is_empty() {
            // :N — goto line
            let s = self.buf().line_start(lo);
            self.set_head(s);
            self.clamp_cursor();
            self.scroll_to_cursor(self.view_rows());
            return;
        }
        match rest {
            "d" | "d!" => {
                let s = self.buf().line_start(lo);
                let e = if hi + 1 < self.buf().len_lines() {
                    self.buf().line_start(hi + 1)
                } else {
                    self.buf().len_bytes()
                };
                let text = self.buf().rope.byte_slice(s..e).to_string();
                self.registers.insert('\0', (text, true));
                let b = self.buf_mut();
                b.history.begin();
                b.delete(strop_core::Range::charwise(s, e));
                b.history.commit();
                self.set_head(self.buf().clamp_boundary(s));
                self.clamp_cursor();
                self.message = format!("{} lines deleted", hi - lo + 1);
            }
            "y" => {
                let s = self.buf().line_start(lo);
                let e = if hi + 1 < self.buf().len_lines() {
                    self.buf().line_start(hi + 1)
                } else {
                    self.buf().len_bytes()
                };
                let text = self.buf().rope.byte_slice(s..e).to_string();
                self.registers.insert('\0', (text, true));
                self.message = format!("{} lines yanked", hi - lo + 1);
            }
            _ if rest.starts_with("s/") => self.substitute_range(lo, hi, &rest[2..]),
            _ => self.message = format!("unsupported ranged command: {rest}"),
        }
    }

    /// `:[range]s/pat/repl/[g]` — literal pattern, vim's flag letter g.
    fn substitute_range(&mut self, lo: usize, hi: usize, spec: &str) {
        let parts: Vec<&str> = spec.split('/').collect();
        if parts.len() < 2 {
            self.message = ":s needs /pat/repl/".into();
            return;
        }
        let (pat, repl) = (parts[0], parts[1]);
        let global = parts.get(2).is_some_and(|f| f.contains('g'));
        if pat.is_empty() {
            self.message = "empty pattern".into();
            return;
        }
        let s0 = self.buf().line_start(lo);
        let e0 = self.buf().line_end(hi);
        let text = self.buf().rope.byte_slice(s0..e0).to_string();
        let mut out = String::with_capacity(text.len());
        let mut hits = 0usize;
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                out.push('\n');
            }
            if global {
                let n = line.matches(pat).count();
                hits += n;
                out.push_str(&line.replace(pat, repl));
            } else if let Some(p) = line.find(pat) {
                hits += 1;
                out.push_str(&line[..p]);
                out.push_str(repl);
                out.push_str(&line[p + pat.len()..]);
            } else {
                out.push_str(line);
            }
        }
        if hits == 0 {
            self.message = format!("pattern not found: {pat}");
            return;
        }
        {
            let b = self.buf_mut();
            b.history.begin();
            b.delete(strop_core::Range::charwise(s0, e0));
            b.insert(s0, &out);
            b.history.commit();
        }
        self.set_head(self.buf().clamp_boundary(s0));
        self.clamp_cursor();
        let end = (s0 + out.len()).min(self.buf().len_bytes());
        self.flash(strop_core::Range::charwise(s0, end));
        self.message = format!("{hits} substitution{}", if hits == 1 { "" } else { "s" });
    }

    pub(crate) fn run_ex(&mut self) {
        let cmdline = self
            .pending
            .trim_start_matches(':')
            .trim_end_matches('\r')
            .to_string();
        self.pending.clear();
        // vim ex ranges: [%, N, N.M, ., $, +/-offsets] prefix the
        // command. Bare :N is goto-line.
        let (range, rest) = self.parse_ex_range(&cmdline);
        if let Some((_, _)) = range {
            self.run_ranged_ex(range.unwrap(), rest);
            return;
        }
        let (cmd, arg) = cmdline.split_once(' ').unwrap_or((cmdline.as_str(), ""));
        match cmd {
            _ if cmdline.starts_with('!') => self.shell_run(&cmdline[1..]),
            "w" | "w!" => {
                // vim: :w {file} writes under a new name and adopts it
                let r = if arg.is_empty() {
                    self.buf_mut().save(cmd == "w!")
                } else {
                    self.buf_mut().save_as(arg)
                };
                match r {
                    Ok(()) => {
                        crate::session::save(self);
                        self.message = "written".into();
                    }
                    Err(e) => self.message = format!("write failed: {e}"),
                }
            }
            "wq" | "wq!" => {
                // a failed save keeps the buffer open and dirty — never
                // close into data loss (0014 wave 1)
                match self.buf_mut().save(cmd == "wq!") {
                    Ok(()) => {
                        crate::session::save(self);
                        // vim: :wq closes the WINDOW like :q — the shared
                        // document lives on in other panes (0015)
                        self.close_pane_or_buffer(false);
                    }
                    Err(e) => self.message = format!("write failed: {e}"),
                }
            }
            "set" => {
                // vim's option surface, narrowly: ro/noro only for now
                match arg {
                    "ro" | "readonly" => {
                        self.buf_mut().readonly = true;
                        self.message = "readonly".into();
                    }
                    "noro" | "noreadonly" => {
                        self.buf_mut().readonly = false;
                        self.message = "writable".into();
                    }
                    _ => self.message = format!("unknown option: {arg}"),
                }
            }
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
            _ if cmdline.starts_with("s/") => {
                // :s without a range = the current line (vim)
                let line = self.buf().line_of(self.head());
                self.substitute_range(line, line, &cmdline[2..]);
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
                self.set_head(self.buf().line_start(n.saturating_sub(1).min(last)));
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
    // ---- handler shims for the command table (0008 stage 2) ---------

    pub(crate) fn repeat_search_pub(&mut self, invert: bool) {
        self.repeat_search(invert);
    }
    pub(crate) fn jump_hunk_pub(&mut self, forward: bool) {
        self.jump_hunk(forward);
    }
    pub(crate) fn search_word_under_cursor_pub(&mut self, backward: bool) {
        self.search_word_under_cursor(backward);
    }
    pub(crate) fn repeat_find_pub(&mut self, reverse: bool) {
        self.repeat_find(reverse);
    }
    /// "p P" row: the completing key picks before/after.
    pub(crate) fn paste_named_pub(&mut self, name: Option<char>, before: bool) {
        self.paste_named(name, before);
    }
    /// "J ." row: the completing key picks the command.
    pub(crate) fn join_or_repeat(&mut self, key: char) {
        if key == 'J' {
            self.join_lines_pub();
        } else {
            self.dot_repeat_pub();
        }
    }
    /// "i a A o O I" row: insert entries by key.
    pub(crate) fn insert_entry_pub(&mut self, key: char) {
        match key {
            'i' => self.enter_insert_from("i"),
            'a' => self.append(),
            'A' => self.append_eol(),
            'o' => self.open_below(),
            'O' => self.open_above(),
            'I' => self.alias("I", "^i"),
            _ => {}
        }
    }
    /// "v V" row.
    pub(crate) fn enter_visual_pub(&mut self, key: char) {
        self.enter_visual(key == 'V');
    }
}

mod dbg3 {
    #[test]
    fn editor_3x() {
        let mut e = crate::editor::Editor::new(strop_core::Buffer::from_text("abcde\n"));
        e.feed_text("3x");
        eprintln!("text {:?} msg {:?}", e.buf().rope.to_string(), e.message);
    }
}
