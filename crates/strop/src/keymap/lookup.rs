//! keymap/lookup.rs — the table's query engine: sequence
//! expansion, trie lookup (exact row / prefix child), which-key
//! hints, and the generated vim-compat report. The table in
//! `mod.rs` is data; this file is the only code that walks it.

use super::{Binding, BINDINGS, SECTIONS};

/// Expand a row's `keys` into its sequences (see the notation above).
/// Dispatch lookup: the sequence (walker's tokens) → its row.
/// `<c>`/`<a>` in a row's keys match any char (parameterized rows).
pub fn expand(keys: &str) -> Vec<Vec<&str>> {
    let toks: Vec<&str> = keys.split(' ').filter(|t| !t.is_empty()).collect();
    let mut seqs: Vec<Vec<&str>> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            // a leading bare `/` is the search-forward key
            "/" if seqs.is_empty() => seqs.push(vec!["/"]),
            // alternatives replace the previous sequence's last key and
            // run to the row's end
            "/" => {
                let base: Vec<&str> = seqs
                    .last()
                    .map(|s| s[..s.len() - 1].to_vec())
                    .unwrap_or_default();
                for alt in &toks[i + 1..] {
                    if *alt != "/" {
                        let mut seq = base.clone();
                        seq.push(alt);
                        seqs.push(seq);
                    }
                }
                break;
            }
            // the leader: its sequence runs to the row's end — or the
            // first `/` with keys after it (a trailing `/` is the
            // grep key: "space /" is one sequence)
            "space" => {
                let end = (i + 1..toks.len())
                    .find(|&j| toks[j] == "/" && j + 1 < toks.len())
                    .unwrap_or(toks.len());
                seqs.push(toks[i..end].to_vec());
                i = end;
                continue;
            }
            // window commands take exactly one key
            "ctrl-w" => {
                if let Some(k) = toks.get(i + 1) {
                    seqs.push(vec!["ctrl-w", k]);
                    i += 2;
                } else {
                    seqs.push(vec!["ctrl-w"]);
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

/// A row's sequences at PER-KEY granularity (0016: the machine's trie
/// walks keys, not row tokens): "gg" is ["g","g"], "ctrl-w h" is
/// ["ctrl-w","h"], placeholders ("<a>") stay whole and match any key.
pub fn key_seqs(row: &Binding) -> Vec<Vec<String>> {
    expand(row.keys)
        .iter()
        .map(|seq| {
            let mut out: Vec<String> = Vec::new();
            for t in seq {
                if t.len() > 1 && !t.starts_with('<') && !t.starts_with(':') && !NAMED.contains(t) {
                    if let Some(i) = t.find('<') {
                        // "r<c>": the key chars, then the placeholder whole
                        for c in t[..i].chars() {
                            out.push(c.to_string());
                        }
                        out.push(t[i..].to_string());
                    } else {
                        for c in t.chars() {
                            out.push(c.to_string());
                        }
                    }
                } else {
                    out.push(t.to_string());
                }
            }
            out
        })
        .collect()
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
    "ctrl-x",
    "ctrl-d",
    "ctrl-u",
    "ctrl-f",
    "ctrl-b",
    "ctrl-^",
    "ctrl-v",
    "ctrl-l",
];

/// A placeholder token ("<a>") matches any key; the operator "<" is
/// a literal (len-1) and must not.
fn is_placeholder(k: &str) -> bool {
    k.len() > 1 && k.starts_with('<')
}

fn seq_matches(seq: &[String], path: &[String]) -> bool {
    seq.len() == path.len()
        && seq
            .iter()
            .zip(path)
            .all(|(k, t)| is_placeholder(k) || k == t)
}

fn seq_has_prefix(seq: &[String], path: &[String]) -> bool {
    seq.len() > path.len()
        && seq
            .iter()
            .zip(path)
            .all(|(k, t)| is_placeholder(k) || k == t)
}

/// The row a key path completes exactly (the machine's trie lookup).
pub fn find_row(path: &[String]) -> Option<&'static Binding> {
    BINDINGS
        .iter()
        .find(|b| b.live && key_seqs(b).iter().any(|seq| seq_matches(seq, path)))
}

/// Any live row whose sequence EXTENDS this path (trie prefix check —
/// the machine's prefixes derive from the table, never a list).
pub fn any_child(path: &[String]) -> bool {
    BINDINGS
        .iter()
        .any(|b| b.live && key_seqs(b).iter().any(|seq| seq_has_prefix(seq, path)))
}

/// One which-key hint row: the next key after a pending prefix.
pub struct Hint {
    pub key: String,
    pub desc: &'static str,
    pub live: bool,
}

/// The which-key card for a pending `prefix` in `mode`: every binding
/// that continues the prefix, keyed by the immediately-next key. When
/// several rows share a next key (the `space g` verbs under `space`),
/// the shortest sequence wins — the prefix's own row, not its first
/// verb. Only mode-appropriate sections feed the card: visual mode
/// never shows normal-mode leader verbs it can't run.
pub fn children_of(prefix: &str, mode: crate::editor::Mode) -> Vec<Hint> {
    use crate::editor::Mode;
    let sections: &[&str] = match mode {
        Mode::Normal => &["normal", "leader", "git", "ex+panes"],
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => &["visual"],
        Mode::Insert => &[],
    };
    let mut cands: Vec<(usize, Hint)> = Vec::new();
    for b in BINDINGS.iter().filter(|b| sections.contains(&b.section)) {
        for seq in expand(b.keys) {
            if let Some(key) = child_key(&seq, prefix) {
                let len: usize = seq
                    .iter()
                    .map(|t| if *t == "space" { 1 } else { t.len() })
                    .sum();
                cands.push((
                    len,
                    Hint {
                        key,
                        desc: b.desc,
                        live: b.live,
                    },
                ));
            }
        }
    }
    cands.sort_by_key(|(len, _)| *len); // shortest wins; table order breaks ties
    let mut out: Vec<Hint> = Vec::new();
    for (_, h) in cands {
        if !out.iter().any(|x| x.key == h.key) {
            out.push(h);
        }
    }
    out
}

/// The next key of `seq` under pending `prefix`: the whole token when
/// the prefix ends on a token boundary, else the rest of the partial
/// token (pending `g` vs `gg` → `g`; pending `m` vs `m<a>` → `<a>`).
fn child_key(seq: &[&str], prefix: &str) -> Option<String> {
    let mut flat = String::new();
    let mut bounds = Vec::new();
    for t in seq {
        bounds.push(flat.len());
        flat.push_str(if *t == "space" { " " } else { t });
    }
    let plen = prefix.chars().count();
    if plen == 0 || !flat.starts_with(prefix) || flat.chars().count() <= plen {
        return None;
    }
    match bounds.iter().position(|b| *b == plen) {
        Some(i) => Some(seq[i].to_string()),
        None => {
            let i = bounds.iter().rposition(|b| *b < plen)?;
            Some(seq[i].chars().skip(plen - bounds[i]).collect())
        }
    }
}

/// The vim-compatibility report (0016): generated from this table —
/// docs can never drift from dispatch. Checked into docs/vim-compat.md;
/// the test pins freshness (STROP_REGEN=1 cargo test regenerates).
pub fn compat_report() -> String {
    let mut out = String::from(
        "# Vim compatibility\n\nGenerated from the command table (`cargo test` pins freshness; \
         STROP_REGEN=1 rewrites).\n`✓` ships exactly; `(soon)` is a planned slot.\n",
    );
    for section in SECTIONS {
        out.push_str(&format!("\n## {section}\n\n"));
        for b in BINDINGS.iter().filter(|b| b.section == *section) {
            let mark = if b.live { "✓" } else { "·" };
            let soon = if b.live { "" } else { " (soon)" };
            out.push_str(&format!("- `{mark} {}` — {}{}\n", b.keys, b.desc, soon));
        }
    }
    out
}
