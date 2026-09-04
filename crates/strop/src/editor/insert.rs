//! Insert mode: text in, Esc out. Every session is recorded so
//! dot-repeat can replay both the command and the inserted text.

use super::{Editor, Key, Mode};

impl Editor {
    /// One-level dedent when the line so far is whitespace-only (called
    /// before inserting a closer). No-op when there is real text or no
    /// indent to give back.
    fn dedent_for_closer(&mut self) {
        let line = self.buf().line_of(self.head());
        let start = self.buf().line_start(line);
        let col = self.buf().col_of(self.head());
        let text = self.buf().line_text(line);
        let before: String = text.chars().take(col).collect();
        if !before.chars().all(|c| c == ' ' || c == '\t') {
            return; // real text before the cursor — never reindent
        }
        let width = self.config.tab_size;
        let mut strip = 0;
        while strip < width && start + strip < self.head() && self.buf().byte(start + strip) == b' '
        {
            strip += 1;
        }
        if strip == 0 {
            return;
        }
        self.buf_mut()
            .delete(strop_core::Range::charwise(start, start + strip));
        self.set_head(self.head().saturating_sub(strip));
        // dot-repeat: the dedent belongs to the same change; the recorded
        // insert text keeps literal content, replay re-derives via the
        // same smartindent path
    }

    /// Indent for a new line at the cursor (0001 daily-driver): copy the
    /// current line's leading whitespace, plus one level after an opener
    /// (`{[`(` c-like, `:` python-ish). Configurable width (config.toml).
    pub(crate) fn auto_indent(&self) -> String {
        let line = self.buf().line_of(self.head());
        let text = self.buf().line_text(line);
        let before_cursor = &text[..self.buf().col_of(self.head()).min(text.len())];
        let base: String = before_cursor
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let trimmed = before_cursor.trim_end();
        let deeper = trimmed.ends_with('{')
            || trimmed.ends_with('[')
            || trimmed.ends_with('(')
            || trimmed.ends_with(':');
        let mut indent = base;
        if deeper {
            indent.push_str(&self.config.indent());
        }
        indent
    }

    /// Indent for o/O: the current line's full leading whitespace,
    /// deepened after an opener even mid-line.
    pub(crate) fn auto_indent_full_line(&self) -> String {
        let line = self.buf().line_of(self.head());
        let text = self.buf().line_text(line);
        let base: String = text
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let trimmed = text.trim_end();
        let deeper = trimmed.ends_with('{')
            || trimmed.ends_with('[')
            || trimmed.ends_with('(')
            || trimmed.ends_with(':');
        let mut indent = base;
        if deeper {
            indent.push_str(&self.config.indent());
        }
        indent
    }

    /// Enter insert mode and start recording (dot-repeat, 0001 §2.1).
    /// `keys` is what got us here (`i`, `o`, `ci[`, …).
    pub(crate) fn enter_insert_from(&mut self, keys: &str) {
        // an insert session is one undo unit (with the change op, if any, that
        // opened it); plain entries open a fresh transaction
        self.tx_begin();
        if !matches!(keys, "o" | "O") {
            self.insert_open = None;
        }
        self.mode = Mode::Insert;
        self.recording_insert = Some(String::new());
        if self.last_cmd_keys.is_empty()
            || !matches!(keys.chars().next(), Some('c') if keys.len() > 1)
        {
            // plain entries remember their own key; change ops already set
            // last_cmd_keys in execute()
            if matches!(keys, "i" | "a" | "A" | "o" | "O" | "v..." | "V...") {
                self.last_cmd_keys = keys.trim_end_matches("...").into();
            }
        }
    }

