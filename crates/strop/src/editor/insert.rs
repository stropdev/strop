//! Insert mode: text in, Esc out. Every session is recorded so
//! dot-repeat can replay both the command and the inserted text.

use super::{Editor, Key, Mode};

impl Editor {
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
                let cur = self.cursor;
                self.buf_mut().insert(cur, "\n");
                self.cursor += 1;
                if let Some(rec) = &mut self.recording_insert {
                    rec.push('\n');
                }
            }
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
