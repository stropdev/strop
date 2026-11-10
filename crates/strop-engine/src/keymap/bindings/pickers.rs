//! Keys inside the picker and Search workspace.
use super::super::{Binding, Handler};

pub(super) const ROWS: &[Binding] = &[
    Binding {
        keys: "ctrl-d ctrl-u ctrl-f ctrl-b",
        desc: "half/full page scroll (count = lines)",
        sections: &["pickers"],
        live: true,
        id: "scroll-pages",
        handler: Handler::TextLine,
    },
    Binding {
        keys: "up down left right tab s-tab",
        desc: "picker navigation / arrows = hjkl everywhere",
        sections: &["pickers"],
        live: true,
        id: "picker-nav",
        handler: Handler::Prefix,
    },
    Binding {
        keys: "ctrl-x",
        desc: "Search: exclude/include match",
        sections: &["pickers"],
        live: true,
        id: "search-exclude",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "ctrl-space",
        desc: "query field: qualifier, language and value suggestions",
        sections: &["pickers"],
        live: true,
        id: "query-suggestions",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "ctrl-d",
        desc: "Search: exclude/include file",
        sections: &["pickers"],
        live: true,
        id: "search-file-exclude",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "ctrl-r",
        desc: "Search: toggle replacement; document: redo",
        sections: &["pickers"],
        live: true,
        id: "search-replacement",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "enter",
        desc: "Search Find: open source; With: prepare review",
        sections: &["pickers"],
        live: true,
        id: "search-accept",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "ctrl-o",
        desc: "Search: collect included matches",
        sections: &["pickers"],
        live: true,
        id: "search-collect",
        handler: Handler::Contextual,
    },
];