    pub(crate) fn feed_insert(&mut self, key: Key) {
        match key {
            Key::CtrlW | Key::CtrlX | Key::CtrlD | Key::CtrlO => {}
            Key::Esc => {
                self.mode = Mode::Normal;
                self.set_head(self.head().saturating_sub(1));
                let extras: Vec<usize> = self
                    .extra_selections()
                    .iter()
                    .map(|s| s.head.saturating_sub(1))
                    .collect();
                self.sels.set_extras(extras);
                // vim insert counts: `3iX` types X three times — the
                // replay joins the session's undo unit (commit after)
                let count = std::mem::replace(&mut self.insert_count, 1);
                if let Some(rec) = self.recording_insert.take() {
                    if count > 1 {
                        let open = self.insert_open.take();
                        for _ in 1..count {
                            if let Some(o) = &open {
                                // o/O: the opened line repeats too
                                let at = (self.head() + 1).min(self.buf().len_bytes());
                                self.buf_mut().insert(at, o);
                                self.set_head(at + o.len().saturating_sub(1));
                            }
                            for ch in rec.chars() {
                                let at = (self.head() + 1).min(self.buf().len_bytes());
                                self.buf_mut().insert(at, &ch.to_string());
                                self.set_head(at + ch.len_utf8().saturating_sub(1));
                            }
                        }
                    }
                    self.last_insert = Some(rec);
                } else {
                    self.insert_open = None;
                }
                self.clamp_cursor();
                self.normalize_cursors();
                self.tx_commit(); // the insert session is one undo unit
            }
            Key::Backspace => {
                // cascade: every cursor deletes one char back, bottom-up
                // so positions never shift mid-batch (0013 §3)
                let mut positions = self.all_cursors();
                positions.sort_unstable();
                positions.dedup();
                positions.retain(|&p| p > 0); // cursors at 0 can't delete
                for &pos in positions.iter().rev() {
                    self.buf_mut()
                        .delete(strop_core::Range::charwise(pos - 1, pos));
                }
                if !positions.is_empty() {
                    self.remap_after_mirrored_edit(&positions, -1);
                    if let Some(rec) = &mut self.recording_insert {
                        rec.pop();
                    }
                }
            }
            Key::Enter => {
                let indent = self.auto_indent();
                let text = format!("\n{indent}");
                let mut positions = self.all_cursors();
                positions.sort_unstable();
                positions.dedup(); // stacked cursors edit once
                for &pos in positions.iter().rev() {
                    self.buf_mut().insert(pos, &text);
                }
                self.remap_after_mirrored_edit(&positions, text.len() as isize);
                if let Some(rec) = &mut self.recording_insert {
                    rec.push('\n');
                    rec.push_str(&indent);
                }
            }
            // arrows move in insert too (vim) — char-boundary honest
            Key::Left => {
                let start = self.buf().line_start(self.buf().line_of(self.head()));
                if self.head() > start {
                    self.set_head(self.buf().clamp_boundary(self.head() - 1));
                }
            }
            Key::Right => {
                let end = self.buf().line_end(self.buf().line_of(self.head()));
                if self.head() < end {
                    self.set_head(self.buf().ceil_boundary(self.head() + 1));
                }
            }
            Key::Up | Key::Down => {
                let line = self.buf().line_of(self.head());
                let target = if key == Key::Up {
                    line.saturating_sub(1)
                } else {
                    (line + 1).min(self.buf().len_lines().saturating_sub(1))
                };
                let col = self
                    .buf()
                    .col_of(self.head())
                    .min(self.buf().line_end(target) - self.buf().line_start(target));
                self.set_head(
                    self.buf()
                        .clamp_boundary(self.buf().line_start(target) + col),
                );
            }
            Key::CtrlR | Key::Tab | Key::Backtab => {}
            Key::Char(c) => {
                // smartindent (vim/helix behavior): a closer typed on an
                // indent-only line dedents one level first — typing `}`
                // after the auto-indent deepens never strands you
                if matches!(c, '}' | ']' | ')') {
                    self.dedent_for_closer();
                }
                let mut tmp = [0u8; 4];
                let encoded = c.encode_utf8(&mut tmp).to_string();
                let mut positions = self.all_cursors();
                positions.sort_unstable();
                positions.dedup(); // stacked cursors edit once
                for &pos in positions.iter().rev() {
                    self.buf_mut().insert(pos, &encoded);
                }
                self.remap_after_mirrored_edit(&positions, c.len_utf8() as isize);
                if let Some(rec) = &mut self.recording_insert {
                    rec.push(c);
                }
            }
        }
    }
}
