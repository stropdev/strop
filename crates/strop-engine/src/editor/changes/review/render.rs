//! Review text and row roles from the same validated replacements as apply.
//! Only affected lines plus context are materialized, never an entire source.
use super::{ReviewRow, CONTEXT};
use strop_core::id::BufferRevision;
use strop_core::{Buffer, EditError, Replacement};

#[derive(Default)]
pub(super) struct ReviewBuffer {
    pub text: String,
    pub rows: Vec<ReviewRow>,
}
impl ReviewBuffer {
    pub fn line(&mut self, text: &str, role: ReviewRow) {
        for line in text.split('\n') {
            self.text.push_str(line);
            self.text.push('\n');
            self.rows.push(role);
        }
    }
    pub fn append(&mut self, other: Self) {
        self.text.push_str(&other.text);
        self.rows.extend(other.rows);
    }
}

struct EditLines {
    first: usize,
    last: usize,
    edits: std::ops::Range<usize>,
}
struct ChangedLines {
    first: usize,
    last: usize,
    added: Vec<String>,
}

pub(super) fn file_diff(
    label: &str,
    buffer: &Buffer,
    revision: BufferRevision,
    edits: &[Replacement],
) -> Result<ReviewBuffer, EditError> {
    let prepared = buffer.prepare_replacements(revision, edits.to_vec())?;
    let edits = prepared.edits();
    let mut view = ReviewBuffer::default();
    let label: String = label
        .chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect();
    view.line(&format!("--- a/{label}"), ReviewRow::File);
    view.line(&format!("+++ b/{label}"), ReviewRow::File);
    let last_line = buffer.last_content_line();
    let mut groups: Vec<EditLines> = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        let first = buffer.line_of(edit.range.start).min(last_line);
        let end = if edit.range.is_empty() {
            edit.range.end.get()
        } else {
            edit.range.end.get() - 1
        };
        let last = buffer.line_of(end).min(last_line);
        if let Some(group) = groups.last_mut().filter(|group| first <= group.last) {
            group.last = group.last.max(last);
            group.edits.end = index + 1;
        } else {
            groups.push(EditLines {
                first,
                last,
                edits: index..index + 1,
            });
        }
    }
    let mut changed = Vec::with_capacity(groups.len());
    for group in groups {
        let start = buffer.line_start(group.first);
        let end = if group.last + 1 < buffer.len_lines() {
            buffer.line_start(group.last + 1)
        } else {
            buffer.len_bytes()
        };
        let mut added = buffer.text().byte_slice(start..end).to_string();
        for edit in edits[group.edits].iter().rev() {
            added.replace_range(
                edit.range.start.get() - start..edit.range.end.get() - start,
                &edit.text,
            );
        }
        changed.push(ChangedLines {
            first: group.first,
            last: group.last,
            added: added
                .split_terminator('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
                .collect(),
        });
    }
    let mut windows: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (index, change) in changed.iter().enumerate() {
        let first = change.first.saturating_sub(CONTEXT);
        let last = (change.last + CONTEXT).min(last_line);
        if let Some(window) = windows.last_mut().filter(|window| first <= window.1 + 1) {
            window.1 = window.1.max(last);
            window.3 = index + 1;
        } else {
            windows.push((first, last, index, index + 1));
        }
    }
    let mut delta = 0isize;
    for (first, last, begin, end) in windows {
        let changes = &changed[begin..end];
        let removed: usize = changes
            .iter()
            .map(|change| change.last + 1 - change.first)
            .sum();
        let added: usize = changes.iter().map(|change| change.added.len()).sum();
        let count = last + 1 - first;
        debug_assert!(
            removed <= count,
            "validated edits have disjoint line groups"
        );
        view.line(
            &format!(
                "@@ -{},{} +{},{} @@",
                first + 1,
                count,
                (first as isize + delta + 1).max(1),
                count - removed + added
            ),
            ReviewRow::Hunk,
        );
        let mut cursor = first;
        for change in changes {
            for line in cursor..change.first {
                view.line(&format!(" {}", buffer.line_text(line)), ReviewRow::Context);
            }
            for line in change.first..=change.last {
                view.line(&format!("-{}", buffer.line_text(line)), ReviewRow::Removed);
            }
            for line in &change.added {
                view.line(&format!("+{line}"), ReviewRow::Added);
            }
            cursor = change.last + 1;
        }
        for line in cursor..=last {
            view.line(&format!(" {}", buffer.line_text(line)), ReviewRow::Context);
        }
        delta += added as isize - removed as isize;
    }
    Ok(view)
}
