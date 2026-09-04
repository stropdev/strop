//! strop-core: the buffer. A rope, byte-offset positions, edit ops.
//! No UI, no modes, no grammar — the thing everything else edits.

pub mod history;
pub mod id;

use history::{Edit, EditKind, History};
use ropey::Rope;

/// A text buffer. Positions are UTF-8 byte offsets, everywhere (0001 §5.1).
pub struct Buffer {
    pub rope: Rope,
    pub path: Option<String>,
    pub dirty: bool,
    /// Monotonic edit counter; async readers (git gutter) diff lazily.
    pub epoch: u64,
    /// Read-only views (git surfaces): motions/yank work, edits refuse.
    pub readonly: bool,
    /// Display name for virtual buffers (statusline shows "[scratch]"
    /// otherwise): "git log", "commit 1a2b3c", …
    pub name: Option<String>,
    /// Undo history (helix-style revision tree). Readonly buffers never
    /// record (their content is owned by jobs, not the user).
    pub history: History,
    /// Suppresses recording while applying undo/redo ops.
    pub replaying: bool,
    /// Disk mtime at load/last save — overwrite protection for `:w`.
    disk_stamp: Option<std::time::SystemTime>,
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
            epoch: 0,
            readonly: false,
            name: None,
            history: History::default(),
            replaying: false,
            disk_stamp: None,
        }
    }

    /// Open a file; a missing file is a new empty buffer with that path
    /// (vim semantics — `:w` creates it). Real I/O errors still error.
    pub fn open(path: &str) -> std::io::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let disk_stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        Ok(Self {
            rope: Rope::from_str(&text),
            path: Some(path.to_string()),
            dirty: false,
            epoch: 0,
            readonly: false,
            name: None,
            history: History::default(),
            replaying: false,
            disk_stamp,
        })
    }

    /// `:w` — atomic (temp + rename in the same dir), refuses to
    /// overwrite a file another process touched since we loaded it.
    /// `force` is `:w!`.
    pub fn save(&mut self, force: bool) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let current = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if !force && current.is_some() && current != self.disk_stamp {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "file changed on disk — :w! to force",
            ));
        }
        let target = std::path::Path::new(&path);
        let tmp = target.with_file_name(format!(
            ".strop-tmp-{}-{}",
            std::process::id(),
            target.file_name().and_then(|n| n.to_str()).unwrap_or("x")
        ));
        std::fs::write(&tmp, self.rope.to_string())?;
        if let Ok(meta) = std::fs::metadata(target) {
            // keep the file's permissions across the atomic swap
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        std::fs::rename(&tmp, target)?;
        self.disk_stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        self.dirty = false;
        Ok(())
    }
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Last *content* line index — a trailing newline's phantom empty
    /// line doesn't count (vim's G lands on real text).
    pub fn last_content_line(&self) -> usize {
        let mut l = self.len_lines().saturating_sub(1);
        if self.len_bytes() > 0 && self.byte(self.len_bytes() - 1) == b'\n' && l > 0 {
            l -= 1;
        }
        l
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

    /// Is `offset` a UTF-8 char boundary? ropey's `try_byte_to_char`
    /// maps a mid-char byte to its containing char without complaint —
    /// only the byte↔char roundtrip actually detects boundaries. (The
    /// pre-0.3.9 clamp trusted it and never clamped anything.)
    pub fn is_boundary(&self, offset: usize) -> bool {
        if offset == 0 || offset == self.len_bytes() {
            return true;
        }
        if offset > self.len_bytes() {
            return false;
        }
        match self.rope.try_byte_to_char(offset) {
            Ok(c) => self.rope.try_char_to_byte(c).is_ok_and(|b| b == offset),
            Err(_) => false,
        }
    }

    /// Clamp a byte offset down to a char boundary (the grapheme policy
    /// in 0001 §5.9 hardens this further when text goes wide).
    pub fn clamp_boundary(&self, mut offset: usize) -> usize {
        offset = offset.min(self.len_bytes());
        while offset > 0 && !self.is_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    /// Smallest char boundary >= offset. Byte arithmetic on a cursor
    /// (`cursor + 1` in x/a/r/~) lands inside a multibyte char; deleting
    /// or inserting there panics ropey. Round up, never down — a
    /// deletion that rounds down eats the previous char's tail.
    pub fn ceil_boundary(&self, mut offset: usize) -> usize {
        offset = offset.min(self.len_bytes());
        while offset < self.len_bytes() && !self.is_boundary(offset) {
            offset += 1;
        }
        offset
    }

    /// Slice as String — for register/paste paths, never for per-frame render.
    /// Stale ranges clamp (fuzz-driven cascades hand these around).
    pub fn slice_string(&self, range: Range) -> String {
        let start = range.start.min(self.len_bytes());
        let end = range.end.min(self.len_bytes());
        self.rope.byte_slice(start..end.max(start)).to_string()
    }

    /// Apply history edits (undo/redo replay — never recorded).
    pub fn apply_history(&mut self, ops: Vec<Edit>) {
        self.replaying = true;
        for op in ops {
            match op.kind {
                EditKind::Insert => {
                    let at = self.clamp_boundary(op.at.min(self.len_bytes()));
                    self.rope.insert(self.rope.byte_to_char(at), &op.text);
                }
                EditKind::Delete => {
                    // both bounds must land on char boundaries — a stale
                    // replay against drifted text panics ropey otherwise
                    let end = self.clamp_boundary((op.at + op.text.len()).min(self.len_bytes()));
                    let start = self.clamp_boundary(op.at.min(end));
                    if start < end {
                        self.rope
                            .remove(self.rope.byte_to_char(start)..self.rope.byte_to_char(end));
                    }
                }
            }
        }
        self.replaying = false;
        self.dirty = true;
        self.epoch += 1;
    }

    /// Replace the whole contents (user-facing path). Refuses on
    /// readonly buffers — the owning subsystem uses
    /// `replace_all_system`.
    pub fn replace_all(&mut self, text: &str) {
        if self.readonly {
            return;
        }
        self.replace_all_system(text);
    }

    /// The privileged replace for generated surfaces: their content is
    /// owned by jobs (git/LSP/shell), refreshed under the user's feet —
    /// the readonly guard is about *user* edits, not the owner.
    pub fn replace_all_system(&mut self, text: &str) {
        self.rope = Rope::from_str(text);
        self.epoch += 1;
    }

    /// Returns the deleted text (register payoff). Refuses on readonly
    /// buffers: the input layer checks first, but the mutation boundary
    /// enforces — no caller-remembered guard (0014).
    pub fn delete(&mut self, range: Range) -> String {
        if self.readonly && !self.replaying {
            return String::new();
        }
        // stale ranges (fuzz-driven cascades, replay drift) clamp, not panic
        let start = self.clamp_boundary(range.start.min(self.len_bytes()));
        let end = self.clamp_boundary(range.end.min(self.len_bytes()));
        if start >= end {
            return String::new();
        }
        let text = self.rope.byte_slice(start..end).to_string();
        // ropey mutates by CHAR index; our offsets are bytes
        let cstart = self.rope.byte_to_char(start);
        let cend = self.rope.byte_to_char(end);
        self.rope.remove(cstart..cend);
        self.dirty = true;
        self.epoch += 1;
        if !self.replaying && !self.readonly {
            self.history.record(
                Edit {
                    at: range.start,
                    text: text.clone(),
                    kind: EditKind::Insert,
                },
                Edit {
                    at: range.start,
                    text: text.clone(),
                    kind: EditKind::Delete,
                },
            );
        }
        text
    }

    pub fn insert(&mut self, at: usize, text: &str) {
        if self.readonly && !self.replaying {
            return;
        }
        self.rope.insert(self.rope.byte_to_char(at), text);
        self.dirty = true;
        self.epoch += 1;
        if !self.replaying && !self.readonly {
            self.history.record(
                Edit {
                    at,
                    text: text.into(),
                    kind: EditKind::Delete,
                },
                Edit {
                    at,
                    text: text.into(),
                    kind: EditKind::Insert,
                },
            );
        }
    }

    pub fn line_text(&self, line: usize) -> String {
        let start = self.line_start(line);
        let end = self.line_end(line);
        self.rope.byte_slice(start..end).to_string()
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[test]
    fn save_refuses_external_change_unless_forced() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, "original\n").unwrap();
        let mut b = Buffer::open(f.to_str().unwrap()).unwrap();
        b.insert(0, "mine ");
        // another process touches the file
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(&f, "theirs\n").unwrap();
        let err = b.save(false).unwrap_err();
        assert!(err.to_string().contains("changed on disk"));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "theirs\n");
        b.save(true).unwrap(); // :w!
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "mine original\n");
        assert!(!b.dirty);
    }

    #[test]
    fn save_is_atomic_and_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.sh");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o750)).unwrap();
        let mut b = Buffer::open(f.to_str().unwrap()).unwrap();
        b.insert(b.len_bytes(), "echo hi\n");
        b.save(false).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "#!/bin/sh\necho hi\n");
        let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750, "permissions survive the swap");
        // no temp litter
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn readonly_refuses_mutation_at_the_boundary() {
        // 0014: the guard lives in Buffer, not in every caller's memory
        let mut b = Buffer::from_text("abc\n");
        b.readonly = true;
        b.insert(0, "nope");
        let gone = b.delete(Range::charwise(0, 2));
        assert_eq!(gone, "");
        assert_eq!(b.rope.to_string(), "abc\n", "untouched");
        // the owner path still works (job-generated surfaces)
        b.replace_all_system("gen\n");
        assert_eq!(b.rope.to_string(), "gen\n");
    }
}
