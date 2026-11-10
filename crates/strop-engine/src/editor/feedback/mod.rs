//! Owner-backed, bounded feedback for meaningful failures and decisions.
//!
//! `Editor.message` remains the ephemeral confirmation channel. Records here
//! survive unrelated input, project through `Space a` and `:explain`, and keep
//! recovery tied to the exact document/endpoint incarnation.

pub(crate) mod actions;
mod messages;
mod model;
mod store;
#[cfg(test)]
mod tests;

pub use model::{
    FeedbackId, FeedbackLifecycle, FeedbackOwner, FeedbackRecord, FeedbackReference,
    FeedbackSeverity, TerminalFeedbackPhase,
};

pub(crate) use model::{FeedbackEvent, FeedbackKind, SaveRetry, TypedAction};
pub(crate) use store::FeedbackStore;

use std::path::PathBuf;

use strop_core::id::{BufferRevision, DocumentId};
use strop_workspace::RemoteEndpoint;

use super::remote::RemoteControl;
use super::Editor;

impl Editor {
    /// All retained records, including recent resolved results. Frontends should
    /// use [`Editor::presented_feedback`] for the single non-focusing notice.
    pub fn feedback(&self) -> &[FeedbackRecord] {
        self.feedback.records()
    }

    /// The highest-severity active record (latest wins ties). Dismissed and
    /// resolved records remain available to actions/`:explain` but are not
    /// presented as the current notice.
    pub fn presented_feedback(&self) -> Option<&FeedbackRecord> {
        self.feedback.presented()
    }

    /// After every event: retain whatever the echo row now says.
    pub(crate) fn log_echo_message(&mut self) {
        self.feedback.log.observe(&self.message);
    }

    /// `:messages`: the retained history as a temporary output buffer.
    pub(crate) fn open_messages(&mut self) {
        // the command's own echo is not history
        self.log_echo_message();
        let mut buffer = strop_core::Buffer::from_text(&self.feedback.log.render());
        buffer.name = Some("messages".into());
        if self.open_temporary_output(buffer).is_some() {
            // land on the newest message, like vim's pager end
            let last = self.buf().len_lines().saturating_sub(2);
            self.set_head(self.buf().line_start(last));
        }
    }

    pub(crate) fn remote_feedback_owner(&self, endpoint: &RemoteEndpoint) -> FeedbackOwner {
        FeedbackOwner::Remote {
            endpoint: endpoint.clone(),
            incarnation: self.remote_incarnation(endpoint).unwrap_or(0),
        }
    }

    pub(crate) fn remote_incarnation(&self, endpoint: &RemoteEndpoint) -> Option<u64> {
        // Endpoint actions predate project roots and must not borrow whichever
        // root happens to iterate first. A disconnect advances every root;
        // the maximum remains the conservative endpoint epoch if an exhausted
        // root is retired and later reselected from incarnation zero.
        self.workspaces
            .iter()
            .filter(|(_, context)| {
                matches!(&context.filesystem, strop_workspace::Filesystem::Remote(current) if current == endpoint)
            })
            .map(|(_, context)| context.incarnation)
            .max()
    }

    pub(crate) fn record(&mut self, event: FeedbackEvent) {
        let _ = self.feedback.record(event);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn note_save_failure(
        &mut self,
        document: DocumentId,
        remote: bool,
        revision: BufferRevision,
        target: Option<PathBuf>,
        force: bool,
        close: bool,
        retry: bool,
        detail: String,
    ) {
        let owner = FeedbackOwner::Save { document, remote };
        let mut actions = Vec::new();
        if retry {
            actions.push(TypedAction::RetrySave(SaveRetry {
                document,
                revision,
                target,
                force,
                close,
            }));
        }
        actions.push(TypedAction::Dismiss);
        self.record(FeedbackEvent {
            severity: FeedbackSeverity::Failure,
            owner,
            kind: FeedbackKind::Save,
            summary: if remote {
                "remote save failed".into()
            } else {
                "local save failed".into()
            },
            detail,
            reference: None,
            actions,
        });
    }

    pub(crate) fn note_unconfirmed_save(
        &mut self,
        document: DocumentId,
        revision: BufferRevision,
        detail: String,
    ) {
        self.record(FeedbackEvent {
            severity: FeedbackSeverity::Warning,
            owner: FeedbackOwner::Save {
                document,
                remote: false,
            },
            kind: FeedbackKind::Save,
            summary: "local save unconfirmed — verify".into(),
            detail,
            reference: Some(FeedbackReference::LocalStore { document, revision }),
            actions: vec![
                TypedAction::VerifySave { document, revision },
                TypedAction::Dismiss,
            ],
        });
    }

    pub(crate) fn resolve_save_feedback(&mut self, document: DocumentId, remote: bool) {
        self.feedback.resolve_owner(
            &FeedbackOwner::Save { document, remote },
            FeedbackKind::Save,
        );
    }

    pub(crate) fn note_remote_failure(&mut self, operation: &RemoteControl, detail: String) {
        let (owner, incarnation, retry, kind, summary) = match operation {
            RemoteControl::Connect(endpoint) | RemoteControl::Disconnect(endpoint) => (
                self.remote_feedback_owner(endpoint),
                self.remote_incarnation(endpoint).unwrap_or(0),
                matches!(operation, RemoteControl::Connect(_)),
                FeedbackKind::Connection,
                format!("remote connection operation failed for {endpoint}"),
            ),
            RemoteControl::DisconnectAll | RemoteControl::Connections => (
                FeedbackOwner::RemoteControls,
                0,
                false,
                FeedbackKind::Connection,
                "remote connection operation failed".into(),
            ),
        };
        let mut actions = Vec::new();
        if retry {
            actions.push(TypedAction::RetryRemote {
                operation: operation.clone(),
                incarnation,
            });
        }
        actions.push(TypedAction::Dismiss);
        self.record(FeedbackEvent {
            severity: FeedbackSeverity::Failure,
            owner,
            kind,
            summary,
            detail,
            reference: None,
            actions,
        });
    }

    pub(crate) fn note_remote_recovered(&mut self, operation: &RemoteControl) {
        match operation {
            RemoteControl::Connect(endpoint) | RemoteControl::Disconnect(endpoint) => self
                .feedback
                .resolve_remote(endpoint, FeedbackKind::Connection),
            RemoteControl::DisconnectAll | RemoteControl::Connections => self
                .feedback
                .resolve_owner(&FeedbackOwner::RemoteControls, FeedbackKind::Connection),
        }
    }
}
