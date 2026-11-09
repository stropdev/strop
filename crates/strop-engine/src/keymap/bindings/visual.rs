//! Visual-mode entry and verbs.
use super::super::{AbsorbKind, Binding, Handler};

pub(super) const ROWS: &[Binding] = &[
    Binding {
        keys: "gv",
        desc: "reselect last visual",
        // pressed from Normal: visible on the Normal `g` card too
        sections: &["visual", "editing"],
        live: true,
        id: "reselect-visual",
        handler: Handler::Leaf(|e, _| e.reselect_visual()),
    },
    Binding {
        keys: "ctrl-v",
        desc: "visual block",
        sections: &["visual"],
        live: true,
        id: "visual-block",
        handler: Handler::Leaf(|e, _| e.enter_block_pub()),
    },
    Binding {
        keys: "v V",
        desc: "visual / visual-line",
        sections: &["visual"],
        live: true,
        id: "visual-enter",
        handler: Handler::Leaf(|e, k| e.enter_visual_pub(k)),
    },
    Binding {
        keys: "d y c x > <",
        desc: "operate on selection",
        sections: &["visual"],
        live: true,
        id: "visual-ops",
        handler: Handler::Operator,
    },
    Binding {
        keys: "ctrl-c",
        desc: "toggle comments on selected lines",
        sections: &["visual"],
        live: true,
        id: "visual-comment",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "S<c>",
        desc: "wrap selection in pair",
        sections: &["visual"],
        live: true,
        id: "visual-surround",
        handler: Handler::AbsorbChar(AbsorbKind::Replace),
    },
    Binding {
        keys: "i<a> a<a>",
        desc: "objects select (vi[ works)",
        sections: &["visual"],
        live: true,
        id: "visual-objects",
        handler: Handler::ObjectPrefix,
    },
    Binding {
        keys: "space y",
        desc: "yank selection → clipboard",
        sections: &["visual"],
        live: true,
        id: "clip-yank",
        handler: Handler::Leaf(|e, _| e.clipboard_yank_pub()),
    },
];
