//! Vim's cursor walk over a buffer. Vim's motions are written against a
//! cursor that can stand on a line's end (its NUL) and that steps from
//! there to the next line; porting them against that same model is what
//! keeps the edge cases (empty lines are words, a word that ends its
//! line) identical instead of approximately right.

use strop_core::Buffer;

/// What one step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Moved to another character of the same line.
    Char,
    /// Crossed into another line.
    Line,
    /// Stepped forward onto the line's end.
    End,
    /// The buffer's edge: nothing moved.
    Edge,
}

impl Step {
    /// Vim's `inc() >= 1`: the step left the line's characters.
    pub(crate) fn left_text(self) -> bool {
        matches!(self, Step::Line | Step::End)
    }
}

/// Blank (space, tab, the line's end), punctuation and keyword classes;
/// a WORD walk folds the last two together.
pub(crate) const BLANK: u8 = 0;
pub(crate) const PUNCT: u8 = 1;
pub(crate) const WORD: u8 = 2;

#[derive(Clone)]
pub(crate) struct Walk<'a> {
    pub(crate) buf: &'a Buffer,
    pub(crate) pos: usize,
    line: usize,
    start: usize,
    end: usize,
    last: usize,
}

impl<'a> Walk<'a> {
    pub(crate) fn new(buf: &'a Buffer, pos: usize) -> Self {
        let last = buf.last_content_line();
        let line = buf.line_of(pos).min(last);
        let (start, end) = (buf.line_start(line), buf.line_end(line));
        Self {
            buf,
            pos: buf.clamp_boundary(pos.clamp(start, end)),
            line,
            start,
            end,
            last,
        }
    }

    fn enter(&mut self, line: usize) {
        self.line = line;
        self.start = self.buf.line_start(line);
        self.end = self.buf.line_end(line);
    }

    pub(crate) fn line(&self) -> usize {
        self.line
    }
    pub(crate) fn col(&self) -> usize {
        self.pos - self.start
    }
    pub(crate) fn on_last_line(&self) -> bool {
        self.line == self.last
    }
    /// Column 0 of a line with no characters.
    pub(crate) fn on_empty_line(&self) -> bool {
        self.start == self.end
    }
    pub(crate) fn at_end(&self) -> bool {
        self.pos >= self.end
    }

    /// The character under the cursor; `None` on the line's end.
    pub(crate) fn ch(&self) -> Option<char> {
        if self.at_end() {
            return None;
        }
        let byte = self.buf.byte(self.pos);
        Some(if byte.is_ascii() {
            char::from(byte)
        } else {
            self.buf.text().char(self.buf.text().byte_to_char(self.pos))
        })
    }

    pub(crate) fn class(&self, big: bool) -> u8 {
        match self.ch() {
            None | Some(' ' | '\t') => BLANK,
            Some(_) if big => PUNCT,
            Some(c) if super::is_keyword(c) => WORD,
            Some(c) if c.is_whitespace() => BLANK,
            Some(_) => PUNCT,
        }
    }

    /// Vim's `inc()`.
    pub(crate) fn inc(&mut self) -> Step {
        if self.pos < self.end {
            self.pos = self.buf.ceil_boundary(self.pos + 1).min(self.end);
            return if self.pos < self.end {
                Step::Char
            } else {
                Step::End
            };
        }
        if self.line < self.last {
            self.enter(self.line + 1);
            self.pos = self.start;
            return Step::Line;
        }
        Step::Edge
    }

    /// Vim's `dec()`: from column 0 it lands on the previous line's end.
    pub(crate) fn dec(&mut self) -> Step {
        if self.pos > self.start {
            self.pos = self.buf.clamp_boundary(self.pos - 1).max(self.start);
            return Step::Char;
        }
        if self.line > 0 {
            self.enter(self.line - 1);
            self.pos = self.end;
            return Step::Line;
        }
        Step::Edge
    }

    /// Vim's `incl()`: `inc()` that does not rest on a non-empty line's end.
    pub(crate) fn incl(&mut self) -> Step {
        let step = self.inc();
        if step.left_text() && self.col() > 0 {
            return self.inc();
        }
        step
    }

    /// Vim's `decl()`: `dec()` that does not rest on a non-empty line's end.
    pub(crate) fn decl(&mut self) -> Step {
        let step = self.dec();
        if step == Step::Line && self.col() > 0 {
            return self.dec();
        }
        step
    }

    /// Jump to column 0 of `line`.
    pub(crate) fn goto_line(&mut self, line: usize) {
        self.enter(line.min(self.last));
        self.pos = self.start;
    }

    /// Vim's `inindent(extra)`: only blanks lie before column `col + extra`.
    pub(crate) fn in_indent(&self, extra: usize) -> bool {
        let mut blanks = 0;
        let mut at = self.start;
        while at < self.end && matches!(self.buf.byte(at), b' ' | b'\t') {
            blanks += 1;
            at += 1;
        }
        blanks >= self.col() + extra
    }
}

/// Whether a line holds only blanks (Vim's `linewhite`).
pub(crate) fn line_white(buf: &Buffer, line: usize) -> bool {
    let (start, end) = (buf.line_start(line), buf.line_end(line));
    (start..end).all(|at| matches!(buf.byte(at), b' ' | b'\t'))
}
