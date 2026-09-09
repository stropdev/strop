//! strop-picker: the one picker component (0001 pillar 1, 0003 §2).
//! Model + scoring + streaming sources. Rendering lives in the binary;
//! this crate never draws.

mod catalog;
pub use catalog::Catalog;
pub mod rank;
pub use rank::{FilterRequest, Ranking, RankingEvent, RankingWorker, Row};
mod line_edit;
mod score;
mod source;

pub use line_edit::LineEdit;

pub use score::fuzzy_score;
pub use source::{spawn_files, GrepWorker, PickerMsg};

use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Payload {
    /// A file path relative to the working directory.
    #[serde(with = "strop_core::path_serde")]
    File(PathBuf),
    /// An open document (stable generational id, 0014 wave 2).
    Buffer(strop_core::id::DocumentId),
    /// A grep hit: path, 1-based line, 1-based col, matched-span length
    /// in bytes, the matched line.
    Grep {
        #[serde(with = "strop_core::path_serde")]
        path: PathBuf,
        line: usize,
        col: usize,
        match_len: usize,
        line_text: String,
    },
    /// A location on a validated remote endpoint. The native path can never
    /// be previewed or opened as an analogous local file.
    Remote {
        endpoint: strop_remote::RemoteEndpoint,
        #[serde(with = "strop_core::path_serde")]
        path: PathBuf,
        line: usize,
        col: usize,
    },
    /// Explicitly chosen SSH directory, never an analogous local path.
    RemoteDirectory(strop_remote::RemoteFile),
    /// Switch the same modal picker to its new-address field.
    RemoteConnect,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// Two-field global search & replace (0007).
    Replace,
    /// Editor-computed items (LSP diagnostics, 0009 §3).
    Diagnostics,
    /// LSP location lists (references/implementation/…): same
    /// payload shape as grep rows, title set per request.
    Locations,
    RemoteHosts,
    RemoteAddress,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::Files => " files ",
            Kind::Buffers => " buffers ",
            Kind::Grep => " grep ",
            Kind::Replace => " replace ",
            Kind::Locations => " locations ",
            Kind::Diagnostics => " diagnostics ",
            Kind::RemoteHosts => " remote destinations ",
            Kind::RemoteAddress => " connect to remote ",
        }
    }
}

/// Which input field has focus in Kind::Replace (0007 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Search,
    Replace,
}

/// Picker input and immutable catalog ownership. Scoring is worker work; the UI
/// installs a checked ranking and renders only its visible rows.
pub struct Picker {
    pub kind: Kind,
    pub input: LineEdit,
    /// Replace mode (0007 §2): the second field, its focus, and the
    /// per-item exclusion set (item indices).
    pub replace_input: LineEdit,
    pub field: Field,
    pub excluded: std::collections::HashSet<usize>,
    /// Whole-file exclusion (replace mode, vscode's file toggle):
    /// ctrl-d on a row excludes every match in that file.
    pub excluded_files: std::collections::HashSet<PathBuf>,
    pub items: Catalog,
    pub rows: Vec<Row>,
    match_columns: Vec<u32>,
    exclusion_count: usize,
    file_counts: std::collections::HashMap<PathBuf, FileCounts>,
    pub selected: usize,
    /// Streaming sources: true while the worker may still send.
    pub streaming: bool,
    /// A source error (rg's stderr, a dead worker): sticky in the card —
    /// the transient modeline clears on the next keystroke, this doesn't.
    pub error: Option<String>,
}

#[derive(Default)]
struct FileCounts {
    items: usize,
    excluded_rows: usize,
}

impl Picker {
    pub fn new(kind: Kind, items: Vec<Item>, streaming: bool) -> Self {
        let mut p = Self {
            kind,
            input: LineEdit::default(),
            replace_input: LineEdit::default(),
            field: Field::Search,
            excluded: std::collections::HashSet::new(),
            excluded_files: std::collections::HashSet::new(),
            items: Catalog::default(),
            rows: Vec::new(),
            match_columns: Vec::new(),
            exclusion_count: 0,
            file_counts: std::collections::HashMap::new(),
            selected: 0,
            streaming,
            error: None,
        };
        p.append(items);
        p
    }

