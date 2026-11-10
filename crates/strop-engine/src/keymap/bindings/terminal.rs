//! Terminal panes: input, inspection, lifecycle.
use super::super::{Binding, Handler};

pub(super) const ROWS: &[Binding] = &[
    Binding {
        keys: ":terminal :terminal-local :terminal-stop",
        desc: "launch a local PTY, explicitly choose local context, or stop and retain output",
        sections: &["terminal"],
        live: true,
        id: "terminal-lifecycle",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "ctrl-\\ ctrl-n",
        desc: "terminal input → editor Normal inspection; Esc stays with the child",
        sections: &["terminal"],
        live: true,
        id: "terminal-normal",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "f2",
        desc: "terminal input → editor Normal inspection (single-key, Windows-safe)",
        sections: &["terminal"],
        live: true,
        id: "terminal-window",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "i I a A",
        desc: "terminal Normal inspection → live child input (never relaunches an exited session)",
        sections: &["terminal"],
        live: true,
        id: "terminal-input",
        handler: Handler::Contextual,
    },
    Binding {
        keys: ":terminal-paste :terminal-paste-cancel",
        desc: "confirm or discard the scoped held multiline/control paste",
        sections: &["terminal"],
        live: true,
        id: "terminal-paste-decision",
        handler: Handler::Contextual,
    },
    Binding {
        keys: ":terminal-refresh",
        desc: "reinstall the latest output into the pinned inspection snapshot (never auto-refreshes a selection)",
        sections: &["terminal"],
        live: true,
        id: "terminal-refresh",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "ctrl-4 ctrl-n",
        desc: "legacy Ctrl-\\ Ctrl-N spelling from the outer terminal decoder",
        sections: &["terminal"],
        live: true,
        id: "terminal-normal-legacy",
        handler: Handler::Contextual,
    },
];
