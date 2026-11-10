use strop_core::id::DocumentId;

use crate::editor::document::DocumentSource;
use crate::editor::remote::RemoteControl;

use super::model::{FeedbackKind, SaveRetry, TypedAction};
use super::{FeedbackId, FeedbackOwner, FeedbackRecord};
use crate::editor::Editor;

pub(crate) struct FeedbackChoice {
    pub id: FeedbackId,
    pub action: usize,
    pub title: String,
}

impl Editor {
    /// Current valid recovery rows for the focused resource. Invalid stale
    /// actions are never offered; dismissal remains available for unresolved
    /// records because it has no effect on the underlying operation.
    pub(crate) fn feedback_choices(&self, document: DocumentId) -> Vec<FeedbackChoice> {
        let mut choices = Vec::new();
        for record in self.feedback.unresolved() {
            if !self.feedback_relevant(record.owner(), document) {
                continue;
            }
            for (index, action) in record.actions.iter().enumerate() {
                if self.feedback_action_valid(record, action) {
                    choices.push(FeedbackChoice {
                        id: record.id(),
                        action: index,
                        title: format!("{} — {}", action.label(), record.summary()),
                    });
                }
            }
        }
        choices
    }

    pub fn presented_feedback_has_actions(&self) -> bool {
        let Some(record) = self.presented_feedback() else {
            return false;
        };
        self.feedback_choices(self.current())
            .iter()
            .any(|choice| choice.id == record.id())
    }

    fn feedback_relevant(&self, owner: &FeedbackOwner, document: DocumentId) -> bool {
        if document != self.current() {
            return false;
        }
        match owner {
            FeedbackOwner::Save {
                document: owner, ..
            } => *owner == document,
            FeedbackOwner::Remote { endpoint, .. } => self
                .remote_endpoint()
                .is_some_and(|current| current == endpoint),
            FeedbackOwner::RemoteControls => self.remote_endpoint().is_some(),
            FeedbackOwner::Terminal {
                session,
                incarnation,
                phase,
            } => self.terminal_owner_matches(*session, *incarnation, *phase, document),
        }
    }

    pub(crate) fn feedback_action_valid(
        &self,
        record: &FeedbackRecord,
        action: &TypedAction,
    ) -> bool {
        if !record.unresolved() {
            return false;
        }
        match action {
            TypedAction::Dismiss => true,
            TypedAction::RetrySave(retry) => self.save_retry_valid(retry),
            TypedAction::VerifySave { document, revision } => {
                self.docs.get(*document).is_some()
                    && self
                        .io
                        .store_attempt(*document)
                        .is_some_and(|attempt| attempt.revision == *revision)
                    && !self.io.save_pending_for(*document)
                    && !self
                        .filesystem
                        .blocks_namespace(&strop_workspace::Filesystem::Local)
            }
            TypedAction::RetryRemote {
                operation,
                incarnation,
            } => {
                if self.remote.control_pending() {
                    return false;
                }
                match operation {
                    RemoteControl::Connect(endpoint) => {
                        self.remote_incarnation(endpoint).unwrap_or(0) == *incarnation
                    }
                    _ => false,
                }
            }
            TypedAction::Terminal(action) => self.terminal_action_valid(action),
        }
    }

    fn save_retry_valid(&self, retry: &SaveRetry) -> bool {
        let Some(document) = self.docs.get(retry.document) else {
            return false;
        };
        if document.buf.revision() != retry.revision
            || self.filesystem_blocks_document(retry.document)
        {
            return false;
        }
        let remote = matches!(&document.source, DocumentSource::Remote(_));
        if remote {
            !self.remote.writes.pending_for(retry.document)
        } else {
            !self.io.save_pending_for(retry.document)
                && self.io.store_attempt(retry.document).is_none()
        }
    }

    pub(crate) fn invoke_feedback_action(&mut self, id: FeedbackId, index: usize) {
        let Some((action, valid)) = self.feedback.get(id).and_then(|record| {
            record.actions.get(index).cloned().map(|action| {
                let valid = self.feedback_action_valid(record, &action);
                (action, valid)
            })
        }) else {
            self.message = "that feedback action no longer exists; reopen actions".into();
            return;
        };
        if !valid {
            self.message =
                "that feedback action is stale; reopen actions for current recovery".into();
            return;
        }
        match action {
            TypedAction::Dismiss => {
                self.feedback.dismiss(id);
                self.message =
                    "notice dismissed; the operation and its guards are unchanged".into();
            }
            TypedAction::RetrySave(retry) => {
                let remote = self
                    .docs
                    .get(retry.document)
                    .is_some_and(|document| matches!(&document.source, DocumentSource::Remote(_)));
                if remote {
                    self.request_remote_save(
                        retry.document,
                        retry.target,
                        retry.force,
                        retry.close,
                    );
                } else {
                    self.request_save_document(
                        retry.document,
                        retry.target,
                        retry.force,
                        retry.close,
                    );
                }
            }
            TypedAction::VerifySave { document, .. } => {
                self.request_store_verification(document);
            }
            TypedAction::RetryRemote { operation, .. } => {
                self.request_remote_control(operation);
            }
            TypedAction::Terminal(action) => self.invoke_terminal_action(action),
        }
    }

    pub(crate) fn explain_feedback_action(
        &self,
        record: &FeedbackRecord,
        action: &TypedAction,
    ) -> String {
        let title = action.label();
        if self.feedback_action_valid(record, action) {
            title.to_owned()
        } else {
            format!("{title} (stale; no effect)")
        }
    }

    pub(crate) fn feedback_saturated(&self) -> bool {
        self.feedback.saturated()
    }

    pub(crate) fn feedback_kind_label(kind: FeedbackKind) -> &'static str {
        match kind {
            FeedbackKind::Save => "save",
            FeedbackKind::Connection => "connection",
            FeedbackKind::TerminalProcess => "terminal process",
            FeedbackKind::TerminalHostControl => "terminal host control",
        }
    }
}
