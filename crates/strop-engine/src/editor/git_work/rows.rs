//! The status buffer's text and its typed row model, built together so
//! they can not disagree: line `n` of the buffer is `rows[n]`. Pure —
//! no editor, no git handle.

use strop_git::{
    DiffLine, FileDelta, Hunk, LineOrigin, LineStat, Section, StatusEntry, WorkingTree,
};

/// What one buffer line is. Indices point into the view's tree and
/// deltas; rendering and the key verbs both read rows, never the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusRow {
    Head,
    Tracking,
    Blank,
    /// "Nothing to commit" on a clean tree.
    Clean,
    Section(Section),
    /// `tree.entries[n]`.
    File(usize),
    /// An expanded file with no line diff to show (binary, mode-only).
    Note(usize),
    HunkHeader {
        entry: usize,
        hunk: usize,
    },
    Line {
        entry: usize,
        hunk: usize,
        line: usize,
    },
}

impl StatusRow {
    /// The entry this row belongs to, for file and diff rows.
    pub fn entry(self) -> Option<usize> {
        match self {
            StatusRow::File(entry)
            | StatusRow::Note(entry)
            | StatusRow::HunkHeader { entry, .. }
            | StatusRow::Line { entry, .. } => Some(entry),
            _ => None,
        }
    }

    /// The hunk this row is part of, as `(entry, hunk)`.
    pub fn hunk(self) -> Option<(usize, usize)> {
        match self {
            StatusRow::HunkHeader { entry, hunk } | StatusRow::Line { entry, hunk, .. } => {
                Some((entry, hunk))
            }
            _ => None,
        }
    }
}

/// A status surface's payload: the last snapshot, which files the user
/// expanded, and the row model of the text currently in the buffer.
#[derive(Debug, Clone, Default)]
pub struct StatusView {
    pub tree: WorkingTree,
    /// Diffs of the expanded entries, as of `tree`.
    pub deltas: Vec<(StatusEntry, FileDelta)>,
    /// Lines added and removed per entry, `stats[n]` for
    /// `tree.entries[n]` (0100 §7); installed with the tree.
    pub stats: Vec<Option<LineStat>>,
    /// What `=` opened; carried into every refresh.
    pub expanded: Vec<StatusEntry>,
    pub rows: Vec<StatusRow>,
    /// False until the first snapshot lands (the buffer says "loading").
    pub loaded: bool,
}

impl StatusView {
    pub fn row(&self, line: usize) -> Option<StatusRow> {
        self.rows.get(line).copied()
    }

    pub fn entry(&self, index: usize) -> Option<&StatusEntry> {
        self.tree.entries.get(index)
    }

    /// `+N -M` of entry `index`: none for a conflict, an untracked
    /// directory, or a file that moved under the read.
    pub fn stat(&self, index: usize) -> Option<LineStat> {
        self.stats.get(index).copied().flatten()
    }

    pub fn delta(&self, entry: &StatusEntry) -> Option<&FileDelta> {
        self.deltas
            .iter()
            .find(|(owner, _)| owner == entry)
            .map(|(_, delta)| delta)
    }

    pub fn hunk(&self, entry: usize, hunk: usize) -> Option<&Hunk> {
        match self.delta(self.entry(entry)?)? {
            FileDelta::Hunks(hunks) => hunks.get(hunk),
            FileDelta::Binary => None,
        }
    }

    pub fn diff_line(&self, entry: usize, hunk: usize, line: usize) -> Option<&DiffLine> {
        self.hunk(entry, hunk)?.lines.get(line)
    }

    /// How many entries a section holds (its heading's count).
    pub fn count(&self, section: Section) -> usize {
        self.tree.section(section).count()
    }

    /// The first line showing `row`.
    pub fn line_of(&self, row: StatusRow) -> Option<usize> {
        self.rows.iter().position(|candidate| *candidate == row)
    }

    /// Install a snapshot: returns the buffer text for it. Expansions
    /// whose entry left the tree are dropped.
    pub fn install(&mut self, data: super::super::git_memory::StatusData) -> String {
        let super::super::git_memory::StatusData {
            tree,
            deltas,
            stats,
        } = data;
        debug_assert_eq!(stats.len(), tree.entries.len(), "one stat per entry");
        self.expanded.retain(|entry| tree.entries.contains(entry));
        self.tree = tree;
        self.deltas = deltas;
        self.stats = stats;
        self.loaded = true;
        let (text, rows) = build(&self.tree, &self.deltas);
        self.rows = rows;
        text
    }
}

fn origin_prefix(origin: LineOrigin) -> char {
    match origin {
        LineOrigin::Addition => '+',
        LineOrigin::Deletion => '-',
        LineOrigin::Context => ' ',
    }
}

