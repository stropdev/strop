//! strop-picker: the one picker component (0001 pillar 1, 0003 §2).
//! Model + scoring + streaming sources. Rendering lives in the binary;
//! this crate never draws.

mod catalog;
pub use catalog::Catalog;
pub mod query;
pub mod rank;
pub use rank::{FilterRequest, Ranking, RankingEvent, RankingWorker, Row};
mod line_edit;
mod score;
mod source;
mod workset;
pub use workset::WorksetSnapshot;

pub use line_edit::LineEdit;

pub use score::fuzzy_score;
pub use source::{
    display_path, PickerMsg, SelectionPolicy, SourceSink, SourceSnapshot, SourceWorker,
};

use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Payload {
    /// A file path relative to the working directory.
    #[serde(with = "strop_core::path_serde")]
    File(PathBuf),
    /// An open document (stable generational id, 0014 wave 2).
    Buffer(strop_core::id::DocumentId),
    /// A content hit with resolved namespace identity and 1-based source coordinates.
    Grep {
        location: strop_workspace::ResourceLocation,
        line: usize,
        col: usize,
        match_len: usize,
        line_text: std::sync::Arc<str>,
    },
    /// A location on a validated remote endpoint. The native path can never
    /// be previewed or opened as an analogous local file.
    Remote {
        endpoint: strop_workspace::RemoteEndpoint,
        #[serde(with = "strop_core::path_serde")]
        path: PathBuf,
        line: usize,
        col: usize,
    },
    /// Explicitly chosen SSH directory, never an analogous local path.
    RemoteDirectory(strop_workspace::RemoteFile),
    /// Switch the same modal picker to its new-address field.
    RemoteConnect,
    /// A language-server code action: the editor's pending action list
    /// index. The action payload itself never crosses the picker.
    CodeAction(usize),
    /// Index into the editor's captured filesystem action selector.
    FilesystemAction(usize),
    /// A running container's canonical inspect id (0037 DC1a).
    Container(String),
    /// A search-visibility setting toggle (0051 R03).
    SearchOption(SearchSetting),
    /// A jumplist entry: document + byte offset (0047 §2).
    Jump {
        document: strop_core::id::DocumentId,
        offset: usize,
    },
    /// A `:tab-size` selector row (0051 R08).
    IndentChoice(IndentChoice),
}

/// A `:tab-size` selector row's action (0051 R08): the width/style
/// override the row applies to the current buffer, or Auto to clear it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IndentChoice {
    /// A common width in spaces.
    Width(usize),
    /// Clear the width override (detect/configure).
    AutoWidth,
    /// Force spaces.
    Spaces,
    /// Force tabs.
    Tabs,
    /// Clear the style override (detect/configure).
    AutoStyle,
    /// The width typed into the filter field, validated on accept.
    CustomWidth,
}

/// Which search-visibility setting a SearchOption row toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SearchSetting {
    Hidden,
    RespectIgnore,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Item {
    /// What the results list renders.
    pub text: String,
    pub payload: Payload,
    /// A short visual chip rendered before the text (symbol kind, e.g.
    /// `fn`/`struct`) — plain text, colored by the renderer; None for
    /// ordinary rows.
    pub badge: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The jumplist as a menu (0047 §2).
    Jumps,
    /// Hidden/ignore visibility controls for search surfaces (R03).
    SearchOptions,
    /// A language server's document symbols (0047 §1).
    Symbols,
    Files,
    Buffers,
    /// One content-search workspace with an optional replacement facet.
    Search,
    /// Editor-computed items (LSP diagnostics, 0009 §3).
    Diagnostics,
    /// LSP location lists (references/implementation/…): same
    /// payload shape as grep rows, title set per request.
    Locations,
    RemoteHosts,
    RemoteAddress,
    /// Language-server code actions (0043): titles listed, acceptance
    /// applies the chosen action's edits through a change plan.
    CodeActions,
    FilesystemActions,
    /// Running containers on the local engine (0037 DC1a).
    Containers,
    /// The `:tab-size` indent selector (0051 R08).
    TabSize,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::Files => " files ",
            Kind::Buffers => " buffers ",
            Kind::Search => " search ",
            Kind::Locations => " locations ",
            Kind::Containers => " containers ",
            Kind::Diagnostics => " diagnostics ",
            Kind::RemoteHosts => " remote destinations ",
            Kind::Jumps => " jumps ",
            Kind::SearchOptions => " search options ",
            Kind::TabSize => " tab size ",
            Kind::Symbols => " symbols ",
            Kind::RemoteAddress => " connect to remote ",
            Kind::CodeActions => " code actions ",
            Kind::FilesystemActions => " filesystem actions ",
        }
    }
}

