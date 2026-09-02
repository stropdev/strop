//! Insert mode: text in, Esc out. Every session is recorded so
//! dot-repeat can replay both the command and the inserted text.

use super::{Editor, Key, Mode};

impl Editor {
    /// Indent for a new line at the cursor (0001 daily-driver): copy the
    /// current line's leading whitespace, plus one level after an opener
    /// (`{[`(` c-like, `:` python-ish). Configurable width (config.toml).
    pub(crate) fn auto_indent(&self) -> String {
        let line = self.buf().line_of(self.cursor);
        let text = self.buf().line_text(line);
        let before_cursor = &text[..self.buf().col_of(self.cursor).min(text.len())];
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
        let line = self.buf().line_of(self.cursor);
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
                    let cur = self.cursor;
                    self.buf_mut()
                        .delete(strop_core::Range::charwise(prev, cur));
                    self.cursor = prev;
                    if let Some(rec) = &mut self.recording_insert {
                        rec.pop();
                    }
                }
            }
            Key::Enter => {
                let indent = self.auto_indent();
                let text = format!("\n{indent}");
                let cur = self.cursor;
                self.buf_mut().insert(cur, &text);
                self.cursor += text.len();
                if let Some(rec) = &mut self.recording_insert {
                    rec.push('\n');
                    rec.push_str(&indent);
                }
            }
            Key::Up | Key::Down | Key::Tab | Key::Backtab => {}
            Key::Char(c) => {
                let mut tmp = [0u8; 4];
                let cur = self.cursor;
                self.buf_mut().insert(cur, c.encode_utf8(&mut tmp));
                self.cursor += c.len_utf8();
                if let Some(rec) = &mut self.recording_insert {
                    rec.push(c);
                }
            }
        }
    }
}
