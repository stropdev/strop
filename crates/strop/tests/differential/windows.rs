//! Window layout against Neovim (plans/0104 §8.4): the window tree
//! (`winlayout()` with window numbers) and the current window must agree,
//! with `splitbelow splitright` set on the Neovim side (strop's posture).
//! Sizes are not compared: strop's equal split can differ by the odd cell
//! and by vim's `winwidth` (0104 §5.4).
//!
//! Not in the corpus: `<c-w>n` and `:new` (the harness ends with `:w`, and
//! the new buffer has no name); `<c-w>p` after a close (0104 §12); anything
//! where the cursor column decides `j`/`k` — headless Neovim does not
//! redraw between fed keys, so its answer is for column 0. Directional
//! cases keep the cursor in column 0; the column rule is a unit test
//! (`panes/tests/tree.rs`).

use super::Run;

const TEXT: &str = "alpha\nbeta\ngamma\n";

#[rustfmt::skip]
const CASES: &[(&str, &str)] = &[
    ("the owner's case", "<c-w>s<c-w>v"),
    ("sp then vs", ":sp<cr>:vs<cr>"),
    ("vs then sp", ":vs<cr>:sp<cr>"),
    ("same axis siblings", ":vs<cr>:vs<cr>"),
    ("split from the first", ":vs<cr>:vs<cr><c-w>h<c-w>h<c-w>s"),
    ("the grid", ":sp<cr><c-w>k:vs<cr><c-w>j:vs<cr>"),
    ("three over two", ":sp<cr>:vs<cr><c-w>k:vs<cr>:vs<cr>"),
    ("close focuses the previous", ":vs<cr>:vs<cr><c-w>c"),
    ("close the first focuses the next", ":vs<cr>:vs<cr><c-w>h<c-w>h<c-w>c"),
    ("close collapses a split", "<c-w>s<c-w>v<c-w>c"),
    ("close splices into the grandparent", ":vs<cr>:sp<cr><c-w>k:vs<cr><c-w>j<c-w>c"),
    ("only from a nest", ":sp<cr>:vs<cr>:only<cr>"),
    ("close on the last window", ":close<cr>"),
    ("w wraps", ":vs<cr>:sp<cr><c-w>w"),
    ("W wraps", ":vs<cr>:sp<cr><c-w>W<c-w>W<c-w>W"),
    ("no wrap at the left edge", ":vs<cr><c-w>h<c-w>h"),
    ("no wrap at the bottom edge", ":sp<cr><c-w>j"),
    ("an edit after moving", "<c-w>s<c-w>v<c-w>kdd"),
];

pub fn cases() -> Vec<Run> {
    let mut runs: Vec<Run> = CASES
        .iter()
        .map(|(name, keys)| Run {
            name: format!("win/{name}"),
            text: TEXT.to_owned(),
            keys: (*keys).to_owned(),
        })
        .collect();
    // every three window commands after the owner's case
    let keys = ['s', 'v', 'c', 'h', 'k'];
    for a in keys {
        for b in keys {
            for c in keys {
                runs.push(Run {
                    name: format!("win/sv{a}{b}{c}"),
                    text: TEXT.to_owned(),
                    keys: format!("<c-w>s<c-w>v<c-w>{a}<c-w>{b}<c-w>{c}"),
                });
            }
        }
    }
    runs
}