/// The buffer text and one row per line of it.
pub fn build(tree: &WorkingTree, deltas: &[(StatusEntry, FileDelta)]) -> (String, Vec<StatusRow>) {
    let mut text = String::new();
    let mut rows = Vec::new();
    let mut push = |line: &str, row: StatusRow| {
        text.push_str(line);
        text.push('\n');
        rows.push(row);
    };
    push(
        &format!("Head: {}", tree.head.as_deref().unwrap_or("(no branch)")),
        StatusRow::Head,
    );
    if let Some(tracking) = &tree.tracking {
        let mut line = format!("Upstream: {}", tracking.upstream);
        if tracking.ahead > 0 {
            line.push_str(&format!(", ahead {}", tracking.ahead));
        }
        if tracking.behind > 0 {
            line.push_str(&format!(", behind {}", tracking.behind));
        }
        push(&line, StatusRow::Tracking);
    }
    if tree.entries.is_empty() {
        push("", StatusRow::Blank);
        push("Nothing to commit, working tree clean", StatusRow::Clean);
    }
    for section in Section::ALL {
        let members: Vec<usize> = (0..tree.entries.len())
            .filter(|index| tree.entries[*index].section == section)
            .collect();
        if members.is_empty() {
            continue;
        }
        push("", StatusRow::Blank);
        push(
            &format!("{} ({})", section.title(), members.len()),
            StatusRow::Section(section),
        );
        for index in members {
            let entry = &tree.entries[index];
            push(
                &format!("{} {}", entry.change.letter(), entry.rel.to_string_lossy()),
                StatusRow::File(index),
            );
            let Some((_, delta)) = deltas.iter().find(|(owner, _)| owner == entry) else {
                continue;
            };
            match delta {
                FileDelta::Binary => push("  binary file", StatusRow::Note(index)),
                FileDelta::Hunks(hunks) if hunks.is_empty() => {
                    push("  no line changes", StatusRow::Note(index))
                }
                FileDelta::Hunks(hunks) => {
                    for (at, hunk) in hunks.iter().enumerate() {
                        push(
                            &hunk.header(),
                            StatusRow::HunkHeader {
                                entry: index,
                                hunk: at,
                            },
                        );
                        for (line_at, line) in hunk.lines.iter().enumerate() {
                            push(
                                &format!("{}{}", origin_prefix(line.origin), line.text_str()),
                                StatusRow::Line {
                                    entry: index,
                                    hunk: at,
                                    line: line_at,
                                },
                            );
                        }
                    }
                }
            }
        }
    }
    (text, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_git::{Change, Tracking};

    fn entry(section: Section, change: Change, rel: &str) -> StatusEntry {
        StatusEntry {
            section,
            change,
            rel: rel.into(),
        }
    }

    #[test]
    fn clean_tree_says_so() {
        let tree = WorkingTree {
            head: Some("main".into()),
            ..Default::default()
        };
        let (text, rows) = build(&tree, &[]);
        assert_eq!(
            text,
            "Head: main\n\nNothing to commit, working tree clean\n"
        );
        assert_eq!(
            rows,
            vec![StatusRow::Head, StatusRow::Blank, StatusRow::Clean]
        );
    }

    #[test]
    fn sections_come_in_order_with_counts_and_inline_hunks() {
        let unstaged = entry(Section::Unstaged, Change::Modified, "src/lib.rs");
        let tree = WorkingTree {
            head: Some("main".into()),
            tracking: Some(Tracking {
                upstream: "origin/main".into(),
                ahead: 2,
                behind: 0,
            }),
            entries: vec![
                entry(Section::Untracked, Change::Untracked, "notes.txt"),
                unstaged.clone(),
                entry(Section::Staged, Change::Added, "new.rs"),
            ],
        };
        let line = |origin, text: &str| DiffLine {
            origin,
            old_lineno: None,
            new_lineno: None,
            text: text.as_bytes().to_vec(),
            has_newline: true,
        };
        let hunk = Hunk::build(
            1,
            1,
            1,
            2,
            vec![
                line(LineOrigin::Context, "fn a() {}"),
                line(LineOrigin::Addition, "fn b() {}"),
            ],
        );
        let (text, rows) = build(&tree, &[(unstaged, FileDelta::Hunks(vec![hunk]))]);
        assert_eq!(
            text,
            "Head: main\n\
             Upstream: origin/main, ahead 2\n\
             \n\
             Staged (1)\n\
             A new.rs\n\
             \n\
             Unstaged (1)\n\
             M src/lib.rs\n\
             @@ -1,1 +1,2 @@\n \
             fn a() {}\n\
             +fn b() {}\n\
             \n\
             Untracked (1)\n\
             ? notes.txt\n"
        );
        assert_eq!(rows.len(), text.lines().count());
        assert_eq!(rows[4], StatusRow::File(2));
        assert_eq!(rows[8], StatusRow::HunkHeader { entry: 1, hunk: 0 });
        assert_eq!(
            rows[10],
            StatusRow::Line {
                entry: 1,
                hunk: 0,
                line: 1
            }
        );
    }
}