    pub fn push_char(&mut self, c: char) {
        self.input.insert_char(c);
    }

    pub fn pop_char(&mut self) {
        self.input.backspace();
    }

    /// Replace-mode second field input.
    pub fn push_replace_char(&mut self, c: char) {
        self.replace_input.insert_char(c);
    }

    pub fn pop_replace_char(&mut self) {
        self.replace_input.backspace();
    }

    /// The focused field's edit state.
    fn active(&mut self) -> &mut LineEdit {
        match self.field {
            Field::Search => &mut self.input,
            Field::Replace => &mut self.replace_input,
        }
    }

    /// Esc in a picker field enters normal mode (rootle's input boxes);
    /// Esc again closes — the editor calls picker_normal() to decide.
    pub fn enter_normal(&mut self) {
        self.active().normal = true;
    }

    pub fn input_normal(&self) -> bool {
        match self.field {
            Field::Search => &self.input,
            Field::Replace => &self.replace_input,
        }
        .normal
    }

    /// One normal-mode key on the focused field; true when the text
    /// changed (x/X) — the glue respawns rg on that.
    pub fn normal_key(&mut self, c: char) -> bool {
        let changed = matches!(c, 'x' | 'X');
        self.active().normal_key(c) && changed
    }

    /// Field switch parks the caret at the field's end.
    pub fn sync_cursor(&mut self) {
        let text_len = self.active().text.len();
        self.active().cursor = text_len;
    }

    /// Arrow-key caret moves work in both modes, on the focused field.
    pub fn caret_left(&mut self) {
        self.active().move_left();
    }

    pub fn caret_right(&mut self) {
        self.active().move_right();
    }

    /// Tab swaps the focused field in Kind::Replace.
    pub fn toggle_field(&mut self) {
        self.field = match self.field {
            Field::Search => Field::Replace,
            Field::Replace => Field::Search,
        };
        self.sync_cursor();
    }

    /// Exclude/include the selected row from the apply set (0007 §2).
    pub fn toggle_excluded(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let item = row.item;
        let removed = self.excluded.remove(&item);
        if !removed {
            self.excluded.insert(item);
        }
        let mut covered = false;
        if let Payload::Grep { path, .. } = &self.items[item].payload {
            covered = self.excluded_files.contains(path);
            match self.file_counts.get_mut(path) {
                Some(counts) => {
                    if removed {
                        counts.excluded_rows -= 1;
                    } else {
                        counts.excluded_rows += 1;
                    }
                }
                None => unreachable!("grep entries are indexed during append"),
            }
        }
        if !covered {
            if removed {
                self.exclusion_count -= 1;
            } else {
                self.exclusion_count += 1;
            }
        }
    }

    /// Exclude/include every match in the selected row's file (replace
    /// mode, vscode's per-file toggle).
    pub fn toggle_file_excluded(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let Some(path) = (match &self.items[row.item].payload {
            Payload::Grep { path, .. } => Some(path.clone()),
            _ => None,
        }) else {
            return;
        };
        let counts = &self.file_counts[&path];
        let changed = counts.items - counts.excluded_rows;
        if self.excluded_files.remove(&path) {
            self.exclusion_count -= changed;
        } else {
            self.excluded_files.insert(path);
            self.exclusion_count += changed;
        }
    }

    /// True when the item is out of the apply set (row or file).
    pub fn is_excluded(&self, item: usize) -> bool {
        if self.excluded.contains(&item) {
            return true;
        }
        match &self.items[item].payload {
            Payload::Grep { path, .. } => self.excluded_files.contains(path),
            _ => false,
        }
    }

