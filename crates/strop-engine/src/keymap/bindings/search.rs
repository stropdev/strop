//! Buffer search.
use super::super::{Binding, Handler};

pub(super) const ROWS: &[Binding] = &[
    Binding {
        keys: "/ ?",
        desc: "literal search forward / backward (live as you type or delete)",
        sections: &["search"],
        live: true,
        id: "search",
        handler: Handler::TextLine,
    },
    Binding {
        keys: "n",
        desc: "next match",
        sections: &["search"],
        live: true,
        id: "search-next",
        handler: Handler::Leaf(|e, _| e.repeat_search_pub(false)),
    },
    Binding {
        keys: "N",
        desc: "previous match",
        sections: &["search"],
        live: true,
        id: "search-prev",
        handler: Handler::Leaf(|e, _| e.repeat_search_pub(true)),
    },
    Binding {
        keys: "* #",
        desc: "word under cursor, forward / backward",
        sections: &["search"],
        live: true,
        id: "word-search",
        handler: Handler::Leaf(|e, k| e.search_word_under_cursor(k == '#', true)),
    },
    Binding {
        keys: "g* g#",
        desc: "word under cursor, also inside longer words",
        sections: &["search"],
        live: true,
        id: "word-search-partial",
        handler: Handler::Leaf(|e, k| e.search_word_under_cursor(k == '#', false)),
    },
    Binding {
        keys: ":noh",
        desc: "clear the persistent search highlight",
        sections: &["search"],
        live: true,
        id: "ex-noh",
        handler: Handler::TextLine,
    },
];