/// Which text field owns input in the Search workspace.
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
    /// Replacement is a presentation/edit-intent facet, not another dataset.
    pub replace_input: LineEdit,
    pub replacement_visible: bool,
    pub field: Field,
    workset: workset::Workset,
    pub items: Catalog,
    pub rows: Vec<Row>,
    match_columns: Vec<u32>,
    pub selected: usize,
    /// First logical result in the viewport, independent of replacement visibility.
    pub scroll_top: usize,
    /// Streaming sources: true while the worker may still send.
    pub streaming: bool,
    /// A source error (rg's stderr, a dead worker): sticky in the card —
    /// the transient modeline clears on the next keystroke, this doesn't.
    /// An error blocks accepting results.
    pub error: Option<String>,
    /// A successful source's advisory (rg's exit-0 stderr chatter, e.g.
    /// a malformed ignore line): displayed like the error headline, but
    /// the streamed results remain valid and accepting stays possible
    /// (0051 §3: named boundaries, not blocked operations).
    pub warning: Option<String>,
    /// Trailing catalog items that filtering never hides (pinned_tail).
    pub pinned_tail: usize,
    /// The ranking needle when it differs from the raw input (0051:
    /// qualifiers never fuzzy-match paths — only the free text ranks).
    pub rank_query: Option<String>,
    pub rank_mode: rank::MatchMode,
}

impl Picker {
    pub fn new(kind: Kind, items: Vec<Item>, streaming: bool) -> Self {
        let mut p = Self {
            kind,
            input: LineEdit::default(),
            replace_input: LineEdit::default(),
            replacement_visible: false,
            field: Field::Search,
            workset: workset::Workset::default(),
            items: Catalog::default(),
            rows: Vec::new(),
            match_columns: Vec::new(),
            selected: 0,
            scroll_top: 0,
            streaming,
            error: None,
            warning: None,
            pinned_tail: 0,
            rank_query: None,
            rank_mode: rank::MatchMode::default(),
        };
        p.append(items);
        p
    }

    pub fn search(items: Vec<Item>, streaming: bool, replacement_visible: bool) -> Self {
        let mut picker = Self::new(Kind::Search, items, streaming);
        picker.replacement_visible = replacement_visible;
        picker
    }

    /// Presentation-only: both parked editors retain text, caret and modal state.
    pub fn toggle_replacement(&mut self) {
        debug_assert_eq!(self.kind, Kind::Search);
        self.replacement_visible = !self.replacement_visible;
        self.field = if self.replacement_visible {
            Field::Replace
        } else {
            Field::Search
        };
    }

