//! normal/execute.rs — the operator engine + editing entry points.

use strop_core::Range;
use strop_grammar::{self as grammar, Command, Op};

use crate::editor::{Editor, Key, Mode};

impl Editor {
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

    pub(super) fn execute(&mut self, cmd: &Command) {
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
    pub(super) fn replace_char_n(&mut self, c: char, count: usize) {
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
}
