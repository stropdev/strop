//! `Space a`: one list of what can be done here, titled "actions". Every
//! row is a typed owner computed as data — retained feedback recoveries,
//! terminal controls, dev-container steps, filesystem/remote operations
//! valid for this view and, as they arrive, the language server's code
//! actions. No branch order decides which list you get (0089 §5): every
//! owner contributes its valid rows to the same list.

use strop_core::id::{BufferRevision, DocumentId};
use strop_picker::{Item, Kind, Payload, Picker};
use strop_workspace::ResourceLocation;

use super::filesystem::actions::ACTIONS;
use super::Editor;

/// One row's owner.
pub(crate) enum Entry {
    Feedback(super::feedback::actions::FeedbackChoice),
    Terminal(super::terminal::actions::TerminalAction),
    Devcontainer(super::devcontainers::DevcontainerAction),
    /// An index into the filesystem table.
    Filesystem(usize),
    /// An index into the language server's pending action list.
    Code(usize),
}

impl Entry {
    /// Rows the user is most likely here for come first: a live problem's
    /// recovery, then the language server's fixes, then everything else.
    fn rank(&self) -> u8 {
        match self {
            Self::Feedback(_) | Self::Terminal(_) => 0,
            Self::Code(_) => 1,
            Self::Devcontainer(_) | Self::Filesystem(_) => 2,
        }
    }
}

/// The view the list was computed for: filesystem rows act on it only
/// while it is unchanged.
#[derive(PartialEq, Eq)]
struct Stamp {
    revision: BufferRevision,
    focus: u64,
    head: usize,
    location: Option<ResourceLocation>,
}

/// The list's owner, carried by the picker that shows it (and dropped
/// with it).
pub(crate) struct ActionOwner {
    document: DocumentId,
    stamp: Stamp,
    entries: Vec<Entry>,
    /// A code-action request for this list is in flight.
    awaiting_code: bool,
}

impl Editor {
    /// `Space a`.
    pub(crate) fn context_actions(&mut self) {
        let mut entries = self.context_entries();
        entries.extend(self.devcontainer_entries());
        entries.extend(self.filesystem_entries().into_iter().map(Entry::Filesystem));
        let code = self.lsp_serves(self.current());
        if entries.is_empty() && !code {
            // nothing else to offer: the language request names why
            self.lsp_code_actions();
            return;
        }
        let filesystem = entries
            .iter()
            .any(|entry| matches!(entry, Entry::Filesystem(_)));
        let message = if filesystem {
            format!(
                "filesystem scope: {} — mutations always require review",
                self.directory_context().label()
            )
        } else {
            "actions: owners and tickets are revalidated".into()
        };
        self.open_actions(entries, code, &message);
        if code {
            self.lsp_code_actions();
        }
    }

    /// `:fs` without a verb: the filesystem (and dev-container) rows only.
    pub(crate) fn open_filesystem_actions(&mut self) {
        let mut entries: Vec<Entry> = self.devcontainer_entries();
        entries.extend(self.filesystem_entries().into_iter().map(Entry::Filesystem));
        let message = format!(
            "filesystem scope: {} — mutations always require review",
            self.directory_context().label()
        );
        self.open_actions(entries, false, &message);
    }

    pub(crate) fn open_terminal_quit_review(&mut self) {
        let entries = self
            .terminal_quit_actions()
            .into_iter()
            .map(Entry::Terminal)
            .collect();
        self.open_actions(
            entries,
            false,
            "live terminals: Focus, Stop, or Return; quit does not kill implicitly",
        );
    }

    /// Retained feedback and terminal controls for this view. A feedback
    /// row that only repeats a live terminal control yields to it.
    fn context_entries(&self) -> Vec<Entry> {
        let terminal = self.terminal_context_actions();
        let mut entries: Vec<Entry> = self
            .feedback_choices(self.current())
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
            .map(Entry::Feedback)
            .collect();
        entries.extend(terminal.into_iter().map(Entry::Terminal));
        entries
    }

    fn devcontainer_entries(&mut self) -> Vec<Entry> {
        let Some(workspace) = self.devcontainers.document_workspace(self.current()) else {
            return Vec::new();
        };
        let Some(discovery) = self.devcontainers.discovery_incarnation(&workspace) else {
            return Vec::new();
        };
        self.devcontainer_actions(&workspace, discovery)
            .into_iter()
            .filter(|action| self.devcontainer_action_valid(action))
            .map(Entry::Devcontainer)
            .collect()
    }