    pub fn reveal_selected(&mut self, visible: usize) {
        let visible = visible.max(1);
        if self.selected < self.scroll_top {
            self.scroll_top = self.selected;
        } else if self.selected >= self.scroll_top.saturating_add(visible) {
            self.scroll_top = self.selected + 1 - visible;
        }
        self.scroll_top = self.scroll_top.min(self.rows.len().saturating_sub(1));
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

    /// Bracketed paste into the focused field at its caret — one text
    /// payload, never keystrokes, and valid in either field mode (the
    /// ex line's pending reducer pastes the same way). Returns true
    /// when the search field changed, so the glue refreshes results.
    pub fn paste(&mut self, text: &str) -> bool {
        let search = self.field == Field::Search;
        self.active().insert_str(text);
        search
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

    /// Arrow-key caret moves work in both modes, on the focused field.
    pub fn caret_left(&mut self) {
        self.active().move_left();
    }

    pub fn caret_right(&mut self) {
        self.active().move_right();
    }

    /// Field movement never moves either editor's parked caret.
    pub fn toggle_field(&mut self) {
        self.field = match self.field {
            Field::Search => Field::Replace,
            Field::Replace => Field::Search,
        };
    }

    /// Exclude/include the selected row from the apply set (0007 §2).
    pub fn toggle_excluded(&mut self) -> bool {
        self.rows
            .get(self.selected)
            .is_some_and(|row| self.workset.toggle_row(&self.items[row.item].payload))
    }

    /// File exclusion preserves each match's individual decision.
    pub fn toggle_file_excluded(&mut self) -> bool {
        self.rows
            .get(self.selected)
            .is_some_and(|row| self.workset.toggle_file(&self.items[row.item].payload))
    }

    /// True when the item is out of the apply set (row or file).
    pub fn is_excluded(&self, item: usize) -> bool {
        self.workset.is_excluded(&self.items[item].payload)
    }

    /// The included, currently published workset shared by Collect and Review.
    pub fn accepted(&self) -> impl Iterator<Item = &Item> {
        self.rows
            .iter()
            .filter(|row| !self.is_excluded(row.item))
            .map(|row| &self.items[row.item])
    }

    pub fn filter_request(&self) -> FilterRequest {
        FilterRequest {
            catalog: self.items.clone(),
            query: self
                .rank_query
                .clone()
                .unwrap_or_else(|| self.input.text.clone()),
            upstream_filtered: self.kind == Kind::Search,
            pinned_tail: self.pinned_tail,
            mode: self.rank_mode.clone(),
        }
    }

    pub fn install_ranking(&mut self, ranking: Ranking) -> bool {
        if ranking.item_count() != self.items.len() {
            return false;
        }
        let selected = self.rows.get(self.selected).map(|row| row.item);
        let top = self.rows.get(self.scroll_top).map(|row| row.item);
        let rows = ranking.rows;
        let locate = |item: Option<usize>, fallback: usize| {
            item.and_then(|item| {
                rows.get(item)
                    .filter(|row| row.item == item)
                    .map(|_| item)
                    .or_else(|| rows.iter().position(|row| row.item == item))
            })
            .unwrap_or_else(|| fallback.min(rows.len().saturating_sub(1)))
        };
        self.selected = locate(selected, self.selected);
        self.scroll_top = locate(top, self.scroll_top);
        self.rows = rows;
        self.match_columns = ranking.columns;
        true
    }

    pub fn match_columns(&self, row: &Row) -> &[u32] {
        &self.match_columns[row.matches.clone()]
    }

    pub fn clear_results(&mut self) {
        self.rows.clear();
        self.match_columns.clear();
        self.selected = 0;
        self.scroll_top = 0;
    }

    pub fn clear_items(&mut self) {
        self.items.clear();
        self.clear_results();
        self.workset = workset::Workset::default();
    }

    /// Refresh the same query without transferring index-based decisions.
    pub fn refresh_items(&mut self) {
        self.items.clear();
        self.clear_results();
        self.workset.begin_refresh();
    }

    pub fn finish_workset_refresh(&mut self) -> usize {
        self.workset.finish_refresh()
    }

    pub fn excluded_count(&self) -> usize {
        self.workset.count()
    }
    pub fn file_match_count(&self, item: usize) -> Option<usize> {
        self.workset.file_count(&self.items.get(item)?.payload)
    }
    pub fn workset_snapshot(&self) -> WorksetSnapshot {
        self.workset.snapshot()
    }

    pub fn append(&mut self, items: Vec<Item>) {
        for item in &items {
            self.workset.observe(&item.payload);
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
    let mut end = start.saturating_add(match_len).min(line_text.len());
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
                badge: None,
                text: "src/main.rs".into(),
                payload: Payload::File("src/main.rs".into()),
            },
            Item {
                badge: None,
                text: "src/render.rs".into(),
                payload: Payload::File("src/render.rs".into()),
            },
            Item {
                badge: None,
                text: "tests/e2e.py".into(),
                payload: Payload::File("tests/e2e.py".into()),
            },
        ];
        let mut p = Picker::new(Kind::Files, items, false);
        for c in "render".chars() {
            p.push_char(c);
        }
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.current().unwrap().text, "src/render.rs");
    }

