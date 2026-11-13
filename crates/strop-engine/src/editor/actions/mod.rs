//! `Space a`: one list of what can be done here, titled "actions". Every
//! row is a typed owner computed as data — retained feedback recoveries,
//! terminal controls, a review's apply/revert/cancel and, as they arrive,
//! the language server's code actions. A folder adds three rows (search
//! in it, a terminal in it, copy its path); changing files is editing the
//! folder's names (0102). On a source file the list is what can be done
//! to the code (0094 §3.4). No branch order decides which
//! list you get (0089 §5): every owner contributes its valid rows.

use strop_core::id::{BufferRevision, DocumentId};
use strop_picker::{Item, Kind, Payload, Picker};
use strop_workspace::ResourceLocation;

use super::Editor;

/// One row's owner.
pub(crate) enum Entry {
    Feedback(super::feedback::actions::FeedbackChoice),
    Terminal(super::terminal::actions::TerminalAction),
    Review(ReviewAction),
    /// Something to do with the folder in view.
    Folder(FolderAction),
    /// An index into the language server's pending action list.
    Code(usize),
}

/// What a review in view offers; the keys do the same (`Enter`, `u`, `q`).
#[derive(Clone, Copy)]
pub(crate) enum ReviewAction {
    Apply,
    Revert,
    Cancel,
}

/// What `Space a` offers in a folder.
#[derive(Clone, Copy)]
pub(crate) enum FolderAction {
    Search,
    Terminal,
    CopyPath,
}

impl FolderAction {
    const ALL: [Self; 3] = [Self::Search, Self::Terminal, Self::CopyPath];
    fn title(self) -> &'static str {
        match self {
            Self::Search => "Search in this folder",
            Self::Terminal => "Terminal in this folder",
            Self::CopyPath => "Copy path",
        }
    }
}

impl ReviewAction {
    fn title(self) -> &'static str {
        match self {
            Self::Apply => "Apply and write the files",
            Self::Revert => "Revert the buffers and the files",
            Self::Cancel => "Cancel this review",
        }
    }
}

impl Entry {
    /// Rows the user is most likely here for come first: a live problem's
    /// recovery, then the language server's fixes, then everything else.
    fn rank(&self) -> u8 {
        match self {
            Self::Feedback(_) | Self::Terminal(_) | Self::Review(_) => 0,
            Self::Code(_) => 1,
            Self::Folder(_) => 2,
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
        let entries = self.local_entries();
        let code = self.lsp_serves(self.current());
        if entries.is_empty() && !code {
            // nothing else to offer: the language request names why
            self.lsp_code_actions();
            return;
        }
        self.open_actions(entries, code, "actions");
        if code {
            self.lsp_code_actions();
        }
    }

    /// Every row this view offers without asking a language server.
    fn local_entries(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self
            .review_actions()
            .into_iter()
            .map(Entry::Review)
            .collect();
        entries.extend(self.context_entries());
        if self.directory().is_some() {
            entries.extend(FolderAction::ALL.into_iter().map(Entry::Folder));
        }
        entries
    }

    /// Fixture hook (tests only): `Space a` as it looks once a language
    /// server has answered with `titles`, without a server round-trip.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture_code_actions(&mut self, titles: &[&str]) {
        let entries = self.local_entries();
        self.open_actions(entries, true, "actions");
        let actions = titles
            .iter()
            .map(|title| strop_lsp::ProtoAction {
                title: (*title).into(),
                edits: None,
                has_external_command: false,
            })
            .collect();
        self.deliver_code_actions(actions, strop_lsp::PositionEncoding::Utf16);
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
            Entry::Review(action) => action.title().into(),
            Entry::Folder(action) => action.title().into(),
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
            Entry::Review(_) if !fresh => {
                self.message = "the view changed since actions were listed; reopen actions".into();
            }
            Entry::Review(ReviewAction::Apply) => self.review_apply_pub(),
            Entry::Review(ReviewAction::Revert) => self.review_revert(),
            Entry::Review(ReviewAction::Cancel) => self.review_cancel_pub(),
            Entry::Code(index) => self.accept_code_action(index),
            Entry::Folder(action) => self.run_folder_action(action),
        }
    }

    fn run_folder_action(&mut self, action: FolderAction) {
        let Some(location) = self.directory().map(|source| source.location.clone()) else {
            self.message = "this action needs a folder".into();
            return;
        };
        match action {
            FolderAction::Search => {
                self.open_search_in(super::picker::SearchScope { root: location }, false)
            }
            FolderAction::Terminal => self.launch_terminal_at(location, ""),
            FolderAction::CopyPath => {
                let path = match location.local_path() {
                    Some(path) => Ok(path.display().to_string()),
                    None => location.uri().map_err(|error| error.to_string()),
                };
                match path {
                    Ok(path) => {
                        self.yank_register(Some('+'), super::Register::characterwise(path));
                        self.message = "path copied".into();
                    }
                    Err(error) => self.message = error,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