    /// Items in the apply set (replace mode): everything not excluded.
    pub fn accepted(&self) -> impl Iterator<Item = &Item> {
        self.items
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.is_excluded(*i))
            .map(|(_, it)| it)
    }

    pub fn filter_request(&self) -> FilterRequest {
        FilterRequest {
            catalog: self.items.clone(),
            query: self.input.text.clone(),
            upstream_filtered: matches!(self.kind, Kind::Grep | Kind::Replace),
        }
    }

    pub fn install_ranking(&mut self, ranking: Ranking) -> bool {
        if ranking.item_count() != self.items.len() {
            return false;
        }
        self.rows = ranking.rows;
        self.match_columns = ranking.columns;
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        true
    }

    pub fn match_columns(&self, row: &Row) -> &[u32] {
        &self.match_columns[row.matches.clone()]
    }

    pub fn clear_results(&mut self) {
        self.rows.clear();
        self.match_columns.clear();
        self.selected = 0;
    }

    pub fn clear_items(&mut self) {
        self.items.clear();
        self.clear_results();
        self.excluded.clear();
        self.excluded_files.clear();
        self.exclusion_count = 0;
        self.file_counts.clear();
    }

    pub fn excluded_count(&self) -> usize {
        self.exclusion_count
    }

    pub fn append(&mut self, items: Vec<Item>) {
        for item in &items {
            if let Payload::Grep { path, .. } = &item.payload {
                self.file_counts.entry(path.clone()).or_default().items += 1;
                self.exclusion_count += usize::from(self.excluded_files.contains(path));
            }
        }
        self.items.append(items);
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

/// The clamped, char-boundary-safe byte span of a match on its line.
/// Row rendering and apply share this (0007 §4: preview cannot lie).
pub fn replace_span(line_text: &str, col: usize, match_len: usize) -> (usize, usize) {
    let mut start = col.saturating_sub(1).min(line_text.len());
    while start > 0 && !line_text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + match_len).min(line_text.len());
    while end > start && !line_text.is_char_boundary(end) {
        end -= 1;
    }

    (start, end)
}

/// The one replacement computation: `line_text` with the matched span
/// swapped for `replacement`.
pub fn replaced_line(line_text: &str, col: usize, match_len: usize, replacement: &str) -> String {
    let (start, end) = replace_span(line_text, col, match_len);
    format!(
        "{}{}{}",
        &line_text[..start],
        replacement,
        &line_text[end..]
    )
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
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap());
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.current().unwrap().text, "src/render.rs");
    }

    #[test]
    fn selection_wraps() {
        let mut arena: strop_core::id::Arena<strop_core::id::DocumentKind, ()> =
            strop_core::id::Arena::default();
        let items: Vec<Item> = (0..3)
            .map(|i| Item {
                text: format!("f{i}"),
                payload: Payload::Buffer(arena.insert(())),
            })
            .collect();
        let mut p = Picker::new(Kind::Buffers, items, false);
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap());
        p.move_by(-1);
        assert_eq!(p.selected, 2);
        p.move_by(1);
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn ctrl_d_excludes_a_whole_file() {
        let hit = |path: &str| Item {
            text: path.into(),
            payload: Payload::Grep {
                path: PathBuf::from(path),
                line: 1,
                col: 1,
                match_len: 1,
                line_text: "x".into(),
            },
        };
        let mut p = Picker::new(
            Kind::Replace,
            vec![hit("a.rs"), hit("a.rs"), hit("b.rs")],
            false,
        );
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap());
        p.toggle_file_excluded(); // row 0 -> a.rs
        assert_eq!(p.accepted().count(), 1);
        assert_eq!(p.accepted().next().unwrap().text, "b.rs");
        p.toggle_file_excluded(); // toggle back
        assert_eq!(p.accepted().count(), 3);
    }
    #[test]
    fn regex_query_never_hides_rg_rows() {
        // 0020 §9: "foo|bar" matched three rows upstream — all of them
        // stay visible even though none contains the literal "|"
        let mut p = Picker::new(
            Kind::Grep,
            vec![
                Item {
                    text: "a.rs:1 foo".into(),
                    payload: Payload::File("a.rs".into()),
                },
                Item {
                    text: "b.rs:2 bar".into(),
                    payload: Payload::File("b.rs".into()),
                },
                Item {
                    text: "c.rs:3 baz".into(),
                    payload: Payload::File("c.rs".into()),
                },
            ],
            false,
        );
        for c in "foo|bar".chars() {
            p.push_char(c);
        }
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap());
        assert_eq!(p.rows.len(), 3);
        // and the apply set is exactly those rows
        assert_eq!(p.accepted().count(), 3);
    }
}
