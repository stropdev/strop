//! Keys in a folder (`strop .`, `:e dir`, `Space e`): the listing is a
//! buffer of names, so every vim edit works on it; these are the keys a
//! folder adds (0102).
use super::super::{Binding, Handler};

pub(super) const ROWS: &[Binding] = &[
    Binding {
        keys: "enter -",
        desc: "open the file or folder under the cursor / the parent folder",
        sections: &["folder"],
        live: true,
        id: "folder-open",
        handler: Handler::Contextual,
    },
    Binding {
        keys: "g.",
        desc: "show or hide dotfiles",
        sections: &["folder"],
        live: true,
        id: "folder-dotfiles",
        handler: Handler::Leaf(|e, _| e.toggle_directory_hidden()),
    },
    Binding {
        keys: "cw dd o yy p",
        desc: "rename / delete / new (name/ for a folder) / copy: edit the names, then :w",
        sections: &["folder"],
        live: true,
        id: "folder-edit",
        handler: Handler::Contextual,
    },
    Binding {
        keys: ":w :e!",
        desc: "show the changes (y applies, q cancels) / drop them and list the folder again",
        sections: &["folder"],
        live: true,
        id: "folder-write",
        handler: Handler::Contextual,
    },
    Binding {
        keys: ":filter :e",
        desc: "narrow to matching names (:filter alone clears) / list the folder again",
        sections: &["folder"],
        live: true,
        id: "folder-filter",
        handler: Handler::Contextual,
    },
];
