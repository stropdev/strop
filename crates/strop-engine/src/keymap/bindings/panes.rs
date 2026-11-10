//! Splits, pane focus and the jumplist.
use super::super::{Binding, Handler};

pub(super) const ROWS: &[Binding] = &[
    Binding {
        keys: ":vs :sp",
        desc: "split vertical / horizontal",
        sections: &["panes"],
        live: true,
        id: "ex-split",
        handler: Handler::TextLine,
    },
    Binding {
        keys: "space w h / l / j / k / w / 1 / 2 / 3 / 4 / 5 / 6 / 7 / 8 / 9",
        desc: "pane move / cycle / focus 1-9",
        sections: &["panes"],
        live: true,
        id: "pane-nav",
        handler: Handler::Leaf(crate::editor::Editor::pane_move_pub),
    },
    Binding {
        keys: "ctrl-o / ctrl-i (tab)",
        desc: "jump back / forward (jumplist)",
        sections: &["panes"],
        live: true,
        id: "jumplist",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::jump_back(e)),
    },
    Binding {
        keys: "space w v / -",
        desc: "pane split (vertical / horizontal)",
        sections: &["panes"],
        live: true,
        id: "pane-split",
        handler: Handler::Leaf(crate::editor::Editor::split_pub),
    },
    Binding {
        keys: "space w q",
        desc: "close pane (last → buffer)",
        sections: &["panes"],
        live: true,
        id: "pane-close",
        handler: Handler::Leaf(|e, _| crate::editor::Editor::pane_close_pub(e)),
    },
    Binding {
        keys: "ctrl-l",
        desc: "redraw: full repaint when the terminal desyncs",
        sections: &["panes"],
        live: true,
        id: "redraw",
        handler: Handler::Leaf(|e, _| e.needs_repaint = true),
    },
];
