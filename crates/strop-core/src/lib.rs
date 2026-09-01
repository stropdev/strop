//! strop-core: the buffer. A rope, byte-offset positions, edit ops.
//! No UI, no modes, no grammar — the thing everything else edits.

use ropey::Rope;

/// A text buffer. Positions are UTF-8 byte offsets, everywhere (0001 §5.1).
pub struct Buffer {
    pub rope: Rope,
    pub path: Option<String>,
    pub dirty: bool,
}

/// A half-open byte range `[start, end)` plus how vim thinks about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: usize,
    /// Inclusive (charwise) ranges include the char at `end - 1`'s semantic
    /// target already — the flag exists for the spec footer and linewise ops.
    pub linewise: bool,
}

impl Range {
    pub fn charwise(start: usize, end: usize) -> Self {
        debug_assert!(start <= end);
        Self {
            start,
            end,
            linewise: false,
        }
    }
    pub fn linewise(start: usize, end: usize) -> Self {
        debug_assert!(start <= end);
        Self {
            start,
            end,
            linewise: true,
        }
    }
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

impl Buffer {
    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            path: None,
            dirty: false,
        }
    }

    pub fn open(path: &str) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self {
            rope: Rope::from_str(&text),
            path: Some(path.to_string()),
            dirty: false,
        })
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        if let Some(path) = &self.path {
            std::fs::write(path, self.rope.to_string())?;
            self.dirty = false;
        }
        Ok(())
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Byte offset of the first char of `line` (0-indexed).
    pub fn line_start(&self, line: usize) -> usize {
        self.rope
            .line_to_byte(line.min(self.len_lines().saturating_sub(1)))
    }

    /// Byte offset one past the last content char of `line` (excludes `\n`).
    pub fn line_end(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let mut end = self.line_start((line + 1).min(self.len_lines().saturating_sub(1)));
        if line + 1 >= self.len_lines() {
            end = self.len_bytes();
        }
        // strip the trailing newline
        if end > start && self.byte(end - 1) == b'\n' {
            end -= 1;
        }
        end
    }

    pub fn line_of(&self, offset: usize) -> usize {
        self.rope.byte_to_line(offset.min(self.len_bytes()))
    }

    /// Column (in bytes) of `offset` within its line.
    pub fn col_of(&self, offset: usize) -> usize {
        offset - self.line_start(self.line_of(offset))
    }

    pub fn byte(&self, offset: usize) -> u8 {
        self.rope
            .byte(offset.min(self.len_bytes().saturating_sub(1)))
    }

    pub fn byte_at(&self, offset: usize) -> Option<u8> {
        if offset < self.len_bytes() {
            Some(self.rope.byte(offset))
        } else {
            None
        }
    }

    /// Clamp a byte offset to a char boundary (prototype is ASCII-honest;
    /// the grapheme policy in 0001 §5.9 hardens this when text goes wide).
    pub fn clamp_boundary(&self, mut offset: usize) -> usize {
        offset = offset.min(self.len_bytes());
        while offset > 0 && self.rope.try_byte_to_char(offset).is_err() {
            offset -= 1;
        }
        offset
    }

    /// Slice as String — for register/paste paths, never for per-frame render.
    pub fn slice_string(&self, range: Range) -> String {
        self.rope.byte_slice(range.start..range.end).to_string()
    }

    /// Returns the deleted text (register payoff).
    pub fn delete(&mut self, range: Range) -> String {
        let text = self.slice_string(range);
        self.rope.remove(range.start..range.end);
        self.dirty = true;
        text
    }

    pub fn insert(&mut self, at: usize, text: &str) {
        self.rope.insert(self.clamp_boundary(at), text);
        self.dirty = true;
    }

    pub fn line_text(&self, line: usize) -> String {
        let start = self.line_start(line);
        let end = self.line_end(line);
        self.rope.byte_slice(start..end).to_string()
    }
}
