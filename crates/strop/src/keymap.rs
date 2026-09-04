//! The keymap table (0008 §3 — keymaps as data). v1: this table drives
//! the `Space ?` keybinds popup and the which-key prefix cards; the trie
//! dispatch cutover (0008 §5) makes it the dispatch source later.
//!
//! Coverage contract (0003 §5.7): every dispatchable binding renders in
//! the popup; the tests below pin both directions. `live: false` rows
//! are planned slots — reserved keys with no dispatch yet (0003 §6) —
//! and render muted with a "(soon)" suffix, never as live bindings.
//!
//! ## Notation
//!
//! `keys` is space-separated tokens, one key per token (`esc`, `:w`,
//! `"+y`; `<a>`/`<c>` parameterize a char):
//!
//! - `space` is the leader: its sequence runs to the row's end.
//! - `ctrl-w` takes exactly one following key.
//! - ` / ` separates alternatives that replace the last key of the
//!   sequence before them (`space d / k` → `space d`, `space k`);
//!   alternatives run to the row's end.
//! - Every other token is one single-key sequence; a row may list
//!   several (`h j k l`), and a leading bare `/` is the search-forward
//!   key (`/ ?`).

/// One binding as it appears in `Space ?` / which-key.
pub struct Binding {
    pub keys: &'static str,
    pub desc: &'static str,
    /// Sidebar section: normal · visual · insert · leader · git · ex+panes
    pub section: &'static str,
    /// false = planned slot: no dispatch yet, renders muted "(soon)".
    pub live: bool,
}

pub const SECTIONS: &[&str] = &["normal", "visual", "insert", "leader", "git", "ex+panes"];

