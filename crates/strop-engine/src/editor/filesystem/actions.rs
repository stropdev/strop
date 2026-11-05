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
pub(super) struct ActionOwner {
    picker: super::super::picker::PickerId,
    document: DocumentId,
    source: ActionSource,
}

enum ContextChoice {
    Feedback(super::super::feedback::actions::FeedbackChoice),
    Terminal(super::super::terminal::actions::TerminalAction),
    Devcontainer(super::super::devcontainers::DevcontainerAction),
}

enum ActionSource {
    Filesystem {
        revision: BufferRevision,
        focus: u64,
        head: usize,
        location: Option<ResourceLocation>,
    },
    Context {
        choices: Vec<ContextChoice>,
    },
}

impl Editor {
    pub(crate) fn context_actions(&mut self) {
        let feedback = self.feedback_choices(self.current());
        let terminal = self.terminal_context_actions();
        let feedback: Vec<_> = feedback
            .into_iter()
            .filter(|choice| {
                terminal.is_empty()
                    || !self.feedback.get(choice.id).is_some_and(|record| {
                        matches!(
                            record.actions.get(choice.action),
                            Some(crate::editor::feedback::TypedAction::Terminal(_))
                        )
                    })
            })
            .collect();
        if !feedback.is_empty() || !terminal.is_empty() {
            let choices = feedback
                .into_iter()
                .map(ContextChoice::Feedback)
                .chain(terminal.into_iter().map(ContextChoice::Terminal))
                .collect();
            self.open_context_actions(
                choices,
                "context actions: owners and tickets are revalidated",
            );
        } else if self.directory().is_some() {
            self.open_filesystem_actions();
        } else {
            self.lsp_code_actions_pub();
        }
    }

    pub(crate) fn open_terminal_quit_review(&mut self) {
        let choices = self
            .terminal_quit_actions()
            .into_iter()
            .map(ContextChoice::Terminal)
            .collect();
        self.open_context_actions(
            choices,
            "live terminals: Focus, Stop, or Return; quit does not kill implicitly",
        );
    }

    fn open_context_actions(&mut self, choices: Vec<ContextChoice>, message: &str) {
        let items = choices
            .iter()
            .enumerate()
            .map(|(index, choice)| Item {
                badge: None,
                text: match choice {
                    ContextChoice::Feedback(choice) => choice.title.clone(),
                    ContextChoice::Terminal(action) => self.terminal_action_title(action),
                    ContextChoice::Devcontainer(action) => action.title(),
                },
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
            source: ActionSource::Context { choices },
        });
        self.message = message.into();
    }

    pub(crate) fn open_filesystem_actions(&mut self) {
        if let Some(workspace) = self.devcontainers.document_workspace(self.current()) {
            if let Some(discovery) = self.devcontainers.discovery_incarnation(&workspace) {
                let choices = self
                    .devcontainer_actions(&workspace, discovery)
                    .into_iter()
                    .filter(|action| self.devcontainer_action_valid(action))
                    .map(ContextChoice::Devcontainer)
                    .collect();
                self.open_context_actions(
                    choices,
                    "dev container actions: configuration, discovery and transition are revalidated",
                );
                return;
            }
        }
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
                        "terminal" => {
                            directory
                                && !draft
                                && self
                                    .namespace_capability_for(
                                        &scope,
                                        crate::editor::WorkspaceCapability::Terminal,
                                        Some(self.current()),
                                    )
                                    .available()
                        }
                        ":terminal-local" => {
                            !draft && scope.filesystem != strop_workspace::Filesystem::Local
                        }
                        "search" => {
                            directory
                                && self
                                    .namespace_capability_for(
                                        &scope,
                                        crate::editor::WorkspaceCapability::Search,
                                        Some(self.current()),
                                    )
                                    .available()
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
            source: ActionSource::Filesystem {
                revision: self.buf().revision(),
                focus: self.focus_epoch,
                head: self.head(),
                location: self
                    .cur()
                    .file_target(&self.cwd)
                    .and_then(|target| target.resource_location()),
            },
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
        if owner.picker != picker {
            self.close_picker();
            self.message = "action selector source changed; reopen actions".into();
            return;
        }
        match owner.source {
            ActionSource::Context { choices } => {
                self.close_picker();
                let Some(choice) = choices.get(index) else {
                    self.message = "unknown context action".into();
                    return;
                };
                match choice {
                    ContextChoice::Feedback(choice) => {
                        self.invoke_feedback_action(choice.id, choice.action);
                    }
                    ContextChoice::Terminal(action) => {
                        self.invoke_terminal_action(action.clone());
                    }
                    ContextChoice::Devcontainer(action) => {
                        self.invoke_devcontainer_action(action.clone());
                    }
                }
            }
            ActionSource::Filesystem {
                revision,
                focus,
                head,
                location,
            } => {
                let valid = !self.docs.is_empty()
                    && owner.document == self.current()
                    && revision == self.buf().revision()
                    && focus == self.focus_epoch
                    && head == self.head()
                    && location
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
                if let Some(command) = action.command.strip_prefix(':') {
                    self.run_ex(command);
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
    }
}
