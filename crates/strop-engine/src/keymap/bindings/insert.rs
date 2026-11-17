//! Insert-mode keys, including completion.
use super::super::{completion, Binding, Handler};

pub(super) const ROWS: &[Binding] = &[
    completion::REQUEST,
    completion::LANGUAGE,
    completion::WORDS,
    completion::ARROWS,
    completion::ACCEPT,
    completion::DISMISS,
    completion::CYCLE,
    Binding {
        keys: "esc",
        desc: "dismiss completion and enter Normal mode in one event (one undo unit)",
        sections: &["insert"],
        live: true,
        id: "insert-esc",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "backspace",
        desc: "delete back",
        sections: &["insert"],
        live: true,
        id: "insert-bs",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "enter",
        desc: "new line (auto-indent); accept completion only after deliberate selection",
        sections: &["insert"],
        live: true,
        id: "insert-enter",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "} ] )",
        desc: "closer on indent-only line aligns to its opener",
        sections: &["insert"],
        live: true,
        id: "insert-closers",
        handler: Handler::Prefix,
    },
];
