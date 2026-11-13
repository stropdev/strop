//! Filesystem proposal and mutation lifetime is independent of its originating view.
pub mod draft;
mod lsp_rename;
#[cfg(test)]
mod lsp_rename_tests;
mod prepare;
mod reconcile;
mod render;
mod shutdown;
#[cfg(test)]
mod tests;
mod verify;
use super::io::IoEvent;
use super::{Editor, ReviewRow};
use std::collections::{HashMap, VecDeque};
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, Completion, FailureKind, Outcome, Ticket, WorkerId};
use strop_workspace::operation::*;
use strop_workspace::ResourceLocation;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FsKey {
    pub origin: DocumentId,
    pub revision: BufferRevision,
    pub focus: u64,
    pub draft: Option<draft::Stamp>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VerifyKey {
    pub operation: WorkerId,
    pub step: usize,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreparedFilesystem {
    pub batch: strop_fs::batch::PreparedBatch,
    pub draft_targets: Vec<(u64, ResourceLocation)>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub enum FsEvent {
    Prepared(Box<Completion<FsKey, PreparedFilesystem>>),
    Applied(Box<Completion<FsKey, Vec<StepReceipt>>>),
    Verified(Box<Completion<VerifyKey, VerifiedOutcome>>),
}

pub(crate) struct Proposal {
    ticket: Ticket<FsKey>,
    batch: strop_fs::batch::PreparedBatch,
    report: DocumentId,
    view_revision: BufferRevision,
}
/// A reviewed proposal parked while language servers answer
/// `workspace/willRenameFiles` (lsp_rename.rs). The apply resumes when
/// every admitted stamp has replied; cancellation demotes the proposal
/// back to `pending`.
pub(crate) struct WillRenameStage {
    proposal: Proposal,
    stamps: Vec<strop_lsp::RequestStamp>,
    servers: lsp_rename::RenameServers,
}
pub(crate) struct Attempt {
    ticket: Ticket<FsKey>,
    batch: strop_fs::batch::PreparedBatch,
    report: DocumentId,
    receipts: Vec<StepReceipt>,
    /// Original publication evidence survives later verification observations.
    publication: Vec<Option<strop_workspace::Observation>>,
    warning: Option<String>,
    /// Servers that prepared these renames; notified on commit.
    lsp_renames: lsp_rename::RenameServers,
}
pub(crate) struct FsState {
    /// The local filesystem, in process.
    local: super::namespace::LocalFs,
    environment: strop_fs::Environment,
    preparing: Option<Ticket<FsKey>>,
    pending: Option<Proposal>,
    will_rename: Option<WillRenameStage>,
    running: Option<WorkerId>,
    verifying: Option<Ticket<VerifyKey>>,
    history: VecDeque<Attempt>,
    reports: std::collections::HashSet<DocumentId>,
}
impl Default for FsState {
    fn default() -> Self {
        Self {
            local: super::namespace::LocalFs::default(),
            environment: strop_fs::Environment::capture(),
            preparing: None,
            pending: None,
            will_rename: None,
            running: None,
            verifying: None,
            history: VecDeque::new(),
            reports: Default::default(),
        }
    }
}
impl FsState {
    /// The local filesystem; prepared authority is bound to its one
    /// execution context.
    pub(crate) fn local(&self) -> &super::namespace::LocalFs {
        &self.local
    }
    pub(crate) fn pending(&self) -> bool {
        self.preparing.is_some()
            || self.will_rename.is_some()
            || self.running.is_some()
            || self.verifying.is_some()
    }
    /// `document` shows changes awaiting apply or cancel (also while
    /// language servers prepare a rename).
    pub(crate) fn reviews(&self, document: DocumentId) -> bool {
        self.pending
            .as_ref()
            .or(self.will_rename.as_ref().map(|stage| &stage.proposal))
            .is_some_and(|proposal| proposal.report == document)
    }
    pub(crate) fn mutation_pending(&self, request: WorkerId) -> bool {
        self.running == Some(request)
    }
    fn blocked_locations(&self) -> impl Iterator<Item = &ResourceLocation> {
        self.history.iter().flat_map(move |attempt| {
            attempt
                .batch
                .steps
                .iter()
                .enumerate()
                .filter(move |(index, _)| {
                    self.running == Some(attempt.ticket.request)
                        || attempt
                            .receipts
                            .get(*index)
                            .is_some_and(|receipt| receipt.outcome.is_unconfirmed())
                })
                .flat_map(|(_, step)| {
                    step.source
                        .iter()
                        .chain(step.destination.iter())
                        .map(|resource| &resource.location)
                })
        })
    }
    pub(crate) fn blocks(&self, location: &ResourceLocation) -> bool {
        self.blocked_locations().any(|resource| {
            resource.filesystem == location.filesystem
                && (resource.path.starts_with(&location.path)
                    || location.path.starts_with(&resource.path))
        })
    }
    pub(crate) fn blocks_namespace(&self, namespace: &strop_workspace::Filesystem) -> bool {
        self.blocked_locations()
            .any(|resource| &resource.filesystem == namespace)
    }
}
impl Editor {
    pub(crate) fn filesystem_report_opened(&mut self, document: DocumentId) {
        self.filesystem.reports.insert(document);
    }
    pub(crate) fn is_filesystem_report(&self, document: DocumentId) -> bool {
        self.filesystem.reports.contains(&document)
    }
    pub(crate) fn filesystem_forget_view(&mut self, document: DocumentId) {
        self.filesystem.reports.remove(&document);
        if self
            .filesystem
            .pending
            .as_ref()
            .is_some_and(|proposal| proposal.report == document)
        {
            self.filesystem.pending = None;
        }
        if self
            .filesystem
            .preparing
            .as_ref()
            .is_some_and(|ticket| ticket.key.origin == document)
        {
            if let Some(ticket) = self.filesystem.preparing.take() {
                if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                    handle.cancel(worker::CancelReason::OwnerClosed);
                }
            }
        }
    }
    pub(crate) fn retire_filesystem_review(&mut self, reason: &str) {
        if let Some(proposal) = self.filesystem.pending.take() {
            if self.docs.get(proposal.report).is_some() {
                let text = format!("{reason}\nnothing changed\n");
                if let Err(error) = self.replace_system(proposal.report, &text) {
                    self.message = error.to_string();
                }
            }
        }
    }
    pub(crate) fn filesystem_blocks_document(&self, document: DocumentId) -> bool {
        self.docs
            .get(document)
            .and_then(|document| document.file_target(&self.cwd))
            .and_then(|target| target.resource_location())
            .is_some_and(|location| self.filesystem.blocks(&location))
    }

    pub(crate) fn apply_filesystem_review(&mut self) -> bool {
        if self
            .filesystem
            .pending
            .as_ref()
            .is_none_or(|proposal| proposal.report != self.current())
        {
            return false;
        }
        let Some(proposal) = self.filesystem.pending.take() else {
            return false;
        };
        if self.filesystem.will_rename.is_some() {
            self.filesystem.pending = Some(proposal);
            self.message = "language-server rename preparation is still in flight".into();
            return true;
        }
        if self
            .docs
            .get(proposal.report)
            .is_none_or(|document| document.buf.revision() != proposal.view_revision)
        {
            self.filesystem.pending = Some(proposal);
            self.message = "these changes went stale; :w in the folder again".into();
            return true;
        }
        if proposal.ticket.key.draft.as_ref().is_some_and(|stamp| {
            !self.filename_draft_fresh(
                proposal.ticket.key.origin,
                proposal.ticket.key.revision,
                stamp,
            )
        }) {
            self.filesystem.pending = Some(proposal);
            self.message = "the folder changed since :w; q, then :w again".into();
            return true;
        }
        let intents: Vec<_> = proposal
            .batch
            .steps
            .iter()
            .map(|step| OperationIntent {
                kind: step.intent.kind,
                copy_version: step.intent.copy_version,
                source: step.source.as_ref().map(|source| source.location.clone()),
                destination: step
                    .destination
                    .as_ref()
                    .map(|destination| destination.location.clone()),
                expected_content: step.intent.expected_content,
                store: step.intent.store,
            })
            .collect();
        if let Err(error) = self.filesystem_admission(
            &intents,
            proposal
                .ticket
                .key
                .draft
                .as_ref()
                .map(|_| proposal.ticket.key.origin),
        ) {
            self.filesystem.pending = Some(proposal);
            self.message = error;
            return true;
        }
        let unresolved = self
            .filesystem
            .history
            .iter()
            .filter(|attempt| {
                attempt
                    .receipts
                    .iter()
                    .any(|receipt| receipt.outcome.is_unconfirmed())
            })
            .map(|attempt| attempt.batch.steps.len())
            .sum::<usize>();
        if unresolved + proposal.batch.steps.len() > 1024 {
            self.filesystem.pending = Some(proposal);
            self.message = "too many unconfirmed changes; reopen the folders they touched".into();
            return true;
        }
        if proposal.batch.steps.is_empty() {
            self.message = "nothing to apply".into();
            self.filesystem.pending = Some(proposal);
            return true;
        }
        if let Some((stamps, servers)) = self.admit_will_rename(&proposal) {
            self.filesystem.will_rename = Some(WillRenameStage {
                proposal,
                stamps,
                servers,
            });
            self.message = "awaiting language-server rename preparation…".into();
            return true;
        }
        self.dispatch_filesystem_apply(proposal, Vec::new());
        true
    }

    /// The mutation half of an accepted review: draft freeze, history
    /// admission, worker spawn. Runs either straight from
    /// `apply_filesystem_review` or from `will_rename_edits` once every
    /// admitted language server answered (lsp_rename.rs).
    fn dispatch_filesystem_apply(
        &mut self,
        proposal: Proposal,
        lsp_renames: lsp_rename::RenameServers,
    ) {
        let request = proposal.ticket.request;
        let ticket = proposal.ticket;
        if let Some(stamp) = &ticket.key.draft {
            self.freeze_filename_draft(ticket.key.origin, stamp, request);
        }
        self.filesystem.running = Some(request);
        self.filesystem.history.push_back(Attempt {
            ticket: ticket.clone(),
            batch: proposal.batch.clone(),
            report: proposal.report,
            receipts: Vec::new(),
            publication: Vec::new(),
            warning: None,
            lsp_renames,
        });
        while self.filesystem.history.len() > 32
            || self
                .filesystem
                .history
                .iter()
                .map(|attempt| attempt.batch.steps.len())
                .sum::<usize>()
                > 1024
        {
            let removable = self.filesystem.history.iter().position(|attempt| {
                attempt.ticket.request != request
                    && attempt
                        .receipts
                        .iter()
                        .all(|receipt| !receipt.outcome.is_unconfirmed())
            });
            if let Some(index) = removable {
                self.filesystem.history.remove(index);
            } else {
                break;
            }
        }
        self.message = format!("applying {}", changes(proposal.batch.steps.len()));
        match self.tape.request("filesystem.apply", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_filesystem(FsEvent::Applied(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return;
            }
        }
        let tx = self.io.tx.clone();
        let local = self.filesystem.local().clone();
        let client = self.remote_client();
        let handle = worker::spawn_effect(
            "strop-fs-apply",
            move |outcome| {
                let _ = tx.send(IoEvent::Filesystem(Box::new(FsEvent::Applied(Box::new(
                    Completion { ticket, outcome },
                )))));
            },
            move |token| {
                Outcome::Success(crate::editor::namespace::execute(
                    &local,
                    &client,
                    &proposal.batch,
                    &HashMap::new(),
                    &token,
                ))
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub(crate) fn cancel_filesystem_review(&mut self) -> bool {
        // A staged will-rename proposal demotes back to pending so the
        // ordinary cancel path renders it; late replies find no stage.
        if let Some(stage) = self.filesystem.will_rename.take() {
            self.filesystem.pending = Some(stage.proposal);
        }
        if let Some(request) = self.filesystem.running.filter(|request| {
            self.filesystem.history.iter().any(|attempt| {
                attempt.ticket.request == *request && attempt.report == self.current()
            })
        }) {
            if let Some(handle) = self.worker_handles.remove(&request) {
                handle.cancel(worker::CancelReason::Dismissed);
            }
            self.message = "stopping; changes already made are kept".into();
            return true;
        }
        if self
            .filesystem
            .pending
            .as_ref()
            .is_none_or(|proposal| proposal.report != self.current())
        {
            return false;
        }
        let Some(proposal) = self.filesystem.pending.take() else {
            return false;
        };
        // Cancelling is closing the confirm buffer: the folder comes back
        // with its edits still pending.
        self.message = "nothing changed; the edits are still pending".into();
        if proposal.report == self.current() {
            self.close_buffer(true);
        }
        true
    }

    pub(crate) fn handle_filesystem(&mut self, event: FsEvent) {
        match event {
            FsEvent::Prepared(completion) => {
                let Completion { ticket, outcome } = *completion;
                if self.filesystem.preparing.as_ref() != Some(&ticket) {
                    return;
                }
                self.filesystem.preparing = None;
                self.worker_handles.remove(&ticket.request);
                match outcome {
                    Outcome::Success(PreparedFilesystem {
                        batch,
                        draft_targets,
                    }) => {
                        if self.docs.is_empty()
                            || self.current() != ticket.key.origin
                            || self.focus_epoch != ticket.key.focus
                            || self.buf().revision() != ticket.key.revision
                        {
                            // A dropped review must never be silent: the
                            // user asked for this operation and gets no
                            // other signal.
                            self.message = "the folder changed while preparing; :w again".into();
                            return;
                        }
                        if let Some(stamp) = &ticket.key.draft {
                            if !self.filename_draft_fresh(
                                ticket.key.origin,
                                ticket.key.revision,
                                stamp,
                            ) {
                                return;
                            }
                            if let Some(draft) = self
                                .docs
                                .get_mut(ticket.key.origin)
                                .and_then(|doc| doc.directory_metadata_mut())
                                .and_then(|source| source.draft.as_mut())
                            {
                                draft.targets = draft_targets.into_iter().collect();
                            }
                        }
                        if batch.steps.is_empty() && batch.refused.is_empty() {
                            self.message = "no changes to write".into();
                            return;
                        }
                        self.retire_text_review_for_filesystem();
                        let root = ticket.key.draft.as_ref().map(|stamp| stamp.root.clone());
                        let label = root
                            .as_ref()
                            .map_or_else(|| "changes".into(), |root| self.folder_label(root));
                        let (text, rows) = render::proposal(&label, root.as_ref(), &batch);
                        let mut buffer = strop_core::Buffer::from_text(&text);
                        buffer.name = Some(format!("changes in {label}"));
                        let Some(report) = self.open_temporary_output(buffer) else {
                            return; // message set; the batch stays unapplied
                        };
                        self.set_filesystem_review_rows(report, rows);
                        let count = batch.steps.len();
                        self.filesystem.pending = Some(Proposal {
                            ticket,
                            batch,
                            report,
                            view_revision: self.doc(report).buf.revision(),
                        });
                        self.message = format!("{} — y applies, q cancels", changes(count));
                    }
                    Outcome::Failed { failure, .. } => self.message = failure.message,
                    Outcome::Cancelled(_) => self.message = "nothing changed".into(),
                }
            }
            FsEvent::Applied(completion) => {
                let Completion { ticket, outcome } = *completion;
                if self.filesystem.running != Some(ticket.request) {
                    return;
                }
                self.worker_handles.remove(&ticket.request);
                let Some(index) = self
                    .filesystem
                    .history
                    .iter()
                    .position(|attempt| attempt.ticket == ticket)
                else {
                    return;
                };
                self.filesystem.history[index].warning = match &outcome {
                    Outcome::Failed {
                        failure,
                        partial: Some(_),
                    } => Some(failure.message.clone()),
                    _ => None,
                };
                let receipts = match outcome {
                    Outcome::Success(receipts)
                    | Outcome::Failed {
                        partial: Some(receipts),
                        ..
                    } if receipts.len() == self.filesystem.history[index].batch.steps.len()
                        && receipts.iter().enumerate().all(|(step, receipt)| {
                            receipt.step == step
                                && receipt.operation
                                    == self.filesystem.history[index].batch.steps[step]
                        }) =>
                    {
                        receipts
                    }
                    other => {
                        let detail = match other {
                            Outcome::Failed { failure, .. } => failure.message,
                            _ => "filesystem worker ended without its mutation receipt".into(),
                        };
                        self.filesystem.history[index]
                            .batch
                            .steps
                            .iter()
                            .enumerate()
                            .map(|(step, operation)| StepReceipt {
                                step,
                                operation: operation.clone(),
                                outcome: StepOutcome::Unconfirmed {
                                    detail: detail.clone(),
                                    observed_destination: None,
                                    recovery: None,
                                    publication: None,
                                },
                            })
                            .collect()
                    }
                };
                self.filesystem.running = None;
                self.filesystem.history[index].publication = receipts
                    .iter()
                    .map(|receipt| match &receipt.outcome {
                        StepOutcome::Committed {
                            destination_after, ..
                        } => destination_after.clone(),
                        StepOutcome::Unconfirmed {
                            observed_destination,
                            ..
                        } => observed_destination.clone(),
                        _ => None,
                    })
                    .collect();
                self.filesystem.history[index].receipts = receipts;
                for step in 0..self.filesystem.history[index].receipts.len() {
                    let receipt = self.filesystem.history[index].receipts[step].clone();
                    self.reconcile_filesystem_step(&receipt);
                }
                self.notify_did_rename(index);
                self.finish_filesystem_draft_attempt(index);
                self.settle_filesystem_attempt(index);
            }
            FsEvent::Verified(completion) => self.filesystem_verified(*completion),
        }
    }

    fn finish_filesystem_draft_attempt(&mut self, index: usize) {
        let attempt = &self.filesystem.history[index];
        let Some(stamp) = attempt.ticket.key.draft.clone() else {
            return;
        };
        let document = attempt.ticket.key.origin;
        let operation = attempt.ticket.request;
        let committed = attempt
            .receipts
            .iter()
            .any(|receipt| receipt.outcome.is_committed());
        let unconfirmed = attempt
            .receipts
            .iter()
            .any(|receipt| receipt.outcome.is_unconfirmed());
        self.finish_filename_draft(document, &stamp, operation, committed, unconfirmed);
    }

    /// After an apply (or a verification): every change made closes the
    /// confirm buffer and says so; otherwise the buffer shows each
    /// change's result, and an unconfirmed change is checked against the
    /// disk — no change is ever retried.
    pub(super) fn settle_filesystem_attempt(&mut self, index: usize) {
        let attempt = &self.filesystem.history[index];
        let report = attempt.report;
        let total = attempt.receipts.len();
        let done = attempt
            .receipts
            .iter()
            .filter(|receipt| receipt.outcome.is_committed())
            .count();
        let unconfirmed = attempt
            .receipts
            .iter()
            .position(|receipt| receipt.outcome.is_unconfirmed());
        let operation = attempt.ticket.request;
        if done == total {
            self.message = format!("{} applied", changes(done));
            if self.current() == report {
                self.close_buffer(true);
            }
            return;
        }
        let root = attempt.ticket.key.draft.as_ref().map(|stamp| &stamp.root);
        let label = root.map_or_else(|| "changes".into(), |root| self.folder_label(root));
        let (text, rows) = render::outcome(&label, root, &attempt.batch, &attempt.receipts);
        if self.docs.get(report).is_some() {
            if let Err(error) = self.replace_system(report, &text) {
                self.message = error.to_string();
                return;
            }
            self.set_filesystem_review_rows(report, rows);
        }
        self.message = format!("{done} of {} applied", changes(total));
        if let Some(step) = unconfirmed {
            if let Err(error) = self.verify_filesystem_step(operation, step) {
                self.message = error;
            }
        }
    }
}

/// `1 change`, `7 changes`.
pub(super) fn changes(count: usize) -> String {
    if count == 1 {
        "1 change".into()
    } else {
        format!("{count} changes")
    }
}
