//! Diff data: hunks, line origins, signs — the typed model every
//! git surface renders from (0010).

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    Add,
    Change,
    Delete,
}

/// Where a diff line comes from — addition/deletion carry which side's
/// line number applies (0010 §1: typed origins, never `+`-sniffing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOrigin {
    Context,
    Addition,
    Deletion,
}

/// One line of a hunk: content without prefix, plus the 1-based line
/// number on each side that has one (absent side: `None`, never `0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub origin: LineOrigin,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
    /// The line's bytes WITHOUT its terminator (0018: byte-precise —
    /// a `\r` stays; non-UTF-8 stays bytes).
    pub text: Vec<u8>,
    /// Whether the source line ended in a newline — the missing-final-
    /// newline marker is data, not a patch-text nuance (0018).
    pub has_newline: bool,
}

impl DiffLine {
    /// Display form (lossy at the render edge only).
    pub fn text_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.text)
    }
    /// The line's bytes WITH its terminator, exactly as stored.
    pub fn bytes_with_terminator(&self) -> Vec<u8> {
        let mut b = self.text.clone();
        if self.has_newline {
            b.push(b'\n');
        }
        b
    }
}

/// One diff hunk between two versions of a file, in 1-based lines.
#[derive(Debug, Clone)]
pub struct Hunk {
    pub kind: HunkKind,
    /// First affected line in the new version (1-based). For pure
    /// deletions this is the line *after* which content vanished.
    pub new_start: usize,
    pub new_count: usize,
    pub old_start: usize,
    pub old_count: usize,
    pub lines: Vec<DiffLine>,
}

/// One file's diff at a commit (vs its parent): the delta view's data.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: PathBuf,
    pub hunks: Vec<Hunk>,
    pub added: usize,
    pub deleted: usize,
}

/// One changed line, for gutter signs. Hunk headers include context
/// lines, so signs track the +/- lines, not the header range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    /// Buffer line was added or changed.
    AddOrChange,
    /// Buffer line sits right below a deletion (the line number may be
    /// one past the buffer end for an EOF deletion — clamp on render).
    DeleteAfter,
}

impl Hunk {
    /// Signs this hunk produces, derived from its line origins.
    pub fn signs(&self) -> Vec<(usize, Sign)> {
        let mut out = Vec::new();
        let mut nl = self.new_start;
        for line in &self.lines {
            match line.origin {
                LineOrigin::Addition => {
                    out.push((nl, Sign::AddOrChange));
                    nl += 1;
                }
                LineOrigin::Deletion => out.push((nl, Sign::DeleteAfter)),
                LineOrigin::Context => nl += 1,
            }
        }
        out
    }

    /// The actual changed region (from add/del lines, not the header,
    /// which includes context): new-side `new_first`/`new_count`
    /// (1-based) and old-side `old_first`/`old_count`. For pure
    /// deletions `new_first` is the new line *following* the gap.
    pub fn changed_region(&self) -> (usize, usize, usize, usize) {
        let mut nl = self.new_start;
        let mut ol = self.old_start;
        let mut new_lines = Vec::new();
        let mut old_lines = Vec::new();
        for line in &self.lines {
            match line.origin {
                LineOrigin::Addition => {
                    new_lines.push(nl);
                    nl += 1;
                }
                LineOrigin::Deletion => {
                    old_lines.push(ol);
                    ol += 1;
                }
                LineOrigin::Context => {
                    nl += 1;
                    ol += 1;
                }
            }
        }
        let new_first = new_lines.first().copied().unwrap_or(nl);
        let old_first = old_lines.first().copied().unwrap_or(ol);
        (new_first, new_lines.len(), old_first, old_lines.len())
    }

    /// Buffer lines covered (signs render on these); `total_lines`
    /// clamps an EOF deletion onto the last line.
    pub fn covers(&self, line_1based: usize, total_lines: usize) -> bool {
        self.signs().iter().any(|&(l, kind)| match kind {
            Sign::AddOrChange => l == line_1based,
            Sign::DeleteAfter => l.min(total_lines) == line_1based,
        })
    }

    /// The `@@ -a,b +c,d @@` header row as the diff surface shows it.
    pub fn header(&self) -> String {
        format!(
            "@@ -{},{} +{},{} @@",
            self.old_start, self.old_count, self.new_start, self.new_count
        )
    }

    /// Assemble a hunk from its header numbers and typed lines; the
    /// kind comes from the actual origins — header counts include
    /// context lines, which would mislabel small-file hunks.
    pub fn build(
        old_start: usize,
        old_count: usize,
        new_start: usize,
        new_count: usize,
        lines: Vec<DiffLine>,
    ) -> Self {
        let has_add = lines.iter().any(|l| l.origin == LineOrigin::Addition);
        let has_del = lines.iter().any(|l| l.origin == LineOrigin::Deletion);
        let kind = match (has_add, has_del) {
            (true, false) => HunkKind::Add,
            (false, true) => HunkKind::Delete,
            _ => HunkKind::Change,
        };
        Hunk {
            kind,
            new_start,
            new_count,
            old_start,
            old_count,
            lines,
        }
    }
}
