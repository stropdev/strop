//! Table-derived dispatch trie and which-key index, compiled once.
//! Querying dispatch never expands notation or allocates key sequences.

use std::sync::LazyLock;

use super::{Binding, BINDINGS, SECTIONS};

/// Expand a row's notation into sequences of table tokens.
pub fn expand(keys: &str) -> Vec<Vec<&str>> {
    let toks: Vec<&str> = keys.split(' ').filter(|t| !t.is_empty()).collect();
    let mut seqs: Vec<Vec<&str>> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            "/" if seqs.is_empty() => seqs.push(vec!["/"]),
            "/" => {
                let base = match seqs.last() {
                    Some(s) => s[..s.len() - 1].to_vec(),
                    None => Vec::new(),
                };
                for alt in &toks[i + 1..] {
                    if *alt != "/" {
                        let mut seq = base.clone();
                        seq.push(alt);
                        seqs.push(seq);
                    }
                }
                break;
            }
            "space" => {
                let end = match (i + 1..toks.len()).find(|&j| toks[j] == "/" && j + 1 < toks.len())
                {
                    Some(end) => end,
                    None => toks.len(),
                };
                seqs.push(toks[i..end].to_vec());
                i = end;
                continue;
            }
            "ctrl-w" | "ctrl-x" | "ctrl-\\" | "ctrl-4" => {
                if let Some(k) = toks.get(i + 1) {
                    seqs.push(vec![toks[i], k]);
                    i += 2;
                } else {
                    seqs.push(vec![toks[i]]);
                    i += 1;
                }
                continue;
            }
            t => seqs.push(vec![t]),
        }
        i += 1;
    }
    seqs
}

fn per_key(seq: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for &t in seq {
        if t.len() > 1 && !t.starts_with('<') && !t.starts_with(':') && !NAMED.contains(&t) {
            if let Some(i) = t.find('<') {
                out.extend(t[..i].chars().map(|c| c.to_string()));
                out.push(t[i..].to_string());
            } else {
                out.extend(t.chars().map(|c| c.to_string()));
            }
        } else {
            out.push(t.to_string());
        }
    }
    out
}

pub(crate) const NAMED: &[&str] = &[
    "space",
    "ctrl-w",
    "ctrl-o",
    "ctrl-i",
    "up",
    "down",
    "left",
    "right",
    "tab",
    "s-tab",
    "esc",
    "enter",
    "backspace",
    "ctrl-r",
    "ctrl-t",
    "ctrl-x",
    "ctrl-a",
    "ctrl-c",
    "ctrl-d",
    "ctrl-u",
    "ctrl-f",
    "ctrl-b",
    "ctrl-^",
    "ctrl-v",
    "ctrl-l",
    "ctrl-space",
    "ctrl-\\",
    "ctrl-n",
    "ctrl-p",
    "ctrl-y",
    "ctrl-e",
    "ctrl-4",
];

/// The single-key operator `<` is literal, not a placeholder.
fn is_placeholder(k: &str) -> bool {
    k.len() > 1 && k.starts_with('<')
}

#[derive(Clone, Copy)]
struct NodeId(usize);

const ROOT: NodeId = NodeId(0);

#[derive(Default)]
struct Node {
    literals: Vec<(String, NodeId)>,
    wildcard: Option<NodeId>,
    row: Option<usize>,
    // Strict descendants, not the node's own terminal row.
    live_child: bool,
}

struct HintSequence {
    row: usize,
    tokens: Vec<&'static str>,
    flat: String,
    bounds: Vec<usize>,
    char_len: usize,
    weight: usize,
}

impl HintSequence {
    fn new(row: usize, tokens: Vec<&'static str>) -> Self {
        let mut flat = String::new();
        let mut bounds = Vec::new();
        let mut weight = 0;
        for &t in &tokens {
            bounds.push(flat.len());
            let text = if t == "space" { " " } else { t };
            flat.push_str(text);
            weight += text.len();
        }
        let char_len = flat.chars().count();
        Self {
            row,
            tokens,
            flat,
            bounds,
            char_len,
            weight,
        }
    }

    // Preserve table-token hint boundaries (m<a> -> <a>, gg -> g),
    // rather than displaying dispatch's per-key representation.
    fn child_key(&self, prefix: &str, plen: usize) -> Option<String> {
        if plen == 0 || !self.flat.starts_with(prefix) || self.char_len <= plen {
            return None;
        }
        match self.bounds.iter().position(|&b| b == plen) {
            Some(i) => Some(self.tokens[i].to_string()),
            None => {
                let i = self.bounds.iter().rposition(|&b| b < plen)?;
                Some(self.tokens[i].chars().skip(plen - self.bounds[i]).collect())
            }
        }
    }
}

