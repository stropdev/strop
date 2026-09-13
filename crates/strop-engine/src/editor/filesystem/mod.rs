//! Filesystem proposal and mutation lifetime is independent of its originating view.
mod actions;
mod commands;
pub(crate) mod draft;
mod prepare;
mod reconcile;
mod recovery;
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
    pub open_created: bool,
    pub intents: std::sync::Arc<Vec<OperationIntent>>,
    pub recovery: Option<recovery::RecoveryGuard>,
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
    copies: HashMap<usize, ropey::Rope>,
    report: DocumentId,
    view_revision: BufferRevision,
}
pub(crate) struct Attempt {
    ticket: Ticket<FsKey>,
    batch: strop_fs::batch::PreparedBatch,
    report: DocumentId,
    receipts: Vec<StepReceipt>,
    apply_focus: u64,
    /// Original publication evidence survives later verification observations.
    publication: Vec<Option<strop_workspace::Observation>>,
    warning: Option<String>,
}
pub(crate) struct FsState {
    environment: strop_fs::Environment,
    preparing: Option<Ticket<FsKey>>,
    preparing_copies: HashMap<ResourceLocation, ropey::Rope>,
    pending: Option<Proposal>,
    running: Option<WorkerId>,
    verifying: Option<Ticket<VerifyKey>>,
    history: VecDeque<Attempt>,
    actions: Option<actions::ActionOwner>,
    reports: std::collections::HashSet<DocumentId>,
}
impl Default for FsState {
    fn default() -> Self {
        Self {
            environment: strop_fs::Environment::capture(),
            preparing: None,
            preparing_copies: HashMap::new(),
            pending: None,
            running: None,
            verifying: None,
            history: VecDeque::new(),
            actions: None,
            reports: Default::default(),
        }
    }
}
impl FsState {
    pub(crate) fn pending(&self) -> bool {
        self.preparing.is_some() || self.running.is_some() || self.verifying.is_some()
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
            self.filesystem.preparing_copies.clear();
        }
    }
    pub(crate) fn retire_filesystem_review(&mut self, reason: &str) {
        if let Some(proposal) = self.filesystem.pending.take() {
            if self.docs.get(proposal.report).is_some() {
                let text = format!(
                    "filesystem review {} — {reason}\nnothing applied\n",
                    proposal.ticket.request.get()
                );
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
        if self
            .docs
            .get(proposal.report)
            .is_none_or(|document| document.buf.revision() != proposal.view_revision)
        {
            self.filesystem.pending = Some(proposal);
            self.message = "filesystem review changed; prepare a new review".into();
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
            self.message = "filename draft changed; cancel this review and prepare it again".into();
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
            self.message = "unconfirmed operation history reached its bound; verify before admitting more work".into();
            return true;
        }
        if proposal.batch.steps.is_empty() {
            self.message = "filesystem review has no applicable steps".into();
            self.filesystem.pending = Some(proposal);
            return true;
        }
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
            apply_focus: self.focus_epoch,
            publication: Vec::new(),
            warning: None,
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
        self.message = format!(
            "applying {} reviewed filesystem step(s)",
            proposal.batch.steps.len()
        );
        match self.tape.request("filesystem.apply", &ticket) {
            Ok(false) => return true,
            Ok(true) => {}
            Err(error) => {
                self.handle_filesystem(FsEvent::Applied(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return true;
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn_effect(
            "strop-fs-apply",
            move |outcome| {
                let _ = tx.send(IoEvent::Filesystem(Box::new(FsEvent::Applied(Box::new(
                    Completion { ticket, outcome },
                )))));
            },
            move |token| {
                Outcome::Success(strop_fs::batch::execute(
                    &proposal.batch,
                    &proposal.copies,
                    &token,
                ))
            },
        );
        self.worker_handles.insert(request, handle);
        true
    }

    pub(crate) fn cancel_filesystem_review(&mut self) -> bool {
        if let Some(request) = self.filesystem.running.filter(|request| {
            self.filesystem.history.iter().any(|attempt| {
                attempt.ticket.request == *request && attempt.report == self.current()
            })
        }) {
            if let Some(handle) = self.worker_handles.remove(&request) {
                handle.cancel(worker::CancelReason::Dismissed);
            }
            self.message = "filesystem cancellation requested; the receipt will retain committed or unconfirmed outcomes".into();
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
        let draft_return = proposal
            .ticket
            .key
            .draft
            .as_ref()
            .filter(|stamp| {
                self.filename_draft(proposal.ticket.key.origin)
                    .is_some_and(|draft| draft.id == stamp.id)
            })
            .map(|_| {
                (
                    proposal.ticket.key.origin,
                    self.doc(proposal.report).return_point().cloned(),
                )
            });
        let text = format!(
            "filesystem review {} — cancelled\nnothing applied\n",
            proposal.ticket.request.get()
        );
        if let Err(error) = self.replace_system(proposal.report, &text) {
            self.message = error.to_string();
        } else {
            self.message = "filesystem review cancelled; nothing applied".into();
        }
        if let Some((document, point)) = draft_return {
            if let Some(point) = point.filter(|point| point.document == document) {
                self.jump_to(point);
            } else {
                self.switch_to(document);
            }
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
                let copies = std::mem::take(&mut self.filesystem.preparing_copies);
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
                            self.message = "filename draft has no filesystem changes".into();
                            return;
                        }
                        self.retire_text_review_for_filesystem();
                        let (text, rows) = render::proposal(ticket.request, &batch);
                        let mut buffer = strop_core::Buffer::from_text(&text);
                        buffer.name = Some("filesystem change review".into());
                        let report = self.open_temporary_output(buffer);
                        self.set_filesystem_review_rows(report, rows);
                        let copies = batch
                            .steps
                            .iter()
                            .enumerate()
                            .filter(|(_, step)| {
                                step.intent.kind == OperationKind::Copy
                                    && step.intent.copy_version == CopyVersion::Buffer
                            })
                            .filter_map(|(index, step)| {
                                step.intent
                                    .source
                                    .as_ref()
                                    .and_then(|source| copies.get(source))
                                    .map(|rope| (index, rope.clone()))
                            })
                            .collect();
                        self.filesystem.pending = Some(Proposal {
                            ticket,
                            copies,
                            batch,
                            report,
                            view_revision: self.doc(report).buf.revision(),
                        });
                        self.message =
                            "review exact filesystem steps; :apply-change or :cancel-change".into();
                    }
                    Outcome::Failed { failure, .. } => self.message = failure.message,
                    Outcome::Cancelled(_) => {
                        self.message = "filesystem preparation cancelled; nothing applied".into()
                    }
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
                self.publish_filesystem_receipt(index);
                self.finish_filesystem_draft_attempt(index);
                self.open_created_filesystem_resource(index);
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

    fn open_created_filesystem_resource(&mut self, index: usize) {
        let attempt = &self.filesystem.history[index];
        if !attempt.ticket.key.open_created
            || self.docs.is_empty()
            || self.current() != attempt.report
            || self.focus_epoch != attempt.apply_focus
        {
            return;
        }
        let target = attempt
            .receipts
            .iter()
            .rev()
            .find(|receipt| {
                receipt.outcome.is_committed()
                    && matches!(
                        receipt.operation.intent.kind,
                        OperationKind::CreateFile | OperationKind::CreateDirectory
                    )
            })
            .and_then(|receipt| {
                receipt.operation.destination.as_ref().map(|destination| {
                    (destination.location.clone(), receipt.operation.intent.kind)
                })
            });
        if let Some((location, kind)) = target {
            match crate::files::FileTarget::from_location(&location) {
                Ok(target) => {
                    self.push_jump();
                    self.request_target(
                        target,
                        if kind == OperationKind::CreateDirectory {
                            super::io::OpenIntent::Browse
                        } else {
                            super::io::OpenIntent::Switch { readonly: false }
                        },
                    );
                }
                Err(error) => {
                    self.message = format!("creation committed, but opening failed: {error}")
                }
            }
        }
    }

    fn publish_filesystem_receipt(&mut self, index: usize) {
        let attempt = &self.filesystem.history[index];
        let report = attempt.report;
        let (text, rows) = render::receipt(
            attempt.ticket.request,
            &attempt.batch,
            &attempt.receipts,
            attempt.warning.as_deref(),
        );
        let committed = attempt
            .receipts
            .iter()
            .filter(|receipt| receipt.outcome.is_committed())
            .count();
        let unknown = attempt
            .receipts
            .iter()
            .filter(|receipt| receipt.outcome.is_unconfirmed())
            .count();
        if self.docs.get(report).is_some() {
            if let Err(error) = self.replace_system(report, &text) {
                self.message = error.to_string();
                return;
            }
            self.set_filesystem_review_rows(report, rows);
        }
        self.message = format!("filesystem: {committed} committed, {unknown} unconfirmed; :fs operations retains receipts");
    }
}
