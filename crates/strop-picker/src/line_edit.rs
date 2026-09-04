//! Modal line editing (rootle's input boxes): one-line text fields that
//! land in insert mode and, on Esc, behave like vim normal mode — h/l
//! move, 0/$ ends, w/b words, x/X delete, i/a/A return to insert. Shared
//! by the picker fields and the ex/search line (one implementation, no
//! forks).

/// A one-line text field with a cursor and a vim mode.
#[derive(Debug, Clone, Default)]
pub struct LineEdit {
    pub text: String,
    /// Byte offset of the caret (always on a char boundary).
    pub cursor: usize,
    /// True after Esc: vim normal mode within the field.
    pub normal: bool,
}

impl LineEdit {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            text,
            cursor,
            normal: false,
        }
    }

    /// Reset the text; caret to the end (the common "new field" shape).
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
    }

    /// Insert at the caret (insert mode typing).
    pub fn insert_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Delete before the caret (insert mode backspace).
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    /// One normal-mode key. Returns false for keys LineEdit doesn't know
    /// (the caller routes them).
    pub fn normal_key(&mut self, c: char) -> bool {
        let len = self.text.len();
        // backward for "move here", forward for "delete up to here"
        let at_boundary = |s: &str, mut i: usize| {
            while i > 0 && !s.is_char_boundary(i) {
                i -= 1;
            }
            i
        };
        let next_boundary = |s: &str, mut i: usize| {
            while i < s.len() && !s.is_char_boundary(i) {
                i += 1;
            }
            i
        };
        match c {
            'h' => {
                if self.cursor > 0 {
                    self.cursor = at_boundary(&self.text, self.cursor - 1)
                }
            }
            'l' => {
                if self.cursor < len {
                    self.cursor = next_boundary(&self.text, self.cursor + 1)
                }
            }
            '0' => self.cursor = 0,
            '$' => self.cursor = len,
            'w' => {
                // next word start
                let mut i = self.cursor;
                while i < len && !self.text.as_bytes()[i].is_ascii_whitespace() {
                    i += 1;
                }
                while i < len && self.text.as_bytes()[i].is_ascii_whitespace() {
                    i += 1;
                }
                self.cursor = i;
            }
            'b' => {
                // previous word start
                let mut i = self.cursor.saturating_sub(1);
                while i > 0 && self.text.as_bytes()[i].is_ascii_whitespace() {
                    i -= 1;
                }
                while i > 0 && !self.text.as_bytes()[i - 1].is_ascii_whitespace() {
                    i -= 1;
                }
                self.cursor = i;
            }
            'x' => {
                if self.cursor < len {
                    let next = next_boundary(&self.text, self.cursor + 1);
                    self.text.drain(self.cursor..next);
                }
            }
            'X' => self.backspace(),
            'i' => self.normal = false,
            'a' => {
                if self.cursor < len {
                    self.cursor = next_boundary(&self.text, self.cursor + 1);
                }
                self.normal = false;
            }
            'A' => {
                self.cursor = len;
                self.normal = false;
            }
            _ => return false,
        }
        self.cursor = at_boundary(&self.text, self.cursor.min(len));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_editing_roundtrip() {
        let mut e = LineEdit::new("rg --json");
        e.insert_char('!'); // append at end
        assert_eq!(e.text, "rg --json!");
        e.backspace();
        assert_eq!(e.text, "rg --json");
        e.normal = true;
        e.normal_key('w'); // "--json" is one whitespace word: lands at end
        e.normal_key('b'); // back to the word's start (the '-')
        e.normal_key('x'); // delete it
        assert_eq!(e.text, "rg -json");
        e.normal_key('$');
        e.normal_key('a'); // append mode at end
        assert!(!e.normal);
        e.insert_char('!');
        assert_eq!(e.text, "rg -json!");
        // multibyte carets stay on boundaries
        let mut e = LineEdit::new("héllo");
        e.normal = true;
        e.normal_key('0');
        e.normal_key('l');
        e.normal_key('x');
        assert_eq!(e.text, "hllo");
    }
}
