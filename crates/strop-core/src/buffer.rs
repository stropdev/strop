//! The buffer: a rope, byte-offset positions, edit ops, persistence.
//! No UI, no modes, no grammar — the thing everything else edits.

mod io;
pub use io::{SaveReceipt, SaveRequest};
mod seed;
pub use seed::BufferSeed;
mod mutation;
use crate::diagnostics::BufferTraceId;
use crate::history::History;
use crate::range::Range;
use crate::{id, layout};
pub use mutation::{
    Change, ChangeOrigin, EditError, HistoryMove, PreparedReplacements, Replacement, SystemEdit,
    UserEdit,
};
use ropey::Rope;

/// A text buffer. Positions are UTF-8 byte offsets, everywhere (0001 §5.1).
pub struct Buffer {
    pub(crate) trace_identity: BufferTraceId,
    rope: Rope,
    /// Filesystem identity (0021 §3: Unix filenames aren't UTF-8 — a
    /// String path makes the filesystem model a UI model). Display via
    /// to_string_lossy at the edge only.
    pub path: Option<std::path::PathBuf>,
    pub dirty: bool,
    /// Monotonic edit counter; async readers (git gutter) diff lazily.
    epoch: u64,
    /// Read-only views (git surfaces): motions/yank work, edits refuse.
    pub readonly: bool,
    /// Display name for virtual buffers (statusline shows "[scratch]"
    /// otherwise): "git log", "commit 1a2b3c", …
    pub name: Option<String>,
    /// Undo history (helix-style revision tree). Readonly buffers never
    /// record (their content is owned by jobs, not the user).
    history: History,
    changes: Vec<Change>,
    /// Disk mtime at load/last save — overwrite protection for `:w`.
    disk_stamp: Option<std::time::SystemTime>,
    file_identity: Option<std::path::PathBuf>,
}

impl Buffer {
    pub fn text(&self) -> &Rope {
        &self.rope
    }
    pub fn snapshot(&self) -> Rope {
        self.rope.clone()
    }
    pub fn history(&self) -> &History {
        &self.history
    }
    pub fn revision(&self) -> id::BufferRevision {
        id::BufferRevision::new(self.epoch)
    }
    pub fn file_identity(&self) -> Option<&std::path::Path> {
        self.file_identity.as_deref()
    }

    pub fn restore_history(
        &mut self,
        history: History,
    ) -> Result<(), crate::history::HistoryError> {
        history.validate_for(&self.rope)?;
        self.history = history;
        Ok(())
    }

    pub fn from_text(text: &str) -> Self {
        Self {
            trace_identity: BufferTraceId::next(),
            rope: Rope::from_str(text),
            path: None,
            dirty: false,
            epoch: 0,
            readonly: false,
            name: None,
            history: History::default(),
            changes: Vec::new(),
            disk_stamp: None,
            file_identity: None,
        }
    }

