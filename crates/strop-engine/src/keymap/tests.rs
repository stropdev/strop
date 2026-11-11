use super::lookup::{expand, NAMED};
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
            BINDINGS.iter().any(|b| b.sections.contains(&section.id)),
            "section {} is empty",
            section.id
        );
    }
    for b in BINDINGS {
        assert!(!b.keys.is_empty() && !b.desc.is_empty());
        for section in b.sections {
            assert!(
                SECTIONS.iter().any(|known| known.id == *section),
                "{}: unknown section {section:?}",
                b.keys
            );
        }
        assert!(
            !b.desc.contains("(soon)"),
            "{}: '(soon)' belongs to live:false, not the desc",
            b.keys
        );
    }
}

/// 0090 §4 / 0003 §5.7: the terminal section opens with how to open a
/// terminal and names the one escape chord and the re-entry keys, all
/// live in `?` — the way in and out can never be undiscoverable.
#[test]
fn terminal_section_opens_with_space_t_and_names_the_one_escape() {
    let terminal: Vec<&Binding> = BINDINGS
        .iter()
        .filter(|b| b.sections.contains(&"terminal"))
        .collect();
    assert_eq!(terminal.first().map(|b| b.keys), Some("space t"));
    for keys in ["space t", "space T", "ctrl-\\ ctrl-n", "i I a A"] {
        assert!(
            terminal.iter().any(|b| b.live && b.keys == keys),
            "terminal row {keys:?} has no live row in the terminal section"
        );
    }
    assert!(terminal
        .iter()
        .all(|b| !b.keys.contains("ctrl-4") && !b.keys.contains("f2")));
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
    for p in [" ", " w", " g", "g", "[", "]", "m", "'", "`"] {
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
        space.iter().any(|h| h.key == "j" && h.live),
        "space j is the live jumplist picker (0047)"
    );
    let git = children_of(" g", Mode::Normal);
    assert_eq!(git.len(), 9); // l h b y o u s S p
    assert!(git.iter().any(|h| h.key == "u" && h.desc.contains("undo")));
    assert_eq!(children_of("m", Mode::Normal)[0].key, "<a>");
    // + gb gB g␣, the case operators (gU gu g~ g?), the g motions
    // (g_ g0 g^ g$ gj gk gn gN), gp gP, gJ, g* g#, g&
    assert_eq!(children_of("g", Mode::Normal).len(), 34);
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
/// The trie already makes leaves == BINDINGS rows for everything it
/// dispatches; this list guards the surfaces still dispatched by hand
/// (ex commands, pickers, panes, visual absorbers) until they move onto
/// the trie, at which point the list goes.
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
    "space j",
    "space s",
    "g<space>",
    "gb",
    "gB",
    "]f",
    "[f",
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
    // panes
    "space w h",
    "space w l",
    "space w j",
    "space w k",
    "space w w",
    "space w 1",
    "space w 2",
    "space w 3",
    "space w 4",
    "space w 5",
    "space w 6",
    "space w 7",
    "space w 8",
    "space w 9",
    "space w v",
    "space w -",
    "space w q",
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

/// 0003 §5.7, reverse direction: every dispatchable sequence has a
/// BINDINGS row. A new match arm without a row fails here.
#[test]
fn every_dispatched_sequence_has_a_row() {
    let mut missing = Vec::new();
    for entry in DISPATCHED {
        // the machine's granularity (0016): per-key tokens; named
        // keys stay whole; placeholders materialize
        let concretized = entry.replace("<a>", "a").replace("<c>", "x");
        let mut toks: Vec<String> = Vec::new();
        for tok in concretized.split(' ') {
            if NAMED.contains(&tok) {
                toks.push(tok.to_string());
            } else if let Some(pos) = tok.find('<') {
                // g<space> style: leading chars, then the named token in
                // the walker's vocabulary
                toks.extend(tok[..pos].chars().map(|c| c.to_string()));
                let named = &tok[pos..];
                toks.push(if named == "<space>" {
                    "space".to_string()
                } else {
                    named.to_string()
                });
            } else {
                toks.extend(tok.chars().map(|c| c.to_string()));
            }
        }
        let covered = find_row(&toks).is_some()
            || find_visual_row(&toks).is_some()
            || BINDINGS.iter().any(|row| {
                matches!(row.handler, Handler::Contextual)
                    && expand(row.keys)
                        .iter()
                        .any(|keys| keys.iter().copied().eq(toks.iter().map(String::as_str)))
            });
        if !covered {
            missing.push(*entry);
        }
    }
    assert!(
        missing.is_empty(),
        "dispatchable sequences missing BINDINGS rows (0003 §5.7): {missing:?}"
    );
}

/// 0003 §5.7 for the `:` line, read off the one table dispatch uses
/// (0095 §2.6): every ex command is named in `?` — by its full name or
/// any abbreviation Vim accepts for it. A new table row without a
/// keymap row fails here; no hand-kept list stands between them.
#[test]
fn every_ex_command_has_a_row() {
    let named: std::collections::HashSet<&str> = BINDINGS
        .iter()
        .flat_map(|row| row.keys.split(' '))
        .filter_map(|token| token.strip_prefix(':'))
        .filter_map(crate::editor::ex::split_command)
        .map(|(command, _, _)| command.name)
        .collect();
    let missing: Vec<&str> = crate::editor::ex::table::COMMANDS
        .iter()
        .map(|command| command.name)
        .filter(|name| !named.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "ex commands missing BINDINGS rows (0003 §5.7): {missing:?}"
    );
}

/// 0008 stage 2 structural pin: every live normal/leader row
/// dispatches observably through the walker — the table IS dispatch.
#[test]
fn live_rows_dispatch_through_the_table() {
    for b in BINDINGS.iter().filter(|b| b.live) {
        for seq in expand(b.keys) {
            if seq.len() > 1 && seq[0].starts_with("ctrl-") {
                // a control PREFIX is a key event: feeding its second
                // key alone would test a different command (ctrl-w
                // rows are pinned in editor/panes.rs)
                continue;
            }
            let keys = seq
                .iter()
                .map(|t| match *t {
                    "space" => " ".to_string(),
                    "<a>" => "a".into(),
                    "<c>" => "x".into(),
                    t if t.starts_with("ctrl-") || t == "up" || t == "down" || t == "tab" => {
                        String::new() // key events, not walker chars
                    }
                    t if t.contains("<space>") => {
                        // "g<space>": the key, then a literal space
                        let i = t.find('<').unwrap();
                        format!("{} ", &t[..i])
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
            if matches!(b.handler, Handler::Soon | Handler::Contextual) {
                continue; // surface-only verbs (]f/[f) dispatch in the
                          // readonly layer, not on plain buffers
            }
            // legal no-ops (motions at edges) say nothing; what must
            // never happen is the unknown-key marker
            let mut e = Editor::new(Buffer::from_text("fn f(x) {\n    let y = f(x);\n}\n"));
            e.set_head(14);
            if matches!(b.handler, Handler::Visual(_)) {
                // Visual's own rows dispatch on a selection
                e.feed_text("v");
            }
            e.feed_text(&keys);
            assert!(
                !e.message.starts_with("not an editor command"),
                "{} (fed as {keys:?}) failed to dispatch — table drift",
                b.keys
            );
        }
    }
}

#[test]
fn uppercase_x_deletes_backward_instead_of_repeating_lowercase_x() {
    let mut backward = Editor::new(Buffer::from_text("abc\n"));
    backward.set_head(1);
    backward.feed_text("X");
    assert_eq!(backward.buf().text().to_string(), "bc\n");
    assert_eq!(backward.head(), 0);
    assert_eq!(backward.register(None).text, "a");

    let mut forward = Editor::new(Buffer::from_text("abc\n"));
    forward.set_head(1);
    forward.feed_text("x");
    assert_eq!(forward.buf().text().to_string(), "ac\n");
    assert_eq!(forward.head(), 1);
    assert_eq!(forward.register(None).text, "b");
}

/// The keys of a table-notation sequence (`ctrl-w x`, `gqip`) as events.
fn notation_keys(sequence: &str) -> Vec<strop_core::frontend_input::Key> {
    use strop_core::frontend_input::Key;
    let mut keys = Vec::new();
    for token in sequence.split(' ') {
        match token {
            "ctrl-w" => keys.push(Key::CtrlW),
            "ctrl-v" => keys.push(Key::CtrlV),
            "ctrl-a" => keys.push(Key::CtrlA),
            // a Ctrl chord strop has no key for never reaches the editor
            "ctrl-k" => {}
            text => keys.extend(text.chars().map(Key::Char)),
        }
    }
    keys
}

/// 0095 §2.7: a Missing row does not dispatch. Feeding it changes
/// neither the text nor the cursor's line nor the mode, opens nothing,
/// and leaves no command half-typed. A row that starts to work fails
/// here until it moves into the binding table.
#[test]
fn missing_rows_do_not_dispatch() {
    use super::missing::{Where, MISSING};
    use strop_core::frontend_input::Key;
    let text = "fn f(x) {\n    let y = f(x);\n\n    one. two.\n}\n";
    for row in MISSING {
        for sequence in row.keys {
            let mut e = Editor::new(Buffer::from_text(text));
            e.set_head(14);
            if row.place == Where::Ex {
                e.feed_text(&format!("{sequence}\r"));
                assert!(
                    e.message.starts_with("not an editor command") || e.message.contains("unknown"),
                    "{sequence}: {:?}",
                    e.message
                );
            } else {
                let (enter, mode) = match row.place {
                    Where::Visual => ("v", Mode::Visual),
                    Where::Insert => ("i", Mode::Insert),
                    _ => ("", Mode::Normal),
                };
                e.feed_text(enter);
                for key in notation_keys(sequence) {
                    e.feed(key);
                }
                assert_eq!(e.mode, mode, "{sequence} changed the mode");
                assert!(e.walker.is_ground(), "{sequence} left a command pending");
                assert!(!e.pending.is_active(), "{sequence} opened a prompt");
                e.feed(Key::Esc);
            }
            assert_eq!(
                e.buf().text().to_string(),
                text,
                "{sequence} changed the text"
            );
            assert_eq!(e.buf().line_of(e.head()), 1, "{sequence} moved the cursor");
            assert!(e.recording.is_none(), "{sequence} started a recording");
        }
    }
}

/// A Reassigned row names a key strop does bind.
#[test]
fn reassigned_rows_are_bound() {
    use super::missing::{Where, REASSIGNED};
    for row in REASSIGNED.iter().filter(|row| row.place != Where::Insert) {
        for keys in row.keys.split(' ') {
            let path: Vec<String> = if NAMED.contains(&keys) {
                vec![keys.to_owned()]
            } else {
                keys.chars().map(|c| c.to_string()).collect()
            };
            let bound = match row.place {
                Where::Visual => find_visual_row(&path).is_some() || any_visual_child(&path),
                // `ds` / `cs` / `ys` are the grammar's, under an operator
                _ => find_row(&path).is_some() || any_child(&path) || keys.ends_with('s'),
            };
            assert!(bound, "{keys} is listed as reassigned but is not bound");
        }
    }
}

/// `docs/vim-compat.md` is `strop compat`'s output, checked in.
#[test]
fn the_compatibility_page_is_the_generated_report() {
    assert_eq!(
        include_str!("../../../../docs/vim-compat.md"),
        compat_report(),
        "regenerate with: strop compat > docs/vim-compat.md"
    );
}
