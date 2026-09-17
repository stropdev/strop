//! One-line input fields (picker query/replace, the `: / ? |` line) ARE
//! one-line buffers (0003 §2): the text lives in a real `Buffer` that the
//! grammar resolver borrows directly — pure motions never allocate. This
//! type owns the buffer, the caret and the modal flag plus the editing
//! primitives insert-mode typing needs. Normal-mode key sequences are
//! the engine's field machine (`strop_engine::editor::field`), which
//! resolves the real vim grammar against `buffer()` and drives these
//! primitives — no key table lives here.

use strop_core::id::BufferRevision;
use strop_core::{Buffer, Range};

/// A one-line text field: a real buffer, a caret, a vim mode flag.
pub struct LineEdit {
    /// The field's text as a persistent one-line buffer — grammar
    /// resolution reads it in place (a transient rope per keystroke
    /// would be an allocation storm; this one is built once per field).
    buf: Buffer,
    /// The render/rank projection of `buf` (one line, never a newline);
    /// every edit below rewrites both sides in the same operation.
    text: String,
    /// Byte offset of the caret (always on a char boundary).
    cursor: usize,
    /// True after Esc: vim normal mode within the field.
    pub normal: bool,
}

impl LineEdit {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        assert!(
            !text.contains(['\r', '\n']),
            "a line field cannot contain a newline"
        );
        let cursor = text.len();
        Self {
            buf: Buffer::from_text(&text),
            text,
            cursor,
            normal: false,
        }
    }

    /// The field text (the buffer's one line).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Caret byte offset into `text()` (always on a char boundary).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The backing buffer — the grammar machine resolves against it.
    pub fn buffer(&self) -> &Buffer {
        &self.buf
    }

    /// Bumped once per edit: callers diff it to learn "the text
    /// changed" without copying the line (one completed operator = one
    /// bump = one notification; pure motions never bump).
    pub fn revision(&self) -> BufferRevision {
        self.buf.revision()
    }

    /// Caret to a byte offset, clamped into the text and back onto a
    /// char boundary.
    pub fn set_cursor(&mut self, cursor: usize) {
        let mut cursor = cursor.min(self.text.len());
        while cursor > 0 && !self.text.is_char_boundary(cursor) {
            cursor -= 1;
        }
        self.cursor = cursor;
    }

    /// Reset the text; caret to the end (the common "new field" shape).
    /// A newline is a caller bug: bracketed paste refuses multi-line
    /// payloads at the editor edge, with a message, before they reach
    /// a field.
    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        assert!(
            !text.contains(['\r', '\n']),
            "a line field cannot contain a newline"
        );
        if self.buf.system_edit().replace_all(&text).is_err() {
            // revision space exhausted: start over on a fresh buffer
            self.buf = Buffer::from_text(&text);
        }
        self.cursor = text.len();
        self.text = text;
    }

    /// Insert at the caret (insert mode typing).
    pub fn insert_char(&mut self, c: char) {
        // Enter arrives as Key::Enter, never Char('\n') — the guard is
        // the single-line invariant, not input policy.
        if c == '\n' || c == '\r' {
            return;
        }
        let mut encoded = [0u8; 4];
        self.insert_str(c.encode_utf8(&mut encoded));
    }

    /// Insert a pasted payload at the caret (bracketed paste: one
    /// payload, no key interpretation — same shape as insert_char).
    pub fn insert_str(&mut self, text: &str) {
        assert!(
            !text.contains(['\r', '\n']),
            "a line field cannot contain a newline"
        );
        if text.is_empty() {
            return;
        }
        let at = self.cursor;
        if self.buf.edit().insert(at, text).is_ok() {
            self.text.insert_str(at, text);
            self.cursor = at + text.len();
        }
    }

    /// Delete before the caret (insert mode backspace).
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut start = self.cursor - 1;
        while !self.text.is_char_boundary(start) {
            start -= 1;
        }
        self.delete_range(start, self.cursor);
        self.set_cursor(start);
    }

    /// Delete [start, end) — the grammar machine's edit primitive: one
    /// buffer edit, one projection edit, one revision bump. Ends clamp
    /// into the text (a linewise range runs exactly to the buffer end).
    pub fn delete_range(&mut self, start: usize, end: usize) {
        let start = start.min(self.text.len());
        let end = end.max(start).min(self.text.len());
        if start == end {
            return;
        }
        if self.buf.edit().delete(Range::charwise(start, end)).is_ok() {
            self.text.drain(start..end);
            self.set_cursor(self.cursor);
        }
    }

    /// Caret one char left/right — arrow keys work in both modes.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.set_cursor(self.cursor - 1);
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            let mut next = self.cursor + 1;
            while next < self.text.len() && !self.text.is_char_boundary(next) {
                next += 1;
            }
            self.cursor = next;
        }
    }
}

impl Default for LineEdit {
    fn default() -> Self {
        Self::new("")
    }
}

impl Clone for LineEdit {
    fn clone(&self) -> Self {
        Self {
            buf: Buffer::from_text(&self.text),
            text: self.text.clone(),
            cursor: self.cursor,
            normal: self.normal,
        }
    }
}

impl std::fmt::Debug for LineEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LineEdit")
            .field("text", &self.text)
            .field("cursor", &self.cursor)
            .field("normal", &self.normal)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_backspace_and_caret_roundtrip() {
        let mut e = LineEdit::new("rg --json");
        e.insert_char('!'); // append at end
        assert_eq!(e.text(), "rg --json!");
        e.backspace();
        assert_eq!(e.text(), "rg --json");
        // multibyte carets stay on boundaries
        let mut e = LineEdit::new("héllo");
        e.set_cursor(0);
        e.move_right();
        assert_eq!(e.cursor(), 1);
        e.delete_range(1, 3); // 'é' is two bytes
        assert_eq!(e.text(), "hllo");
        assert_eq!(e.cursor(), 1);
        e.move_left();
        e.move_left();
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn edits_bump_the_revision_exactly_once() {
        let mut e = LineEdit::new("main");
        let clean = e.revision();
        e.set_cursor(0);
        e.move_right();
        assert_eq!(e.revision(), clean, "pure caret moves never bump");
        e.delete_range(0, 4);
        assert_eq!(e.revision().get(), clean.get() + 1, "one edit, one bump");
        assert_eq!(e.text(), "");
        e.insert_str("xy");
        assert_eq!(e.revision().get(), clean.get() + 2);
        assert_eq!(e.text(), "xy");
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn the_single_line_invariant_holds() {
        let mut e = LineEdit::new("main");
        e.insert_char('\n'); // unreachable from the key layer; ignored
        assert_eq!(e.text(), "main");
        e.set_text("reset");
        assert_eq!(e.text(), "reset");
        assert_eq!(e.cursor(), 5);
    }

    #[test]
    #[should_panic(expected = "cannot contain a newline")]
    fn set_text_refuses_newlines() {
        let mut e = LineEdit::default();
        e.set_text("two\nlines");
    }
}
