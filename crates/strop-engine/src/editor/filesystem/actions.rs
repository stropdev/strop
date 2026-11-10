//! The filesystem rows of `Space a` as data: what each does and whether
//! it mutates. Which rows are valid for a view is decided in
//! `editor/actions.rs`, beside every other owner of the list.
pub(crate) struct Action {
    pub(crate) title: &'static str,
    pub(crate) command: &'static str,
    pub(crate) operand: bool,
}
pub(crate) const ACTIONS: &[Action] = &[
    Action {
        title: "New file",
        command: "create",
        operand: true,
    },
    Action {
        title: "New directory",
        command: "mkdir",
        operand: true,
    },
    Action {
        title: "Rename",
        command: "rename",
        operand: true,
    },
    Action {
        title: "Move into directory",
        command: "move",
        operand: true,
    },
    Action {
        title: "Copy stored file",
        command: "copy stored",
        operand: true,
    },
    Action {
        title: "Copy current buffer contents",
        command: "copy buffer",
        operand: true,
    },
    Action {
        title: "Trash (recoverable)",
        command: "trash",
        operand: false,
    },
    Action {
        title: "Delete permanently",
        command: "remove",
        operand: false,
    },
    Action {
        title: "Toggle mark",
        command: "mark",
        operand: false,
    },
    Action {
        title: "Clear marks",
        command: "unmark",
        operand: false,
    },
    Action {
        title: "Refresh directory",
        command: "refresh",
        operand: false,
    },
    Action {
        title: "Copy resource URI",
        command: "path",
        operand: false,
    },
    Action {
        title: "Operations and receipts",
        command: "operations",
        operand: false,
    },
    Action {
        title: "Verify operation outcome",
        command: "verify",
        operand: true,
    },
    Action {
        title: "Prepare checked recovery",
        command: "undo",
        operand: true,
    },
    Action {
        title: "Edit names with modal grammar",
        command: "edit",
        operand: false,
    },
    Action {
        title: "Review filename changes",
        command: ":w",
        operand: false,
    },
    Action {
        title: "Discard filename draft",
        command: "discard",
        operand: false,
    },
    Action {
        title: "Draft copies: stored files",
        command: "copies stored",
        operand: false,
    },
    Action {
        title: "Draft copies: current buffers",
        command: "copies buffer",
        operand: false,
    },
    Action {
        title: "Draft deletions: Trash",
        command: "deletes trash",
        operand: false,
    },
    Action {
        title: "Draft deletions: permanently remove",
        command: "deletes permanent",
        operand: false,
    },
    Action {
        title: "Search here",
        command: "search",
        operand: false,
    },
    Action {
        title: "Terminal here",
        command: "terminal",
        operand: false,
    },
    Action {
        title: "Open local terminal",
        command: ":terminal-local",
        operand: false,
    },
];