    /// The filesystem table rows valid for this view and its namespace.
    fn filesystem_entries(&self) -> Vec<usize> {
        let scope = self.directory_context();
        let directory = self.directory().is_some();
        let draft = self.filename_draft(self.current()).is_some();
        let local = scope.filesystem == strop_workspace::Filesystem::Local;
        let container = matches!(scope.filesystem, strop_workspace::Filesystem::Container(_));
        let available = |capability| {
            self.namespace_capability_for(&scope, capability, Some(self.current()))
                .available()
        };
        ACTIONS
            .iter()
            .enumerate()
            .filter(|(_, action)| {
                (!action.mutation || !container)
                    && match action.command {
                        "trash" => local && !draft,
                        "deletes trash" => local && draft,
                        ":w" | "discard" | "copies stored" | "copies buffer"
                        | "deletes permanent" => draft,
                        "operations" | "verify" | "path" => true,
                        "refresh" => directory,
                        "terminal" => {
                            directory
                                && !draft
                                && available(crate::editor::WorkspaceCapability::Terminal)
                        }
                        ":terminal-local" => !draft && !local,
                        "search" => {
                            directory && available(crate::editor::WorkspaceCapability::Search)
                        }
                        "mark" | "unmark" | "edit" => directory && !draft,
                        _ => !draft,
                    }
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn action_stamp(&self) -> Stamp {
        Stamp {
            revision: self.buf().revision(),
            focus: self.focus_epoch,
            head: self.head(),
            location: self
                .cur()
                .file_target(&self.cwd)
                .and_then(|target| target.resource_location()),
        }
    }

    fn action_title(&self, entry: &Entry) -> String {
        match entry {
            Entry::Feedback(choice) => choice.title.clone(),
            Entry::Terminal(action) => self.terminal_action_title(action),
            Entry::Devcontainer(action) => action.title(),
            Entry::Filesystem(index) => ACTIONS[*index].title.into(),
            Entry::Code(index) => self
                .changes
                .pending_actions
                .get(*index)
                .map_or_else(String::new, |action| action.title.clone()),
        }
    }

    fn action_items(&self, entries: &[Entry]) -> Vec<Item> {
        entries
            .iter()
            .enumerate()
            .map(|(index, entry)| Item {
                badge: None,
                text: self.action_title(entry),
                payload: Payload::Action(index),
            })
            .collect()
    }

    fn open_actions(&mut self, mut entries: Vec<Entry>, awaiting_code: bool, message: &str) {
        entries.sort_by_key(Entry::rank);
        let picker = Picker::new(Kind::Actions, self.action_items(&entries), awaiting_code);
        let owner = ActionOwner {
            document: self.current(),
            stamp: self.action_stamp(),
            entries,
            awaiting_code,
        };
        self.set_picker(super::picker::PickerGlue::diagnostics(picker));
        let Some(glue) = self
            .picker
            .as_mut()
            .filter(|glue| glue.picker.kind == Kind::Actions)
        else {
            return;
        };
        glue.actions = Some(owner);
        self.message = message.into();
    }

    /// The language server answered `Space a`: its actions join the open
    /// list (a closed list wanted nothing; an empty answer to an otherwise
    /// empty list says so).
    pub(crate) fn deliver_code_actions(
        &mut self,
        actions: Vec<strop_lsp::ProtoAction>,
        encoding: strop_lsp::PositionEncoding,
    ) {
        let current = self.current();
        let Some(mut owner) = self
            .picker
            .as_mut()
            .filter(|glue| {
                glue.picker.kind == Kind::Actions
                    && glue
                        .actions
                        .as_ref()
                        .is_some_and(|owner| owner.awaiting_code && owner.document == current)
            })
            .and_then(|glue| glue.actions.take())
        else {
            return;
        };
        owner.awaiting_code = false;
        owner.entries.extend((0..actions.len()).map(Entry::Code));
        owner.entries.sort_by_key(Entry::rank);
        self.changes.pending_actions = actions;
        self.changes.pending_encoding = encoding;
        let items = self.action_items(&owner.entries);
        let empty = owner.entries.is_empty();
        if let Some(glue) = self.picker.as_mut() {
            glue.picker.streaming = false;
            glue.picker.clear_items();
            glue.picker.append(items);
            glue.actions = Some(owner);
        }
        if empty {
            self.close_picker();
            self.message = "no actions here".into();
            return;
        }
        self.request_picker_ranking();
    }

    /// Enter on a row: run it through its owner's own revalidation.
    pub(crate) fn accept_action(&mut self, index: usize) {
        let owner = self.picker.as_mut().and_then(|glue| glue.actions.take());
        let fresh = owner.as_ref().is_some_and(|owner| {
            owner.document == self.current() && owner.stamp == self.action_stamp()
        });
        self.close_picker();
        let Some(mut owner) = owner else {
            self.message = "action list lost its source; reopen actions".into();
            return;
        };
        if index >= owner.entries.len() {
            self.message = "unknown action".into();
            return;
        }
        match owner.entries.swap_remove(index) {
            Entry::Feedback(choice) => self.invoke_feedback_action(choice.id, choice.action),
            Entry::Terminal(action) => self.invoke_terminal_action(action),
            Entry::Devcontainer(action) => self.invoke_devcontainer_action(action),
            Entry::Code(index) => self.accept_code_action(index),
            Entry::Filesystem(_) if !fresh => {
                self.message = "the view changed since actions were listed; reopen actions".into();
            }
            Entry::Filesystem(index) => self.run_filesystem_action(index),
        }
    }

    fn run_filesystem_action(&mut self, index: usize) {
        let action = &ACTIONS[index];
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
            self.feed_pending_event(super::pending::PendingEvent::Paste(format!(
                "fs {} ",
                action.command
            )));
        } else {
            self.run_filesystem_ex(action.command, None);
        }
    }
}

#[cfg(test)]
mod tests;
