//! Insert completion bindings and compact card hints share these same rows.
use super::{Binding, Handler};

pub const REQUEST: Binding = Binding {
    keys: "ctrl-space",
    desc: "code completion: language service + current source words",
    sections: &["insert"],
    live: true,
    id: "completion-request",
    handler: Handler::Contextual,
};
pub const LANGUAGE: Binding = Binding {
    keys: "ctrl-x ctrl-o",
    desc: "language-service completion",
    sections: &["insert"],
    live: true,
    id: "completion-language",
    handler: Handler::Contextual,
};
pub const WORDS: Binding = Binding {
    keys: "ctrl-n ctrl-p",
    desc: "source words; choose next/previous completion",
    sections: &["insert"],
    live: true,
    id: "completion-words",
    handler: Handler::Contextual,
};
pub const ARROWS: Binding = Binding {
    keys: "up down",
    desc: "choose completion when open; otherwise move the caret",
    sections: &["insert"],
    live: true,
    id: "completion-arrows",
    handler: Handler::Contextual,
};
pub const ACCEPT: Binding = Binding {
    keys: "ctrl-y",
    desc: "accept choice",
    sections: &["insert"],
    live: true,
    id: "completion-accept",
    handler: Handler::Contextual,
};
pub const DISMISS: Binding = Binding {
    keys: "ctrl-e",
    desc: "dismiss",
    sections: &["insert"],
    live: true,
    id: "completion-dismiss",
    handler: Handler::Contextual,
};
pub const CYCLE: Binding = Binding {
    keys: "tab s-tab",
    desc: "cycle candidates with a live preview",
    sections: &["insert"],
    live: true,
    id: "insert-tab",
    handler: Handler::Contextual,
};
