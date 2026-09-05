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
    assert_eq!(git.len(), 9); // l h b y o u s S p
    assert!(git.iter().any(|h| h.key == "u" && h.desc.contains("undo")));
    assert_eq!(children_of("m", Mode::Normal)[0].key, "<a>");
    assert_eq!(children_of("g", Mode::Normal).len(), 13); // gg gd gs ge gE gv gi g; g, gr gI gy gD
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
    "space g S",
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
    "*",
    "#",
    ";",
    ",",
    "^",
    "~",
    "S",
    "I",
    ":view",
    "space |",
    // prefixes with their own which-key cards
    "gg",
    "gd",
    "gs",
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
    "left",
    "right",
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
        // the machine's granularity (0016): per-key tokens; named
        // keys and ex commands stay whole; placeholders materialize
        let concretized = canonical.replace("<a>", "a").replace("<c>", "x");
        let mut toks: Vec<String> = Vec::new();
        for tok in concretized.split(' ') {
            if NAMED.contains(&tok) || tok.starts_with(':') {
                toks.push(tok.to_string());
            } else {
                toks.extend(tok.chars().map(|c| c.to_string()));
            }
        }
        assert!(
            find_row(&toks).is_some(),
            "{entry} dispatches but has no BINDINGS row (0003 §5.7)"
        );
    }
}

/// 0008 stage 2 structural pin: every live normal/leader row
/// dispatches observably through the walker — the table IS dispatch.
#[test]
fn live_rows_dispatch_through_the_table() {
    for b in BINDINGS.iter().filter(|b| b.live) {
        for seq in expand(b.keys) {
            let keys = seq
                .iter()
                .map(|t| match *t {
                    "space" => " ".to_string(),
                    "<a>" => "a".into(),
                    "<c>" => "x".into(),
                    t if t.starts_with("ctrl-") || t == "up" || t == "down" || t == "tab" => {
                        String::new() // key events, not walker chars
                    }
                    t if t.contains('<') => {
                        // "f<c>": the key, then a concrete char
                        let i = t.find('<').unwrap();
                        format!("{}x", &t[..i])
                    }
                    t => t.to_string(),
                })
                .collect::<String>();
            if keys.is_empty() || keys.starts_with(':') || keys.starts_with('-') {
                continue; // event-layer, ex-line, and CLI-flag rows
            }
            if matches!(b.handler, Handler::Soon) {
                continue; // surface-only verbs (]f/[f) dispatch in the
                          // readonly layer, not on plain buffers
            }
            // legal no-ops (motions at edges) say nothing; what must
            // never happen is the unknown-key marker
            let mut e = Editor::new(Buffer::from_text("fn f(x) {\n    let y = f(x);\n}\n"));
            e.set_head(14);
            e.feed_text(&keys);
            assert!(
                !e.message.starts_with("not an editor command"),
                "{} (fed as {keys:?}) failed to dispatch — table drift",
                b.keys
            );
        }
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
            if !dispatched_something(&e) {
                panic!(
                    "{} (fed as {keys:?}) no-op: msg={:?} pending={:?} prefix={:?}",
                    b.keys,
                    e.message,
                    e.pending,
                    e.walker.prefix_display()
                );
            }
        }
    }
}

fn dispatched_something(e: &Editor) -> bool {
    !e.message.is_empty()
        || !e.pending.is_empty()
        || e.picker_open()
        || !e.walker.prefix_display().is_empty()
        || e.clip_paste_pending.is_some()
        || e.osc52.is_some()
        || e.mode != Mode::Normal
        || e.docs.len() != 1
        || e.head() != 0
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
        !e.message.is_empty() || e.head() != 0,
        "]c must jump or report, never no-op"
    );

    let mut e = Editor::new(Buffer::from_text("one two\n"));
    e.feed_text("gd");
    assert!(!e.message.is_empty(), "gd with no LSP must say so");

    let mut e = Editor::new(Buffer::from_text("int x;\n"));
    e.feed_text("gs");
    assert!(!e.message.is_empty(), "gs with no LSP must say so");

    let mut e = Editor::new(Buffer::from_text("hello world\n"));
    e.feed_text("\"+yiw");
    assert_eq!(e.register(Some('+')).0, "hello");
    assert!(e.osc52.is_some(), "clipboard yank stages OSC52");
}
#[test]
fn compat_report_is_fresh() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/vim-compat.md");
    let generated = super::compat_report();
    if std::env::var_os("STROP_REGEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).unwrap();
    }
    let checked_in = std::fs::read_to_string(&path)
        .expect("docs/vim-compat.md missing — STROP_REGEN=1 cargo test");
    assert_eq!(
        checked_in, generated,
        "docs/vim-compat.md is stale — STROP_REGEN=1 cargo test to regen"
    );
}