struct Index {
    nodes: Vec<Node>,
    hints: Vec<HintSequence>,
}

/// Normal's keys: every row but Visual's own verbs.
static INDEX: LazyLock<Index> = LazyLock::new(|| Index::compile(false));
/// Visual's own keys: the rows of the `visual` section. A key that means
/// something else in Normal (`u`, `p`, `J`) is a different row here.
static VISUAL: LazyLock<Index> = LazyLock::new(|| Index::compile(true));

impl Index {
    fn compile(visual: bool) -> Self {
        let mut index = Self {
            nodes: vec![Node::default()],
            hints: Vec::new(),
        };
        for (row, binding) in BINDINGS.iter().enumerate() {
            let member = if visual {
                binding.sections.contains(&"visual")
            } else {
                !matches!(binding.handler, super::Handler::Visual(_))
            };
            for tokens in expand(binding.keys) {
                if member && binding.live && !matches!(binding.handler, super::Handler::Contextual)
                {
                    index.insert(row, per_key(&tokens));
                }
                index.hints.push(HintSequence::new(row, tokens));
            }
        }
        // Stable sorting preserves row and alternative order for equal weights.
        index.hints.sort_by_key(|seq| seq.weight);
        index
    }

    fn insert(&mut self, row: usize, keys: Vec<String>) {
        let mut at = ROOT;
        for key in keys {
            self.nodes[at.0].live_child = true;
            let wildcard = is_placeholder(&key);
            let existing = if wildcard {
                self.nodes[at.0].wildcard
            } else {
                self.nodes[at.0]
                    .literals
                    .iter()
                    .find(|(literal, _)| literal == &key)
                    .map(|(_, id)| *id)
            };
            at = match existing {
                Some(id) => id,
                None => {
                    let id = NodeId(self.nodes.len());
                    self.nodes.push(Node::default());
                    if wildcard {
                        self.nodes[at.0].wildcard = Some(id);
                    } else {
                        self.nodes[at.0].literals.push((key, id));
                    }
                    id
                }
            };
        }
        // Compilation follows table order, so the first terminal wins.
        if self.nodes[at.0].row.is_none() {
            self.nodes[at.0].row = Some(row);
        }
    }

    fn literal_child(&self, at: NodeId, key: &str) -> Option<NodeId> {
        self.nodes[at.0]
            .literals
            .iter()
            .find(|(literal, _)| literal == key)
            .map(|(_, id)| *id)
    }

    fn find(&self, at: NodeId, path: &[String]) -> Option<usize> {
        let Some((key, rest)) = path.split_first() else {
            return self.nodes[at.0].row;
        };
        let literal = self
            .literal_child(at, key)
            .and_then(|id| self.find(id, rest));
        let wildcard = self.nodes[at.0].wildcard.and_then(|id| self.find(id, rest));
        // A literal must not automatically outrank an earlier placeholder row.
        match (literal, wildcard) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, b) => b,
        }
    }

    fn has_child(&self, at: NodeId, path: &[String]) -> bool {
        let Some((key, rest)) = path.split_first() else {
            return self.nodes[at.0].live_child;
        };
        self.literal_child(at, key)
            .is_some_and(|id| self.has_child(id, rest))
            || self.nodes[at.0]
                .wildcard
                .is_some_and(|id| self.has_child(id, rest))
    }
}

/// The first live table row completed by this path, including placeholders.
pub fn find_row(path: &[String]) -> Option<&'static Binding> {
    INDEX.find(ROOT, path).map(|row| &BINDINGS[row])
}

/// Whether a live sequence strictly extends this path.
pub fn any_child(path: &[String]) -> bool {
    INDEX.has_child(ROOT, path)
}

/// The Visual-section row completed by this path.
pub fn find_visual_row(path: &[String]) -> Option<&'static Binding> {
    VISUAL.find(ROOT, path).map(|row| &BINDINGS[row])
}

/// Whether a Visual-section sequence strictly extends this path.
pub fn any_visual_child(path: &[String]) -> bool {
    VISUAL.has_child(ROOT, path)
}

/// One which-key hint row: the next key after a pending prefix, with
/// the row's home section so cards group the way help does.
pub struct Hint {
    pub key: String,
    pub desc: &'static str,
    pub live: bool,
    pub section: &'static str,
}

