//! The command line (plan 0110): how it opens, what a range is, and one
//! row per key action of `editor/cmdline/keys.rs` (a test pins both
//! directions). The commands themselves are not listed here — `?`
//! writes one row per command from the registry (`editor/ex/table`).
use super::super::{Binding, Handler};

const fn row(keys: &'static str, desc: &'static str, id: &'static str) -> Binding {
    Binding {
        keys,
        desc,
        sections: &["cmdline"],
        live: true,
        id,
        handler: Handler::TextLine,
    }
}

/// A key of the open line: the line's reducer dispatches it, not the
/// Normal-mode trie.
const fn key(keys: &'static str, desc: &'static str, id: &'static str) -> Binding {
    Binding {
        keys,
        desc,
        sections: &["cmdline"],
        live: true,
        id,
        handler: Handler::Contextual,
    }
}

pub(super) const ROWS: &[Binding] = &[
    row(
        ":",
        "the command line (from Visual it opens with '<,'>)",
        "cmdline-open",
    ),
    row(
        ":N :N,M :. :$ :% :'a :'<,'> :/pat/ :?pat? :.+N :N;+M",
        "ranges: lines, marks, searches and offsets; :N alone goes to the line",
        "cmdline-ranges",
    ),
    key(
        "enter ctrl-j ctrl-m",
        "run the line (a failed line says so in error ink)",
        "cmdline-run",
    ),
    key(
        "esc ctrl-[ ctrl-c",
        "cancel; the view returns to where it was",
        "cmdline-cancel",
    ),
    key(
        "backspace ctrl-h",
        "delete before the cursor; on an empty line, cancel",
        "cmdline-backspace",
    ),
    key(
        "delete",
        "delete under the cursor (at the end: before it)",
        "cmdline-delete",
    ),
    key(
        "ctrl-w",
        "delete the word before the cursor",
        "cmdline-delete-word",
    ),
    key(
        "ctrl-u",
        "delete everything before the cursor",
        "cmdline-delete-to-start",
    ),
    key("left", "one character left", "cmdline-left"),
    key("right", "one character right", "cmdline-right"),
    key("ctrl-b home", "start of the line", "cmdline-home"),
    key(
        "ctrl-e end",
        "end of the line; with a completion selected: put back what was typed",
        "cmdline-end",
    ),
    key(
        "ctrl-r",
        "insert a register (\" a-z 0 : / +); ctrl-r ctrl-w / ctrl-a: the word / WORD under the cursor",
        "cmdline-register",
    ),
    key(
        "tab",
        "next completion, written into the line (after the last: what was typed)",
        "cmdline-complete",
    ),
    key("s-tab", "previous completion", "cmdline-complete-back"),
    key(
        "ctrl-p",
        "older history line; previous completion while the menu is open",
        "cmdline-older",
    ),
    key(
        "ctrl-n",
        "newer history line; next completion while the menu is open",
        "cmdline-newer",
    ),
    key(
        "up",
        "older history line that starts with what is typed; the row above once one is selected",
        "cmdline-older-prefix",
    ),
    key(
        "down",
        "newer history line that starts with what is typed; the row below once one is selected",
        "cmdline-newer-prefix",
    ),
    key(
        "ctrl-y",
        "keep the selected completion, close the menu",
        "cmdline-accept",
    ),
    key("ctrl-d", "list the completions", "cmdline-list"),
    key(
        "ctrl-f",
        "edit this line in the command-line window",
        "cmdline-window",
    ),
    key("ctrl-l", "repaint the screen", "cmdline-repaint"),
    key(
        "q: q/ q?",
        "the command-line window: the : (or / ?) history as a buffer, the line being typed last",
        "cmdline-window-open",
    ),
    key(
        "enter",
        "in the command-line window: run the cursor's line (Insert too)",
        "cmdline-window-run",
    ),
    key(
        "ctrl-c :q",
        "in the command-line window: close it",
        "cmdline-window-close",
    ),
];