pub const BINDINGS: &[Binding] = &[
    // normal: motions
    Binding {
        keys: "h j k l",
        desc: "move (never off the line)",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "w b e W B E",
        desc: "word / WORD motions",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "0 $ G %",
        desc: "line/file/pair jumps",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "gg",
        desc: "top of file",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "gd",
        desc: "goto definition (LSP)",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "f<c> F<c> t<c> T<c>",
        desc: "find/till char (candidates light up)",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "/ ?",
        desc: "search forward / backward",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "n N",
        desc: "next / prev match",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "]c [c",
        desc: "next / prev git hunk",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "m<a>",
        desc: "set mark at cursor",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "'a `a",
        desc: "jump to mark",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "Q",
        desc: "toggle cursor at point (multicursor)",
        section: "normal",
        live: true,
    },
    // normal: operators
    Binding {
        keys: "d y c > <",
        desc: "operators + motion/object (live preview)",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "dd yy cc D C Y s x X",
        desc: "line/char shortcuts",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "iw i\" i' i( i[ i{",
        desc: "inner objects (quotes scan the line)",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "ds\" cs\"' ysiw\"",
        desc: "surround: delete / change / add",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "i a A o O",
        desc: "insert (auto-indent)",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "p P",
        desc: "paste after / before",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "r<c> J .",
        desc: "replace char / join / repeat",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "u ctrl-r",
        desc: "undo / redo (one unit per command)",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "\"+y \"+p \"+P",
        desc: "system clipboard: yank / paste after / before",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "\"xy \"xp",
        desc: "named register: yank / paste",
        section: "normal",
        live: true,
    },
    Binding {
        keys: "v V",
        desc: "visual / visual-line",
        section: "normal",
        live: true,
    },
    // visual
    Binding {
        keys: "d y c x",
        desc: "operate on selection",
        section: "visual",
        live: true,
    },
    Binding {
        keys: "S<c>",
        desc: "wrap selection in pair",
        section: "visual",
        live: true,
    },
    Binding {
        keys: "i<a> a<a>",
        desc: "objects select (vi[ works)",
        section: "visual",
        live: true,
    },
    Binding {
        keys: "space y",
        desc: "yank selection → clipboard",
        section: "visual",
        live: true,
    },
    // insert
    Binding {
        keys: "esc",
        desc: "normal mode (session = one undo unit)",
        section: "insert",
        live: true,
    },
    Binding {
        keys: "backspace",
        desc: "delete back",
        section: "insert",
        live: true,
    },
    Binding {
        keys: "enter",
        desc: "new line (auto-indent)",
        section: "insert",
        live: true,
    },
    Binding {
        keys: "} ] )",
        desc: "closer on indent-only line dedents",
        section: "insert",
        live: true,
    },
    // leader
    Binding {
        keys: "space f",
        desc: "file finder",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space b",
        desc: "buffers (MRU)",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space /",
        desc: "live grep",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space R",
        desc: "global search & replace",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space ?",
        desc: "this popup",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space y",
        desc: "yank motion → system clipboard",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space p",
        desc: "paste clipboard after",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space P",
        desc: "paste clipboard before",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space d",
        desc: "diagnostics picker",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space k",
        desc: "hover docs",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space j",
        desc: "jumplist picker",
        section: "leader",
        live: false,
    },
    Binding {
        keys: "space u",
        desc: "undo-tree browser",
        section: "leader",
        live: true,
    },
    Binding {
        keys: "space c",
        desc: "cursor on next line too (multicursor)",
        section: "leader",
        live: true,
    },
    // git
    Binding {
        keys: "space g",
        desc: "git…",
        section: "git",
        live: true,
    },
    Binding {
        keys: "space g l",
        desc: "commit browser",
        section: "git",
        live: true,
    },
    Binding {
        keys: "space g h",
        desc: "file history",
        section: "git",
        live: true,
    },
    Binding {
        keys: "space g b",
        desc: "toggle blame gutter / card",
        section: "git",
        live: true,
    },
    Binding {
        keys: "space g y",
        desc: "permalink: copy",
        section: "git",
        live: true,
    },
    Binding {
        keys: "space g o",
        desc: "permalink: open",
        section: "git",
        live: true,
    },
    Binding {
        keys: "space g u",
        desc: "hunk: undo (reset to HEAD)",
        section: "git",
        live: true,
    },
    Binding {
        keys: "space g s",
        desc: "hunk: stage",
        section: "git",
        live: true,
    },
    Binding {
        keys: "space g p",
        desc: "hunk: preview",
        section: "git",
        live: true,
    },
    Binding {
        keys: "]f [f",
        desc: "next / prev file in commit diff",
        section: "git",
        live: true,
    },
    Binding {
        keys: "enter",
        desc: "dive into the line's commit (blame gutter)",
        section: "git",
        live: true,
    },
    Binding {
        keys: "q",
        desc: "close surface (readonly buffers)",
        section: "git",
        live: true,
    },
    // ex + panes
    Binding {
        keys: ":w :q :q! :wq",
        desc: "write / quit (force) / write-quit",
        section: "ex+panes",
        live: true,
    },
    Binding {
        keys: ":e",
        desc: "edit file",
        section: "ex+panes",
        live: true,
    },
    Binding {
        keys: ":help",
        desc: "help buffer (this text — / searches it)",
        section: "ex+panes",
        live: true,
    },
    Binding {
        keys: ":vs :sp",
        desc: "split vertical / horizontal",
        section: "ex+panes",
        live: true,
    },
    Binding {
        keys: "ctrl-w h / l / j / k / w",
        desc: "pane move / cycle",
        section: "ex+panes",
        live: true,
    },
    Binding {
        keys: "ctrl-w v / s",
        desc: "pane split (vs / sp)",
        section: "ex+panes",
        live: true,
    },
    Binding {
        keys: "ctrl-w q",
        desc: "close pane (last → buffer)",
        section: "ex+panes",
        live: true,
    },
    Binding {
        keys: "up down tab s-tab",
        desc: "picker navigation",
        section: "ex+panes",
        live: true,
    },
    Binding {
        keys: "ctrl-x",
        desc: "replace picker: exclude/include match",
        section: "ex+panes",
        live: true,
    },
];

