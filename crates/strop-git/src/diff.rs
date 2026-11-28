//! Diff data: hunks, line origins, signs — the typed model every
//! git surface renders from (0010).

use std::path::PathBuf;
use strop_kernel::lines::{self, Origin, Region};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HunkKind {
    Add,
    Change,
    Delete,
}

/// Where a diff line comes from — addition/deletion carry which side's
/// line number applies (0010 §1: typed origins, never `+`-sniffing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LineOrigin {
    Context,
    Addition,
    Deletion,
}

/// One line of a hunk: content without prefix, plus the 1-based line
/// number on each side that has one (absent side: `None`, never `0`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileDiff {
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
    pub hunks: Vec<Hunk>,
    pub added: usize,
    pub deleted: usize,
}

impl FileDiff {
    /// Assemble the delta from typed hunks, deriving the added/deleted
    /// counts from line origins — the one constructor both the local
    /// libgit2 path and the remote blob-diff path feed.
    pub(crate) fn from_hunks(path: PathBuf, hunks: Vec<Hunk>) -> Self {
        let added = hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.origin == LineOrigin::Addition)
            .count();
        let deleted = hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.origin == LineOrigin::Deletion)
            .count();
        Self {
            path,
            hunks,
            added,
            deleted,
        }
    }
}

/// One changed line, for gutter signs. Hunk headers include context
/// lines, so signs track the +/- lines, not the header range; and a
/// hunk may hold several change runs (libgit2 merges nearby changes),
/// so the kind is decided per run, never per hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    /// Buffer line was added (its run has no deletions).
    Add,
    /// Buffer line replaces deleted lines (its run has both).
    Change,
    /// Buffer line sits right below a deletion. The line may be one past
    /// the last row (deletion at the end) — the gutter clamps it onto a
    /// content row.
    DeleteAfter,
}

impl Sign {
    /// The signs one change region puts on 0-based new-side lines:
    /// additions only → [`Sign::Add`] on each added line; deletions only →
    /// one [`Sign::DeleteAfter`] on the line the gap sits before; both →
    /// [`Sign::Change`] on each added line, no separate deletion sign.
    pub fn of_region(region: &Region) -> impl Iterator<Item = (usize, Sign)> {
        let sign = if region.old_len == 0 {
            Sign::Add
        } else {
            Sign::Change
        };
        let deletion = (region.new_len == 0).then_some((region.new, Sign::DeleteAfter));
        (region.new..region.new + region.new_len)
            .map(move |line| (line, sign))
            .chain(deletion)
    }
}

/// The 0-based first line of one side of a hunk header: a side with lines
/// starts at `start - 1`; an empty side names the line *before* the gap,
/// so the gap's 0-based position is `start` itself (`@@ -0,0 +1,3 @@`
/// inserts at position 0).
fn first_line(start: usize, count: usize) -> usize {
    if count == 0 {
        start
    } else {
        start.saturating_sub(1)
    }
}

impl Hunk {
    /// The 0-based new-side line the hunk's header starts at, context
    /// included (an empty new side: the line the gap sits before).
    pub fn new_first(&self) -> usize {
        first_line(self.new_start, self.new_count)
    }

    /// Append this hunk's change regions in 0-based lines, one per maximal
    /// stretch of non-context lines (`strop_kernel::lines::runs`, proved).
    pub fn regions(&self, out: &mut Vec<Region>) {
        let body: Vec<Origin> = self
            .lines
            .iter()
            .map(|line| match line.origin {
                LineOrigin::Context => Origin::Context,
                LineOrigin::Addition => Origin::Addition,
                LineOrigin::Deletion => Origin::Deletion,
            })
            .collect();
        lines::runs(
            first_line(self.old_start, self.old_count),
            self.new_first(),
            &body,
            out,
        );
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

#[cfg(test)]
mod tests;
