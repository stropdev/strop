//! Terminal buffers (0090 §4, Neovim's model): open, escape, re-enter,
//! close. `space t` / `space T` live in the leader table and list here too.
use super::super::{Binding, Handler};

pub(super) const ROWS: &[Binding] = &[
    Binding {
        keys: ":terminal [cmd] :term",
        desc: "new shell (or cmd) in this pane; space t opens one in a split below",
        sections: &["terminal"],
        live: true,
        id: "terminal-open",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "ctrl-\\ ctrl-n",
        desc: "TERMINAL → NORMAL: a live buffer that follows output while the cursor is on the last line; k stops, G resumes",
        sections: &["terminal"],
        live: true,
        id: "terminal-normal",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "i I a A",
        desc: "NORMAL → TERMINAL at the child's cursor; every key goes to the child, Esc and ctrl-w included",
        sections: &["terminal"],
        live: true,
        id: "terminal-input",
        handler: Handler::Contextual,
    },
    Binding {
        keys: ":q :bd!",
        desc: ":q hides a running terminal (space b returns), :bd! stops it; :qa refuses while terminals run, :qa! stops them",
        sections: &["terminal"],
        live: true,
        id: "terminal-close",
        handler: Handler::Contextual,
    },
    Binding {
        keys: ":r !cmd",
        desc: "insert a command's output below the cursor",
        sections: &["terminal"],
        live: true,
        id: "read-command",
        handler: Handler::Contextual,
    },
];