    #[test]
    fn pinned_tail_survives_filtering_below_real_matches() {
        // The remote destinations picker pins "Add a host…": filtering a
        // query that matches nothing real still offers it, and any real
        // match ranks above it.
        let items = vec![
            Item {
                badge: None,
                text: "prtdv-pw-846".into(),
                payload: Payload::RemoteConnect,
            },
            Item {
                badge: None,
                text: "Add a host\u{2026}".into(),
                payload: Payload::RemoteConnect,
            },
        ];
        let mut p = Picker::new(Kind::RemoteHosts, items, false);
        p.pinned_tail = 1;
        for c in "ewosd".chars() {
            p.push_char(c);
        }
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
        assert_eq!(p.rows.len(), 1, "the pinned row survives a dead query");
        assert_eq!(p.current().unwrap().text, "Add a host\u{2026}");
        let mut p = Picker::new(
            Kind::RemoteHosts,
            vec![
                Item {
                    badge: None,
                    text: "prtdv-pw-846".into(),
                    payload: Payload::RemoteConnect,
                },
                Item {
                    badge: None,
                    text: "Add a host\u{2026}".into(),
                    payload: Payload::RemoteConnect,
                },
            ],
            false,
        );
        p.pinned_tail = 1;
        for c in "prtdv".chars() {
            p.push_char(c);
        }
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
        assert_eq!(p.rows.len(), 2);
        assert_eq!(
            p.current().unwrap().text,
            "prtdv-pw-846",
            "a real match ranks above the pinned row"
        );
    }

    #[test]
    fn selection_wraps() {
        let mut arena: strop_core::id::Arena<strop_core::id::DocumentKind, ()> =
            strop_core::id::Arena::default();
        let items: Vec<Item> = (0..3)
            .map(|i| Item {
                badge: None,
                text: format!("f{i}"),
                payload: Payload::Buffer(arena.insert(())),
            })
            .collect();
        let mut p = Picker::new(Kind::Buffers, items, false);
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
        p.move_by(-1);
        assert_eq!(p.selected, 2);
        p.move_by(1);
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn ctrl_d_excludes_a_whole_file() {
        let hit = |path: &str| Item {
            badge: None,
            text: path.into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(PathBuf::from(path)),
                line: 1,
                col: 1,
                match_len: 1,
                line_text: "x".into(),
            },
        };
        let mut p = Picker::search(vec![hit("a.rs"), hit("a.rs"), hit("b.rs")], false, true);
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
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
            Kind::Search,
            vec![
                Item {
                    badge: None,
                    text: "a.rs:1 foo".into(),
                    payload: Payload::File("a.rs".into()),
                },
                Item {
                    badge: None,
                    text: "b.rs:2 bar".into(),
                    payload: Payload::File("b.rs".into()),
                },
                Item {
                    badge: None,
                    text: "c.rs:3 baz".into(),
                    payload: Payload::File("c.rs".into()),
                },
            ],
            false,
        );
        for c in "foo|bar".chars() {
            p.push_char(c);
        }
        p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
        assert_eq!(p.rows.len(), 3);
        // and the apply set is exactly those rows
        assert_eq!(p.accepted().count(), 3);
    }
}
