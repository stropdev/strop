//! Tab pages against Neovim (plans/0113 §8.5): the window tree of every
//! tab page (`winlayout(N)` with window numbers in their own tab page) and
//! the current tab page must agree, with the windows and text as ever.
//!
//! Every case ends in a tab page showing the fixture (the harness ends
//! with `:w`), so tab pages are made with `:tab split` and `ctrl-w T`.
//! Most cases start from `SHAPED`: three tab pages whose trees differ, so
//! an order is visible.
//!
//! Not in the corpus: a bare `:tabnew` (no file name to write), and a step
//! that errors anywhere but last — Neovim's `feedkeys` drops the rest of
//! the keys after an error (`:tabc` on the last tab page, `:tabm -1` at the
//! first, `<c-w>T` with one window), so the oracle would answer for a
//! shorter sequence. The generated family can hit that before its last
//! step only through `<c-w>T` on a one-pane tab page; the generator drops
//! those sequences. The error rows are unit tests (`editor/tabs/tests.rs`,
//! `editor/ex/tests/tabs.rs`).

use super::Run;

const TEXT: &str = "alpha\nbeta\ngamma\n";

/// Three tab pages — side by side, stacked, alone — the third current.
const SHAPED: &str = ":vs<cr>:tab split<cr>:sp<cr>:tab split<cr>";

#[rustfmt::skip]
const CASES: &[(&str, &str)] = &[
    ("tab split", ":tab split<cr>"),
    ("tab split keeps the cursor", "jl:vs<cr>:tab split<cr>"),
    ("ctrl-w T from two panes", ":vs<cr><c-w>T"),
    ("ctrl-w T from one", "<c-w>T"),
    ("ctrl-w T from the first of three", ":vs<cr>:tab split<cr>:tab split<cr>1gt<c-w>T"),
    ("gt wraps", "SHAPEDgt"),
    ("gT wraps", "SHAPED1gtgT"),
    ("2gt", "SHAPED2gt"),
    ("2gT from the first", "SHAPED1gt2gT"),
    ("g tab", "SHAPED1gtg<tab>"),
    ("g tab twice", "SHAPED1gtg<tab>g<tab>"),
    ("g tab after the previous closed", "SHAPED1gt:tabclose 3<cr>g<tab>"),
    ("tabclose in the middle", "SHAPED2gt:tabclose<cr>"),
    ("tabclose at the end", "SHAPED:tabclose<cr>"),
    ("tabclose at the start", "SHAPED1gt:tabclose<cr>"),
    ("tabclose 1 from the third", "SHAPED:tabclose 1<cr>"),
    ("q on a tab page's only pane", "SHAPED:q<cr>"),
    ("ctrl-w c on a tab page's only pane", "SHAPED<c-w>c"),
    ("tabonly keeps its splits", "SHAPED2gt:tabonly<cr>"),
    ("tabmove 0", "SHAPED:tabmove 0<cr>"),
    ("tabmove", "SHAPED1gt:tabmove<cr>"),
    ("tabmove +1", "SHAPED1gt:tabmove +1<cr>"),
    ("tabmove -1", "SHAPED:tabmove -1<cr>"),
    ("tabmove 1 from the third", "SHAPED:tabmove 1<cr>"),
    ("tabnext 2", "SHAPED:tabnext 2<cr>"),
    ("tabprevious", "SHAPED1gt:tabprevious<cr>"),
    ("tablast and tabfirst", "SHAPED:tabfirst<cr>:tablast<cr>2gt"),
    ("only inside a tab page", ":vs<cr>:tab split<cr>:vs<cr>:only<cr>"),
    ("an edit seen from the other tab page", ":tab split<cr>dd1gt"),
];

/// Neovim's answer to one generated step, or `None` where it errors (the
/// rest of the keys would be dropped). `tabs` holds each tab page's panes.
fn step(tabs: &mut Vec<usize>, current: &mut usize, key: &str) -> Option<()> {
    let count = tabs.len();
    match key {
        "gt" => *current = (*current + 1) % count,
        "gT" => *current = (*current + count - 1) % count,
        ":tabc<cr>" => {
            if count == 1 {
                return None;
            }
            tabs.remove(*current);
            *current = (*current).min(count - 2);
        }
        "<c-w>T" => {
            if tabs[*current] == 1 {
                return None;
            }
            tabs[*current] -= 1;
            tabs.insert(*current + 1, 1);
            *current += 1;
        }
        _ => tabs[*current] += 1,
    }
    Some(())
}

pub fn cases() -> Vec<Run> {
    let mut runs: Vec<Run> = CASES
        .iter()
        .map(|(name, keys)| Run {
            name: format!("tab/{name}"),
            text: TEXT.to_owned(),
            keys: keys.replace("SHAPED", SHAPED),
        })
        .collect();
    // every three steps after two `:tab split`s from a split
    let keys = ["gt", "gT", ":tabc<cr>", "<c-w>T", "<c-w>v"];
    for a in keys {
        for b in keys {
            for c in keys {
                let (mut tabs, mut current) = (vec![2, 1, 1], 2);
                let clean = step(&mut tabs, &mut current, a)
                    .and_then(|()| step(&mut tabs, &mut current, b))
                    .is_some();
                if !clean {
                    continue;
                }
                runs.push(Run {
                    name: format!("tab/{a}{b}{c}"),
                    text: TEXT.to_owned(),
                    keys: format!(":vs<cr>:tab split<cr>:tab split<cr>{a}{b}{c}"),
                });
            }
        }
    }
    runs
}