/// Expand a row's `keys` into its sequences (see the notation above).
pub(crate) fn expand(keys: &str) -> Vec<Vec<&str>> {
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
        Mode::Visual | Mode::VisualLine => &["visual"],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Editor, Mode};
    use strop_core::Buffer;

    /// Every section has at least one binding; no empty rows; the
    /// "(soon)" marker comes from `live: false` only, never the desc —
    /// a desc carrying it would render the suffix twice.
    #[test]
    fn coverage_shape() {
        for section in SECTIONS {
            assert!(
                BINDINGS.iter().any(|b| b.section == *section),
                "section {section} is empty"
            );
        }
        for b in BINDINGS {
            assert!(!b.keys.is_empty() && !b.desc.is_empty());
            assert!(
                !b.desc.contains("(soon)"),
                "{}: '(soon)' belongs to live:false, not the desc",
                b.keys
            );
        }
    }

    /// The notation expands: leaders glue, alternatives fan out, a
    /// leading `/` is the search-forward key (never an alternative).
    #[test]
    fn keys_notation_expands() {
        assert_eq!(expand("space f"), vec![vec!["space", "f"]]);
        assert_eq!(expand("space /"), vec![vec!["space", "/"]]);
        assert_eq!(
            expand("space g u / s / p"),
            vec![
                vec!["space", "g", "u"],
                vec!["space", "g", "s"],
                vec!["space", "g", "p"]
            ]
        );
        assert_eq!(expand("/ ?"), vec![vec!["/"], vec!["?"]]);
        assert_eq!(expand("h j k l").len(), 4);
        assert_eq!(
            expand("ctrl-w h / l"),
            vec![vec!["ctrl-w", "h"], vec!["ctrl-w", "l"]]
        );
        for b in BINDINGS {
            let seqs = expand(b.keys);
            assert!(!seqs.is_empty(), "{}: no sequences", b.keys);
            assert!(seqs.iter().all(|s| !s.is_empty()));
        }
    }

    /// Every pending prefix with a card has hints; the space card points
    /// `g` at the git prefix row (shortest sequence wins over the
    /// verbs); soon rows come through as soon.
    #[test]
    fn which_key_children() {
        for p in [" ", " g", "g", "[", "]", "m", "'", "`"] {
            assert!(
                !children_of(p, Mode::Normal).is_empty(),
                "no hints for pending {p:?}"
            );
        }
        let space = children_of(" ", Mode::Normal);
        let g = space.iter().find(|h| h.key == "g").expect("space g hint");
        assert_eq!(g.desc, "git…");
        assert!(g.live);
        assert!(
            space.iter().any(|h| h.key == "j" && !h.live),
            "space j renders as soon"
        );
        let git = children_of(" g", Mode::Normal);
        assert_eq!(git.len(), 8);
        assert!(git.iter().any(|h| h.key == "u" && h.desc.contains("undo")));
        assert_eq!(children_of("m", Mode::Normal)[0].key, "<a>");
        assert_eq!(children_of("g", Mode::Normal).len(), 2); // gg, gd
                                                             // visual mode: only the visual table feeds the card
        let v = children_of(" ", Mode::Visual);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].key, "y");
        assert!(v[0].desc.contains("selection"));
    }

    /// Every dispatchable sequence, enumerated from the match arms in
    /// editor/{normal,visual,insert,git,git_memory,picker,panes}.rs and
    /// mod.rs's surface/card handlers. Adding a dispatch arm: add its
    /// sequence here AND the row it needs in BINDINGS — this test fails
    /// until both exist. Single-key motions, operators, aliases and
    /// count prefixes are the grammar's leaves (the resolver owns them;
    /// 0008's Count/Dynamic trie nodes make that structural); only
    /// sequences the hand-written match tree completes on its own are
    /// listed.
    ///
    /// The trie cutover (0008 §5) DELETES this list: dispatch walks the
    /// keymap trie, so leaves == BINDINGS rows mechanically and this
    /// hand enumeration is dead weight.
    const DISPATCHED: &[&str] = &[
        // leader (normal mode)
        "space f",
        "space b",
        "space /",
        "space R",
        "space ?",
        "space d",
        "space k",
        "space y",
        "space p",
        "space P",
        // git namespace
        "space g u",
        "space g s",
        "space g p",
        "space g l",
        "space g h",
        "space g b",
        "space g y",
        "space g o",
        "space u",
        // multicursor (0013)
        "Q",
        "space c",
        // search repeat (normal mode)
        "n",
        "N",
        // prefixes with their own which-key cards
        "gg",
        "gd",
        "]c",
        "[c",
        "]f",
        "[f",
        "m<a>",
        "'a",
        "`a",
        "r<c>",
        // registers
        "\"xy",
        "\"xp",
        "\"+y",
        "\"+p",
        "\"+P",
        // ex + panes
        ":w",
        ":q",
        ":q!",
        ":wq",
        ":e",
        ":help",
        ":e!",
        ":vs",
        ":vsplit",
        ":sp",
        ":split",
        "ctrl-w h",
        "ctrl-w l",
        "ctrl-w j",
        "ctrl-w k",
        "ctrl-w w",
        "ctrl-w v",
        "ctrl-w s",
        "ctrl-w q",
        // picker (0007 replace incl. ctrl-x), insert closers, surfaces
        "up",
        "down",
        "tab",
        "s-tab",
        "ctrl-x",
        "}",
        "]",
        ")",
        "enter",
        "esc",
        "backspace",
        "q",
        // visual
        "S<c>",
        "space y",
    ];

    /// Long-form ex commands dispatch to the same arms as their short
    /// forms; they need no separate rows.
    const EX_ALIASES: &[(&str, &str)] = &[(":vsplit", ":vs"), (":split", ":sp"), (":e!", ":e")];

    /// 0003 §5.7, reverse direction: every dispatchable sequence has a
    /// BINDINGS row. A new match arm without a row fails here.
    #[test]
    fn every_dispatched_sequence_has_a_row() {
        for entry in DISPATCHED {
            let canonical = EX_ALIASES
                .iter()
                .find(|(a, _)| a == entry)
                .map(|(_, c)| *c)
                .unwrap_or(*entry);
            let want: Vec<&str> = canonical.split(' ').collect();
            assert!(
                BINDINGS.iter().any(|b| expand(b.keys).contains(&want)),
                "{entry} dispatches but has no BINDINGS row (0003 §5.7)"
            );
        }
    }

    /// 0003 §5.7, forward direction: every live leader/git-space
    /// binding, fed through a real editor, does something observable.
    /// A silent no-op means dispatch drifted from the table (or the row
    /// lies about being live) — either way the test fails.
    ///
    /// Probe heuristic: git/lsp-dependent verbs legitimately degrade to
    /// a message ("no hunk here", "no language server") in a bare
    /// scratch buffer; what must never happen is the unknown-key no-op
    /// (empty pending, empty message, no state change) that an unmapped
    /// key produces.
    #[test]
    fn live_leader_bindings_reach_dispatch() {
        for b in BINDINGS
            .iter()
            .filter(|b| b.live && b.keys.starts_with("space"))
        {
            for seq in expand(b.keys) {
                let keys = seq
                    .iter()
                    .map(|t| if *t == "space" { " " } else { t })
                    .collect::<String>();
                let mut e = Editor::new(Buffer::from_text("x\n"));
                e.feed_text(&keys);
                assert!(
                    dispatched_something(&e),
                    "{} (fed as {keys:?}) was a no-op — table drift",
                    b.keys
                );
            }
        }
    }

    fn dispatched_something(e: &Editor) -> bool {
        !e.message.is_empty()
            || e.picker_open()
            || !e.pending.is_empty()
            || e.clip_paste_pending.is_some()
            || e.osc52.is_some()
            || e.mode != Mode::Normal
            || e.buffers.len() != 1
            || e.cursor != 0
            || e.hover_card.is_some()
            || e.blame_card.is_some()
    }

    /// Parameterized leaves the match tree completes on its own, fed
    /// concretely: marks, hunk jumps, gd without an LSP, and the
    /// clipboard register staged via OSC52.
    #[test]
    fn parameterized_leaves_reach_dispatch() {
        let mut e = Editor::new(Buffer::from_text("one two\n"));
        e.feed_text("ma");
        assert_eq!(e.message, "mark a set");
        e.feed_text("'b");
        assert!(e.message.contains("not set"));

        let mut e = Editor::new(Buffer::from_text("one two\n"));
        e.feed_text("]c");
        assert!(
            !e.message.is_empty() || e.cursor != 0,
            "]c must jump or report, never no-op"
        );

        let mut e = Editor::new(Buffer::from_text("one two\n"));
        e.feed_text("gd");
        assert!(!e.message.is_empty(), "gd with no LSP must say so");

        let mut e = Editor::new(Buffer::from_text("hello world\n"));
        e.feed_text("\"+yiw");
        assert_eq!(e.register(Some('+')).0, "hello");
        assert!(e.osc52.is_some(), "clipboard yank stages OSC52");
    }
}
