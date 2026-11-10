//! The filesystem rows of `Space a` as data: what each does and whether
//! it mutates. Which rows are valid for a view is decided in
//! `editor/actions.rs`, beside every other owner of the list.
pub(crate) struct Action {
    pub(crate) title: &'static str,
    pub(crate) command: &'static str,
    pub(crate) operand: bool,
    pub(crate) mutation: bool,
}
pub(crate) const ACTIONS: &[Action] = &[
    Action {
        title: "New file",
        command: "create",
        operand: true,
        mutation: true,
    },
    Action {
        title: "New directory",
        command: "mkdir",
        operand: true,
        mutation: true,
    },
    Action {
        title: "Rename",
        command: "rename",
        operand: true,
        mutation: true,
    },
    Action {
        title: "Move into directory",
        command: "move",
        operand: true,
        mutation: true,
    },
    Action {
        title: "Copy stored file",
        command: "copy stored",
        operand: true,
        mutation: true,
    },
    Action {
        title: "Copy current buffer contents",
        command: "copy buffer",
        operand: true,
        mutation: true,
    },
    Action {
        title: "Trash (recoverable)",
        command: "trash",
        operand: false,
        mutation: true,
    },
    Action {
        title: "Delete permanently",
        command: "remove",
        operand: false,
        mutation: true,
    },
    Action {
        title: "Toggle mark",
        command: "mark",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Clear marks",
        command: "unmark",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Refresh directory",
        command: "refresh",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Copy resource URI",
        command: "path",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Operations and receipts",
        command: "operations",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Verify operation outcome",
        command: "verify",
        operand: true,
        mutation: false,
    },
    Action {
        title: "Prepare checked recovery",
        command: "undo",
        operand: true,
        mutation: true,
    },
    Action {
        title: "Edit names with modal grammar",
        command: "edit",
        operand: false,
        mutation: true,
    },
    Action {
        title: "Review filename changes",
        command: ":w",
        operand: false,
        mutation: true,
    },
    Action {
        title: "Discard filename draft",
        command: "discard",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Draft copies: stored files",
        command: "copies stored",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Draft copies: current buffers",
        command: "copies buffer",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Draft deletions: Trash",
        command: "deletes trash",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Draft deletions: permanently remove",
        command: "deletes permanent",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Search here",
        command: "search",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Terminal here",
        command: "terminal",
        operand: false,
        mutation: false,
    },
    Action {
        title: "Open local terminal",
        command: ":terminal-local",
        operand: false,
        mutation: false,
    },
];
