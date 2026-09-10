//! Worker-built Git display data. UI clones share immutable text and indexes;
//! replay records source data and reconstructs the same checked projection.
use std::ops::Deref;
use std::sync::Arc;

use ropey::Rope;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strop_git::{DiffLine, Hunk, LineOrigin};

use crate::editor::DiffRow;

#[derive(Debug, Clone)]
pub struct PreparedDiff(Arc<DiffData>);

#[derive(Debug)]
struct DiffData {
    label: String,
    hunks: Vec<Arc<Hunk>>,
    text: Rope,
    starts: Vec<usize>,
    emphasis: Vec<Option<(usize, usize)>>,
    added: usize,
    deleted: usize,
    gutter_width: usize,
}

impl PreparedDiff {
    pub fn new(label: String, hunks: Vec<Hunk>) -> Self {
        let (added, deleted) = super::hunk_stats(&hunks);
        let text = Rope::from_str(&super::diff_surface_text(&label, &hunks));
        let mut starts = Vec::with_capacity(hunks.len());
        let mut emphasis = vec![None]; // stats row
        let mut max_lineno = 0;
        for hunk in &hunks {
            starts.push(emphasis.len());
            emphasis.push(None); // hunk header
            let base = emphasis.len();
            emphasis.resize(base + hunk.lines.len(), None);
            prepare_emphasis(&hunk.lines, &mut emphasis[base..]);
            for line in &hunk.lines {
                max_lineno = max_lineno
                    .max(line.old_lineno.unwrap_or(0))
                    .max(line.new_lineno.unwrap_or(0));
            }
        }
        let digits = (max_lineno.checked_ilog10().unwrap_or(0) as usize + 1).max(3);
        Self(Arc::new(DiffData {
            label,
            hunks: hunks.into_iter().map(Arc::new).collect(),
            text,
            starts,
            emphasis,
            added,
            deleted,
            gutter_width: 2 * digits + 3,
        }))
    }

    pub fn label(&self) -> &str {
        &self.0.label
    }
    pub(crate) fn text(&self) -> Rope {
        self.0.text.clone()
    }
    pub fn added(&self) -> usize {
        self.0.added
    }
    pub fn deleted(&self) -> usize {
        self.0.deleted
    }
    pub fn gutter_width(&self) -> usize {
        self.0.gutter_width
    }
    pub fn emphasis(&self, row: usize) -> Option<(usize, usize)> {
        self.0.emphasis.get(row).copied().flatten()
    }
    pub(crate) fn row(&self, row: usize) -> Option<DiffRow<'_>> {
        if row == 0 {
            return Some(DiffRow::Stats);
        }
        let index = self
            .0
            .starts
            .partition_point(|start| *start <= row)
            .checked_sub(1)?;
        let hunk = &self.0.hunks[index];
        let relative = row - self.0.starts[index];
        if relative == 0 {
            Some(DiffRow::HunkHeader(hunk))
        } else {
            hunk.lines.get(relative - 1).map(DiffRow::Line)
        }
    }
}

impl Deref for PreparedDiff {
    type Target = [Arc<Hunk>];
    fn deref(&self) -> &Self::Target {
        &self.0.hunks
    }
}
impl<'a> IntoIterator for &'a PreparedDiff {
    type Item = &'a Arc<Hunk>;
    type IntoIter = std::slice::Iter<'a, Arc<Hunk>>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl Serialize for PreparedDiff {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (&self.0.label, &self.0.hunks).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for PreparedDiff {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (label, hunks) = Deserialize::deserialize(deserializer)?;
        Ok(Self::new(label, hunks))
    }
}

// Pair whole deletion/addition runs once, rather than rescanning a run for
// every visible row. Each range belongs to its own side's UTF-8 byte domain.
fn prepare_emphasis(lines: &[DiffLine], output: &mut [Option<(usize, usize)>]) {
    let mut at = 0;
    while at < lines.len() {
        if lines[at].origin != LineOrigin::Deletion {
            at += 1;
            continue;
        }
        let deleted = at;
        while at < lines.len() && lines[at].origin == LineOrigin::Deletion {
            at += 1;
        }
        let added = at;
        while at < lines.len() && lines[at].origin == LineOrigin::Addition {
            at += 1;
        }
        for offset in 0..(added - deleted).min(at - added) {
            let left = lines[deleted + offset].text_str();
            let right = lines[added + offset].text_str();
            output[deleted + offset] = Some(changed_range(&left, &right));
            output[added + offset] = Some(changed_range(&right, &left));
        }
    }
}

fn changed_range(a: &str, b: &str) -> (usize, usize) {
    let prefix: usize = a
        .chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = a[prefix..]
        .chars()
        .rev()
        .zip(b[prefix..].chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(c, _)| c.len_utf8())
        .sum();
    (prefix, a.len() - suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_spans_use_each_sides_unicode_offsets() {
        let lines = vec![
            DiffLine {
                origin: LineOrigin::Deletion,
                old_lineno: Some(4),
                new_lineno: None,
                text: "a界z".into(),
                has_newline: true,
            },
            DiffLine {
                origin: LineOrigin::Addition,
                old_lineno: None,
                new_lineno: Some(4),
                text: "aééz".into(),
                has_newline: true,
            },
        ];
        let mut spans = vec![None; 2];
        prepare_emphasis(&lines, &mut spans);
        assert_eq!(
            &lines[0].text_str()[spans[0].unwrap().0..spans[0].unwrap().1],
            "界"
        );
        assert_eq!(
            &lines[1].text_str()[spans[1].unwrap().0..spans[1].unwrap().1],
            "éé"
        );
    }
}
