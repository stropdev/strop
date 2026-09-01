//! strop-picker: the one picker component (0001 pillar 1, 0003 §2).
//! Model + scoring + streaming sources. Rendering lives in the binary;
//! this crate never draws.

mod score;
mod source;

pub use score::fuzzy_score;
pub use source::{spawn_files, GrepWorker, PickerMsg};

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Payload {
    /// A file path relative to the working directory.
    File(PathBuf),
    /// An open buffer (index into the editor's buffer list).
    Buffer(usize),
    /// A grep hit: path, 1-based line, 1-based col, the matched line.
    Grep {
        path: PathBuf,
        line: usize,
        col: usize,
        line_text: String,
    },
}

#[derive(Debug, Clone)]
pub struct Item {
    /// What the results list renders.
    pub text: String,
    pub payload: Payload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Files,
    Buffers,
    Grep,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::Files => " files ",
            Kind::Buffers => " buffers ",
            Kind::Grep => " grep ",
        }
    }
}

/// A scored, filtered row: index into `items` + matched char columns.
#[derive(Debug, Clone)]
pub struct Row {
    pub item: usize,
    /// Denormalized display text — the renderer shouldn't chase indices.
    pub text: String,
    pub score: i32,
    pub match_cols: Vec<u32>,
}

/// Picker state: input, accumulated items (streaming sources append),
/// filtered rows, selection. Filtering is synchronous over the
/// accumulated items — cheap at repo scale with subsequence scoring.
pub struct Picker {
    pub kind: Kind,
    pub input: String,
    pub items: Vec<Item>,
    pub rows: Vec<Row>,
    pub selected: usize,
    /// Streaming sources: true while the worker may still send.
    pub streaming: bool,
}

impl Picker {
    pub fn new(kind: Kind, items: Vec<Item>, streaming: bool) -> Self {
        let mut p = Self {
            kind,
            input: String::new(),
            items,
            rows: Vec::new(),
            selected: 0,
            streaming,
        };
        p.refilter();
        p
    }

    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn pop_char(&mut self) {
        self.input.pop();
    }

    /// Recompute rows from items + input. Grep rows arrive pre-filtered
    /// from rg; everything else fuzzy-filters here.
    pub fn refilter(&mut self) {
        self.rows = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                if self.input.is_empty() {
                    return Some(Row {
                        item: i,
                        text: item.text.clone(),
                        score: 0,
                        match_cols: vec![],
                    });
                }
                fuzzy_score(&self.input, &item.text).map(|(score, cols)| Row {
                    item: i,
                    text: item.text.clone(),
                    score,
                    match_cols: cols,
                })
            })
            .collect();
        self.rows.sort_by_key(|r| std::cmp::Reverse(r.score));
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    pub fn append(&mut self, items: Vec<Item>) {
        self.items.extend(items);
        self.refilter();
    }

    pub fn move_by(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(n)) as usize;
    }

    pub fn current(&self) -> Option<&Item> {
        self.rows.get(self.selected).map(|r| &self.items[r.item])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_and_rank() {
        let items = vec![
            Item {
                text: "src/main.rs".into(),
                payload: Payload::File("src/main.rs".into()),
            },
            Item {
                text: "src/render.rs".into(),
                payload: Payload::File("src/render.rs".into()),
            },
            Item {
                text: "tests/e2e.py".into(),
                payload: Payload::File("tests/e2e.py".into()),
            },
        ];
        let mut p = Picker::new(Kind::Files, items, false);
        for c in "render".chars() {
            p.push_char(c);
        }
        p.refilter();
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.current().unwrap().text, "src/render.rs");
    }

    #[test]
    fn selection_wraps() {
        let items: Vec<Item> = (0..3)
            .map(|i| Item {
                text: format!("f{i}"),
                payload: Payload::Buffer(i),
            })
            .collect();
        let mut p = Picker::new(Kind::Buffers, items, false);
        p.move_by(-1);
        assert_eq!(p.selected, 2);
        p.move_by(1);
        assert_eq!(p.selected, 0);
    }
}