/// The sections whose rows a mode can reach: Insert and Terminal rows
/// never show on a Normal-mode card, Visual sees only its own.
pub fn sections_for(mode: crate::editor::Mode) -> &'static [&'static str] {
    use crate::editor::Mode;
    match mode {
        Mode::Normal => &[
            "moving", "editing", "search", "leader", "panes", "git", "pickers", "ex",
        ],
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => &["visual"],
        Mode::Insert => &[],
    }
}

/// Mode-appropriate hints, including planned rows. Shortest sequences win;
/// table order and then alternative order break ties.
pub fn children_of(prefix: &str, mode: crate::editor::Mode) -> Vec<Hint> {
    let sections = sections_for(mode);
    let mut out: Vec<Hint> = Vec::new();
    let plen = prefix.chars().count();
    for seq in &INDEX.hints {
        let b = &BINDINGS[seq.row];
        if !b.sections.iter().any(|s| sections.contains(s)) {
            continue;
        }
        if let Some(key) = seq.child_key(prefix, plen) {
            if !out.iter().any(|hint| hint.key == key) {
                out.push(Hint {
                    key,
                    desc: b.desc,
                    live: b.live,
                    section: b.sections.first().copied().unwrap_or(""),
                });
            }
        }
    }
    out
}

/// The vim-compatibility report, generated from the single binding table.
pub fn compat_report() -> String {
    let mut out = String::from(
        "# Vim compatibility\n\nGenerated from the command table (`strop compat`).\n\
         `✓` works today; `(soon)` is planned. What Neovim binds and strop\n\
         does not is listed at the end, under Missing and Reassigned.\n",
    );
    for section in SECTIONS {
        out.push_str(&format!("\n## {}\n\n", section.title));
        for b in BINDINGS.iter().filter(|b| b.sections.contains(&section.id)) {
            let mark = if b.live { "✓" } else { "·" };
            let soon = if b.live { "" } else { " (soon)" };
            out.push_str(&format!("- `{mark} {}` — {}{}\n", b.keys, b.desc, soon));
        }
    }
    out.push_str(
        "\n## Missing\n\nNeovim binds these; in strop the key does nothing. \
         An operator on this list\n(`=`, `gq`, `gw`, `zf`) still takes its motion, \
         so the motion's keys never run as\ncommands.\n\n",
    );
    for row in super::missing::MISSING {
        out.push_str(&format!(
            "- {}: `{}` — {}\n",
            row.place.title(),
            row.keys.join("` `"),
            row.what
        ));
    }
    out.push_str(
        "\n## Reassigned\n\nNeovim binds these; strop uses the key for something else.\n\n",
    );
    for row in super::missing::REASSIGNED {
        out.push_str(&format!(
            "- {}: `{}` — Neovim: {}. strop: {}.\n",
            row.place.title(),
            row.keys,
            row.neovim,
            row.strop
        ));
    }
    out
}

#[cfg(test)]
mod index_tests {
    use super::*;

    #[test]
    fn table_precedence_wins_over_literal_specificity() {
        let mut index = Index {
            nodes: vec![Node::default()],
            hints: Vec::new(),
        };
        index.insert(1, vec!["g".into(), "<c>".into()]);
        index.insert(3, vec!["g".into(), "x".into()]);
        assert_eq!(index.find(ROOT, &["g".into(), "x".into()]), Some(1));
        assert_eq!(index.find(ROOT, &["g".into(), "z".into()]), Some(1));
        assert!(index.has_child(ROOT, &["g".into()]));
        assert!(!index.has_child(ROOT, &["g".into(), "x".into()]));
    }

    #[test]
    fn literals_and_longer_paths_keep_independent_terminals() {
        let mut index = Index {
            nodes: vec![Node::default()],
            hints: Vec::new(),
        };
        index.insert(0, vec!["g".into(), "x".into()]);
        index.insert(2, vec!["g".into(), "<c>".into()]);
        index.insert(4, vec!["g".into(), "x".into(), "y".into()]);
        assert_eq!(index.find(ROOT, &["g".into(), "x".into()]), Some(0));
        assert_eq!(index.find(ROOT, &["g".into(), "z".into()]), Some(2));
        assert!(index.has_child(ROOT, &["g".into(), "x".into()]));
        assert_eq!(
            index.find(ROOT, &["g".into(), "x".into(), "y".into()]),
            Some(4)
        );
    }
}
