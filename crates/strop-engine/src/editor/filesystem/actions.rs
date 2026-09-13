use super::*;
use strop_picker::{Item, Kind, Payload, Picker};
struct Action {
    title: &'static str,
    command: &'static str,
    operand: bool,
    mutation: bool,
}
const ACTIONS: &[Action] = &[
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
];
pub(super) struct ActionOwner {
    picker: super::super::picker::PickerId,
    document: DocumentId,
    revision: BufferRevision,
    focus: u64,
    head: usize,
    source: Option<ResourceLocation>,
}
impl Editor {
    pub(crate) fn context_actions(&mut self) {
        if self.directory().is_some() {
            self.open_filesystem_actions();
        } else {
            self.lsp_code_actions_pub();
        }
    }
    pub(crate) fn open_filesystem_actions(&mut self) {
        let scope = self.directory_context();
        let directory = self.directory().is_some();
        let draft = self.filename_draft(self.current()).is_some();
        let items = ACTIONS
            .iter()
            .enumerate()
            .filter(|(_, action)| {
                (!action.mutation
                    || !matches!(scope.filesystem, strop_workspace::Filesystem::Container(_)))
                    && (action.command != "trash"
                        || scope.filesystem == strop_workspace::Filesystem::Local)
                    && (action.command != "deletes trash"
                        || scope.filesystem == strop_workspace::Filesystem::Local)
                    && match action.command {
                        ":w" | "discard" | "copies stored" | "copies buffer" | "deletes trash"
                        | "deletes permanent" => draft,
                        "operations" | "verify" | "path" => true,
                        "refresh" => directory,
                        "search" => {
                            directory
                                && !matches!(
                                    scope.filesystem,
                                    strop_workspace::Filesystem::Container(_)
                                )
                        }
                        "mark" | "unmark" | "edit" => directory && !draft,
                        _ => !draft,
                    }
            })
            .map(|(index, action)| Item {
                badge: None,
                text: action.title.into(),
                payload: Payload::FilesystemAction(index),
            })
            .collect();
        let picker = Picker::new(Kind::FilesystemActions, items, false);
        self.set_picker(super::super::picker::PickerGlue::diagnostics(picker));
        let Some(picker) = self
            .picker
            .as_ref()
            .filter(|glue| glue.picker.kind == Kind::FilesystemActions)
        else {
            return;
        };
        self.filesystem.actions = Some(ActionOwner {
            picker: picker.id,
            document: self.current(),
            revision: self.buf().revision(),
            focus: self.focus_epoch,
            head: self.head(),
            source: self
                .cur()
                .file_target(&self.cwd)
                .and_then(|target| target.resource_location()),
        });
        self.message = format!(
            "filesystem scope: {} — mutations always require review",
            scope.label()
        );
    }
    pub(crate) fn revoke_filesystem_actions(&mut self, picker: super::super::picker::PickerId) {
        if self
            .filesystem
            .actions
            .as_ref()
            .is_some_and(|owner| owner.picker == picker)
        {
            self.filesystem.actions = None;
        }
    }
    pub(crate) fn accept_filesystem_action(
        &mut self,
        picker: super::super::picker::PickerId,
        index: usize,
    ) {
        let Some(owner) = self.filesystem.actions.take() else {
            self.message = "filesystem selector lost its source".into();
            return;
        };
        let valid = owner.picker == picker
            && !self.docs.is_empty()
            && owner.document == self.current()
            && owner.revision == self.buf().revision()
            && owner.focus == self.focus_epoch
            && owner.head == self.head()
            && owner.source
                == self
                    .cur()
                    .file_target(&self.cwd)
                    .and_then(|target| target.resource_location());
        self.close_picker();
        if !valid {
            self.message = "filesystem selector source changed; reopen actions".into();
            return;
        }
        let Some(action) = ACTIONS.get(index) else {
            self.message = "unknown filesystem action".into();
            return;
        };
        if action.command == ":w" {
            self.request_save_document(self.current(), None, false, false);
            return;
        }
        if action.operand {
            self.begin_text_line(':', Default::default());
            self.feed_pending_event(super::super::pending::PendingEvent::Paste(format!(
                "fs {} ",
                action.command
            )));
        } else {
            self.run_filesystem_ex(action.command, None);
        }
    }
}