    /// Open a file; a missing file is a new empty buffer with that path
    /// (vim semantics — `:w` creates it). Real I/O errors still error.
    pub fn open(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let (rope, disk_stamp) = match std::fs::File::open(path) {
            Ok(file) => {
                let stamp = file.metadata()?.modified()?;
                (Rope::from_reader(file)?, Some(stamp))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Rope::new(), None),
            Err(e) => return Err(e),
        };
        Ok(Self {
            trace_identity: BufferTraceId::next(),
            rope,
            path: Some(path.to_path_buf()),
            dirty: false,
            epoch: 0,
            readonly: false,
            name: None,
            history: History::default(),
            changes: Vec::new(),
            disk_stamp,
            file_identity: Some(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())),
        })
    }

    /// Display CELL of an offset within its line (0017/R6): cursor placement
    /// and overlays need terminal cells, not byte columns — wide chars and
    /// tabs make the difference. Streams through the containing cluster only:
    /// no whole-line String, no layout vector, no u16 saturation.
    pub fn cell_col_with_tab(
        &self,
        offset: impl Into<id::ByteOffset>,
        tab: usize,
    ) -> id::DisplayColumn {
        let offset = offset.into().get().min(self.len_bytes());
        let line = self.line_of(offset);
        let start = self.line_start(line);
        let text = self.text().byte_slice(start..self.line_end(line));
        let byte = offset.saturating_sub(start).min(text.len_bytes());
        let mut end = id::DisplayColumn::new(0);
        for (span, cluster) in layout::RopeGraphemes::new(text, tab) {
            if byte < span.byte + cluster.len() {
                return span.cell;
            }
            end = span.cell + span.width;
        }
        end
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
    pub fn line_start(&self, line: impl Into<id::LineIndex>) -> usize {
        self.rope
            .line_to_byte(line.into().get().min(self.len_lines().saturating_sub(1)))
    }

    /// Byte offset one past the last content char (excludes LF or CRLF).
    pub fn line_end(&self, line: impl Into<id::LineIndex>) -> usize {
        let line = line.into().get();
        let start = self.line_start(line);
        let mut end = self.line_start((line + 1).min(self.len_lines().saturating_sub(1)));
        if line + 1 >= self.len_lines() {
            end = self.len_bytes();
        }
        // strip the trailing newline
        if end > start && self.byte(end - 1) == b'\n' {
            end -= 1;
            if end > start && self.byte(end - 1) == b'\r' {
                end -= 1;
            }
        }
        end
    }

    pub fn line_of(&self, offset: impl Into<id::ByteOffset>) -> usize {
        self.rope
            .byte_to_line(offset.into().get().min(self.len_bytes()))
    }

    /// Column (in bytes) of `offset` within its line.
    pub fn col_of(&self, offset: impl Into<id::ByteOffset>) -> usize {
        let offset = offset.into();
        offset.get() - self.line_start(self.line_of(offset))
    }
    /// Byte at a position. An empty rope reads as NUL: every classifier
    /// treats NUL as a boundary, and the alternative (a panic) is how
    /// the second review found this (0015). `byte_at` when absence
    /// itself matters.
    pub fn byte(&self, offset: impl Into<id::ByteOffset>) -> u8 {
        if self.len_bytes() == 0 {
            return 0;
        }
        self.rope
            .byte(offset.into().get().min(self.len_bytes().saturating_sub(1)))
    }

    pub fn byte_at(&self, offset: impl Into<id::ByteOffset>) -> Option<u8> {
        let off = offset.into().get();
        if off < self.len_bytes() {
            Some(self.rope.byte(off))
        } else {
            None
        }
    }

    /// Is `offset` a UTF-8 char boundary? ropey's `try_byte_to_char`
    /// maps a mid-char byte to its containing char without complaint —
    /// only the byte↔char roundtrip actually detects boundaries. (The
    /// pre-0.3.9 clamp trusted it and never clamped anything.)
    pub fn is_boundary(&self, offset: impl Into<id::ByteOffset>) -> bool {
        let off = offset.into().get();
        if off == 0 || off == self.len_bytes() {
            return true;
        }
        if off > self.len_bytes() {
            return false;
        }
        match self.rope.try_byte_to_char(off) {
            Ok(c) => self.rope.try_char_to_byte(c).is_ok_and(|b| b == off),
            Err(_) => false,
        }
    }

    /// Clamp a byte offset down to a char boundary (the grapheme policy
    /// in 0001 §5.9 hardens this further when text goes wide).
    pub fn clamp_boundary(&self, offset: impl Into<id::ByteOffset>) -> usize {
        let mut offset = offset.into().get().min(self.len_bytes());
        while offset > 0 && !self.is_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    /// Smallest char boundary >= offset. Byte arithmetic on a cursor
    /// (`cursor + 1` in x/a/r/~) lands inside a multibyte char; deleting
    /// or inserting there panics ropey. Round up, never down — a
    /// deletion that rounds down eats the previous char's tail.
    pub fn ceil_boundary(&self, offset: impl Into<id::ByteOffset>) -> usize {
        let mut offset = offset.into().get().min(self.len_bytes());
        while offset < self.len_bytes() && !self.is_boundary(offset) {
            offset += 1;
        }
        offset
    }

    /// Slice as String — for register/paste paths, never for per-frame render.
    /// Stale ranges clamp (fuzz-driven cascades hand these around).
    pub fn slice_string(&self, range: Range) -> String {
        let start = self.clamp_boundary(range.start);
        let end = self.clamp_boundary(range.end);
        self.rope.byte_slice(start..end.max(start)).to_string()
    }

    pub fn line_text(&self, line: impl Into<id::LineIndex>) -> String {
        let line = line.into().get();
        let start = self.line_start(line);
        let end = self.line_end(line);
        self.rope.byte_slice(start..end).to_string()
    }
}

/// Pre-edit and post-edit geometry recorded at the instant text changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start_point: (usize, usize),
    pub old_end_point: (usize, usize),
    pub new_end_point: (usize, usize),
}

impl Buffer {
    /// (line, col) of a byte offset, as tree-sitter Points.
    pub fn point_of(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.len_bytes());
        (self.line_of(offset), self.col_of(offset))
    }

    /// The (line, col) extent of a text fragment.
    fn point_extent(text: &str) -> (usize, usize) {
        let lines = text.bytes().filter(|b| *b == b'\n').count();
        let col = if lines == 0 {
            text.len()
        } else {
            text.rsplit('\n').next().map(str::len).unwrap_or(0)
        };
        (lines, col)
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
        b.edit().insert(id::ByteOffset::new(0), "mine ").unwrap();
        // another process touches the file
        std::fs::write(&f, "theirs\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(123))
            .unwrap();
        let err = b.prepare_save(None, false).unwrap().execute().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "theirs\n");
        let receipt = b.prepare_save(None, true).unwrap().execute().unwrap();
        assert!(b.accept_save(receipt));
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
        let end = b.len_bytes();
        b.edit()
            .insert(id::ByteOffset::new(end), "echo hi\n")
            .unwrap();
        let receipt = b.prepare_save(None, false).unwrap().execute().unwrap();
        assert!(b.accept_save(receipt));
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
        assert_eq!(b.edit().insert(0, "nope"), Err(EditError::ReadOnly));
        assert_eq!(
            b.edit().delete(Range::charwise(0, 2)),
            Err(EditError::ReadOnly)
        );
        assert_eq!(b.rope.to_string(), "abc\n", "untouched");
        // the owner path still works (job-generated surfaces)
        b.system_edit().replace_all("gen\n").unwrap();
        assert_eq!(b.rope.to_string(), "gen\n");
    }
    #[test]
    fn non_utf8_filename_opens_and_roundtrips() {
        // 0021 §3: the filesystem is not UTF-8 — a weird name must open,
        // save, and keep its identity
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let weird = dir
            .path()
            .join(std::ffi::OsStr::from_bytes(b"weird-\xff.rs"));
        std::fs::write(&weird, "fn main() {}\n").unwrap();
        let mut b = Buffer::open(&weird).unwrap();
        assert_eq!(b.path.as_deref(), Some(weird.as_path()));
        b.edit().insert(0, "// x\n").unwrap();
        let receipt = b.prepare_save(None, false).unwrap().execute().unwrap();
        assert!(b.accept_save(receipt));
        assert_eq!(
            std::fs::read_to_string(&weird).unwrap(),
            "// x\nfn main() {}\n"
        );
    }
}
