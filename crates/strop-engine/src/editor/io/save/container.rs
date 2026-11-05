//! Explicit per-document container edit authority and frozen save attempts.

use crate::editor::containers::BoundWorker;
use crate::editor::document::DocumentSource;
use crate::editor::io::IoEvent;
use crate::editor::Editor;
use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, CancelReason, Completion, Outcome, Ticket, WorkerId};
use strop_worker_protocol::NamespaceIdentity;
use strop_workspace::operation::{FsFailureKind, PreparedOperation, StepOutcome, StepReceipt};
use strop_workspace::{FileTime, ObjectId, ResourceLocation};

mod store;
#[cfg(test)]
mod tests;
mod work;

use work::Work;

pub(crate) type FrozenStore =
    Arc<parking_lot::Mutex<Option<(NamespaceIdentity, PreparedOperation)>>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerWriteVersion {
    pub location: ResourceLocation,
    pub started_at: String,
    pub modified: FileTime,
    pub size: u64,
    pub identity: Option<ObjectId>,
    pub attributes: Option<[u8; 32]>,
    pub content: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ContainerWritePermit {
    id: WorkerId,
    version: ContainerWriteVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ContainerWriteAction {
    Enable,
    Save { close: bool },
    Verify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerWriteKey {
    document: DocumentId,
    revision: BufferRevision,
    location: ResourceLocation,
    started_at: String,
    permit: Option<WorkerId>,
    focus: u64,
    action: ContainerWriteAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContainerSaveError {
    Refused { kind: FsFailureKind, detail: String },
    Unconfirmed { detail: String },
}

#[derive(Serialize, Deserialize)]
pub enum ContainerWriteResult {
    Enabled(ContainerWriteVersion),
    Saved(ContainerWriteVersion),
    StoreUncertain {
        detail: String,
        receipt: Box<StepReceipt>,
    },
    Verified(VerifiedContainerWrite),
    Refused(ContainerSaveError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerifiedContainerWrite {
    Committed(Box<ContainerWriteVersion>),
    Unchanged,
}

struct Attempt {
    permit: WorkerId,
    revision: BufferRevision,
    before: ContainerWriteVersion,
    contents: Rope,
    close: bool,
    unconfirmed: bool,
    store: Option<Box<StepReceipt>>,
    namespace: Option<NamespaceIdentity>,
    prepared: FrozenStore,
}

#[derive(Default)]
pub(crate) struct ContainerWriteState {
    pending: HashMap<DocumentId, Ticket<ContainerWriteKey>>,
    attempts: HashMap<DocumentId, Attempt>,
    permits: HashMap<DocumentId, ContainerWritePermit>,
}

impl ContainerWriteState {
    pub(crate) fn permit_matches(
        &self,
        document: DocumentId,
        root: &ResourceLocation,
        started_at: &str,
    ) -> bool {
        self.permits.get(&document).is_some_and(|permit| {
            permit.version.started_at == started_at
                && permit.version.location.filesystem == root.filesystem
                && permit.version.location.path.starts_with(&root.path)
        })
    }

    pub(crate) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn pending_for(&self, document: DocumentId) -> bool {
        self.pending.contains_key(&document)
    }

    pub(crate) fn unconfirmed_revision(&self, document: DocumentId) -> Option<BufferRevision> {
        self.attempts
            .get(&document)
            .filter(|attempt| attempt.unconfirmed)
            .map(|attempt| attempt.revision)
    }

    pub(crate) fn unconfirmed(&self) -> usize {
        self.attempts
            .values()
            .filter(|attempt| attempt.unconfirmed)
            .count()
    }
}

impl Editor {
    fn container_source(&self, document: DocumentId) -> Option<ResourceLocation> {
        self.docs
            .get(document)
            .and_then(|document| match &document.source {
                DocumentSource::Container { container, path } => Some(ResourceLocation {
                    filesystem: strop_workspace::Filesystem::Container(container.clone()),
                    path: path.clone(),
                }),
                _ => None,
            })
    }

    fn container_worker_for(
        &self,
        location: &ResourceLocation,
        started_at: Option<&str>,
    ) -> Result<Arc<BoundWorker>, String> {
        let strop_workspace::Filesystem::Container(id) = &location.filesystem else {
            return Err("open a container regular file first".into());
        };
        let identity = self
            .containers
            .attached
            .get(id.as_str())
            .ok_or("attach this container incarnation first")?;
        if started_at.is_some_and(|expected| expected != identity.started_at) {
            return Err("container was recycled; reattach and re-enable editing".into());
        }
        self.containers.workers.get(identity).ok_or_else(|| {
            "container has no admitted worker; use :container-worker first, then :container-edit"
                .into()
        })
    }

    pub(crate) fn enable_container_edit(&mut self) -> Result<(), String> {
        let document = self.current();
        if self.filesystem_blocks_document(document) {
            return Err("filesystem operation pending or unconfirmed; verify its receipt before edit admission".into());
        }
        if self.io.container_writes.pending.contains_key(&document) {
            return Err("container write operation already pending".into());
        }
        if self.io.container_writes.permits.contains_key(&document) {
            return Err("container editing is already enabled for this document".into());
        }
        if self.remote_refresh_pending(document) {
            return Err("container refresh is pending".into());
        }
        let location = self
            .container_source(document)
            .ok_or("open a container regular file first")?;
        let worker = self.container_worker_for(&location, None)?;
        let Some(attached) = self.containers.attached.get(match &location.filesystem {
            strop_workspace::Filesystem::Container(id) => id.as_str(),
            _ => unreachable!(),
        }) else {
            panic!("worker lookup retained the attached identity");
        };
        let started_at = attached.started_at.clone();
        let contents = self.doc(document).buf.snapshot();
        let revision = self.doc(document).buf.revision();
        self.start_container_write(
            document,
            revision,
            None,
            Work::Enable {
                location,
                started_at,
                contents,
                worker,
            },
        )
    }

    pub(crate) fn request_container_save(
        &mut self,
        document: DocumentId,
        target: Option<std::path::PathBuf>,
        force: bool,
        close: bool,
    ) -> bool {
        match self.prepare_container_save(document, target, force, close) {
            Ok(()) => true,
            Err(error) => {
                self.message = error.clone();
                let revision = self
                    .io
                    .container_writes
                    .attempts
                    .get(&document)
                    .filter(|attempt| attempt.unconfirmed)
                    .map(|attempt| attempt.revision)
                    .or_else(|| self.docs.get(document).map(|doc| doc.buf.revision()))
                    .unwrap_or_default();
                if self
                    .io
                    .container_writes
                    .attempts
                    .get(&document)
                    .is_some_and(|attempt| attempt.unconfirmed)
                {
                    self.note_unconfirmed_save(document, false, revision, error);
                } else {
                    self.note_save_failure(
                        document, false, revision, None, force, close, false, error,
                    );
                }
                false
            }
        }
    }

    fn prepare_container_save(
        &mut self,
        document: DocumentId,
        target: Option<std::path::PathBuf>,
        force: bool,
        close: bool,
    ) -> Result<(), String> {
        if target.is_some() {
            return Err("container save-as is unsupported; no local fallback".into());
        }
        if self.filesystem_blocks_document(document) {
            return Err("filesystem operation pending or unconfirmed; verify before saving".into());
        }
        if self.io.container_writes.pending.contains_key(&document) {
            return Err("container write operation already pending".into());
        }
        if self.remote_refresh_pending(document) {
            return Err("container refresh is pending".into());
        }
        if self
            .io
            .container_writes
            .attempts
            .get(&document)
            .is_some_and(|attempt| attempt.unconfirmed)
        {
            return self.verify_container_save_document(document);
        }
        let location = self
            .container_source(document)
            .ok_or("container directories cannot be saved")?;
        let permit = self
            .io
            .container_writes
            .permits
            .get(&document)
            .cloned()
            .ok_or("container file is read-only; use :container-edit first")?;
        let worker = self.container_worker_for(&location, Some(&permit.version.started_at))?;
        if permit.version.location != location {
            return Err("container write permit belongs to another file".into());
        }
        if self.doc(document).buf.readonly && !force {
            return Err(
                "readonly buffer; container write authority remains available via :w!".into(),
            );
        }
        let before = permit.version.clone();
        let contents = self.doc(document).buf.snapshot();
        let revision = self.doc(document).buf.revision();
        let prepared = FrozenStore::default();
        self.io.container_writes.attempts.insert(
            document,
            Attempt {
                permit: permit.id,
                revision,
                before: before.clone(),
                contents: contents.clone(),
                close,
                unconfirmed: false,
                store: None,
                namespace: None,
                prepared: Arc::clone(&prepared),
            },
        );
        let result = self.start_container_write_with_action(
            document,
            revision,
            Some(permit.id),
            ContainerWriteAction::Save { close },
            Work::Save {
                before,
                contents,
                worker,
                prepared,
            },
        );
        if result.is_err() {
            self.io.container_writes.attempts.remove(&document);
        }
        result
    }

    fn verify_container_save_document(&mut self, document: DocumentId) -> Result<(), String> {
        if self.io.container_writes.pending.contains_key(&document) {
            return Err("container write operation already pending".into());
        }
        let attempt = self
            .io
            .container_writes
            .attempts
            .get(&document)
            .filter(|attempt| attempt.unconfirmed)
            .ok_or("no unconfirmed container save to verify")?;
        let permit = self
            .io
            .container_writes
            .permits
            .get(&document)
            .filter(|permit| permit.id == attempt.permit)
            .ok_or("container write permit was revoked")?;
        let worker =
            self.container_worker_for(&attempt.before.location, Some(&attempt.before.started_at))?;
        let work = Work::Verify {
            before: attempt.before.clone(),
            contents: attempt.contents.clone(),
            store: attempt.store.clone(),
            namespace: attempt.namespace.clone(),
            worker,
        };
        let revision = attempt.revision;
        let permit = permit.id;
        self.start_container_write(document, revision, Some(permit), work)
    }

    fn start_container_write(
        &mut self,
        document: DocumentId,
        revision: BufferRevision,
        permit: Option<WorkerId>,
        work: Work,
    ) -> Result<(), String> {
        let action = work.action();
        self.start_container_write_with_action(document, revision, permit, action, work)
    }

    fn start_container_write_with_action(
        &mut self,
        document: DocumentId,
        revision: BufferRevision,
        permit: Option<WorkerId>,
        action: ContainerWriteAction,
        work: Work,
    ) -> Result<(), String> {
        let request = self.worker_ids.allocate().map_err(|error| error.message)?;
        let ticket = Ticket {
            request,
            key: ContainerWriteKey {
                document,
                revision,
                location: work.location().clone(),
                started_at: work.started_at().to_owned(),
                permit,
                focus: self.focus_epoch,
                action,
            },
        };
        self.io
            .container_writes
            .pending
            .insert(document, ticket.clone());
        self.message = match action {
            ContainerWriteAction::Enable => "checking container edit authority",
            ContainerWriteAction::Save { .. } => "saving in container",
            ContainerWriteAction::Verify => "verifying container save",
        }
        .into();
        match self.tape.request(
            "container.write",
            &(&ticket, work.before(), work.contents().len_bytes()),
        ) {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => {
                self.io.container_writes.pending.remove(&document);
                return Err(format!("container write replay diverged: {error}"));
            }
        }
        let sender = self.io.tx.clone();
        let handle = worker::spawn(
            "container-write",
            move |outcome| {
                let _ = sender.send(IoEvent::ContainerWrite(Box::new(Completion {
                    ticket,
                    outcome,
                })));
            },
            move |token| Outcome::Success(work.execute(&token)),
        );
        self.worker_handles.insert(request, handle);
        Ok(())
    }

    pub(crate) fn container_write_done(
        &mut self,
        completion: Completion<ContainerWriteKey, ContainerWriteResult>,
    ) {
        let document = completion.ticket.key.document;
        if self.io.container_writes.pending.get(&document) != Some(&completion.ticket) {
            return;
        }
        self.io.container_writes.pending.remove(&document);
        self.worker_handles.remove(&completion.ticket.request);
        let request = completion.ticket.request;
        let key = completion.ticket.key;
        if matches!(key.action, ContainerWriteAction::Save { .. }) {
            if let Some(attempt) = self.io.container_writes.attempts.get_mut(&document) {
                if let Some((namespace, operation)) = attempt.prepared.lock().take() {
                    attempt.namespace = Some(namespace);
                    if !matches!(
                        &completion.outcome,
                        Outcome::Success(
                            ContainerWriteResult::Saved(_)
                                | ContainerWriteResult::StoreUncertain { .. }
                        )
                    ) {
                        attempt.store = Some(Box::new(StepReceipt {
                            step: 0,
                            operation,
                            outcome: StepOutcome::Unconfirmed {
                                detail: "container Store reply lost after preparation".into(),
                                observed_destination: None,
                                recovery: None,
                                publication: None,
                            },
                        }));
                    }
                }
            }
        }
        let worker_live = self
            .container_worker_for(&key.location, Some(&key.started_at))
            .is_ok();
        let valid = worker_live
            && self.container_source(document).as_ref() == Some(&key.location)
            && match key.action {
                ContainerWriteAction::Enable => {
                    !self.io.container_writes.permits.contains_key(&document)
                        && self.doc(document).buf.revision() == key.revision
                }
                _ => self
                    .io
                    .container_writes
                    .permits
                    .get(&document)
                    .is_some_and(|permit| Some(permit.id) == key.permit),
            };
        if !valid {
            if key.action == ContainerWriteAction::Enable {
                self.message =
                    "container edit admission cancelled: snapshot or incarnation changed".into();
            }
            if matches!(key.action, ContainerWriteAction::Save { .. }) {
                self.container_write_uncertain(
                    document,
                    "container save result discarded: source authority changed; outcome unconfirmed"
                        .into(),
                );
            }
            return;
        }
        match completion.outcome {
            Outcome::Success(ContainerWriteResult::Enabled(version))
                if key.action == ContainerWriteAction::Enable
                    && version.location == key.location
                    && version.started_at == key.started_at =>
            {
                self.io.container_writes.permits.insert(
                    document,
                    ContainerWritePermit {
                        id: request,
                        version,
                    },
                );
                self.doc_mut(document).buf.clear_readonly();
                self.message =
                    "container editing enabled (conditional saves still protect the baseline)"
                        .into();
            }
            Outcome::Success(ContainerWriteResult::Saved(version))
                if matches!(key.action, ContainerWriteAction::Save { .. }) =>
            {
                return self.accept_container_receipt(key, version);
            }
            Outcome::Success(ContainerWriteResult::StoreUncertain { detail, receipt })
                if matches!(key.action, ContainerWriteAction::Save { .. }) =>
            {
                if let Some(attempt) = self.io.container_writes.attempts.get_mut(&document) {
                    attempt.store = Some(receipt);
                }
                self.container_write_uncertain(document, detail);
            }
            Outcome::Success(ContainerWriteResult::Verified(
                VerifiedContainerWrite::Committed(version),
            )) if key.action == ContainerWriteAction::Verify => {
                return self.accept_container_receipt(key, *version);
            }
            Outcome::Success(ContainerWriteResult::Verified(VerifiedContainerWrite::Unchanged))
                if key.action == ContainerWriteAction::Verify =>
            {
                self.io.container_writes.attempts.remove(&document);
                self.resolve_save_feedback(document, false);
                self.message =
                    "container original is unchanged; edits remain unsaved; :w retries".into();
            }
            Outcome::Success(ContainerWriteResult::Refused(error)) => match error {
                ContainerSaveError::Unconfirmed { detail } => {
                    self.container_write_uncertain(document, detail)
                }
                ContainerSaveError::Refused { kind, detail } => {
                    if key.action == ContainerWriteAction::Verify && kind != FsFailureKind::Conflict
                    {
                        self.container_write_uncertain(
                            document,
                            format!("verification failed ({kind:?}): {detail}"),
                        );
                    } else {
                        if key.action != ContainerWriteAction::Verify
                            || kind == FsFailureKind::Conflict
                        {
                            self.io.container_writes.attempts.remove(&document);
                        }
                        self.message = format!("container write failed ({kind:?}): {detail}");
                        self.note_save_failure(
                            document,
                            false,
                            key.revision,
                            None,
                            false,
                            matches!(key.action, ContainerWriteAction::Save { close: true }),
                            false,
                            self.message.clone(),
                        );
                    }
                }
            },
            Outcome::Cancelled(_) if key.action == ContainerWriteAction::Enable => {
                self.message = "container edit admission cancelled".into()
            }
            Outcome::Failed { failure, .. } if key.action == ContainerWriteAction::Enable => {
                self.message = format!("container edit admission failed: {}", failure.message);
                self.note_save_failure(
                    document,
                    false,
                    key.revision,
                    None,
                    false,
                    false,
                    false,
                    self.message.clone(),
                );
            }
            Outcome::Cancelled(_) => self.container_write_uncertain(
                document,
                "cancelled; container outcome is unconfirmed — :w verifies".into(),
            ),
            Outcome::Failed { failure, .. } => self.container_write_uncertain(
                document,
                format!(
                    "{}; container outcome unconfirmed — :w verifies",
                    failure.message
                ),
            ),
            Outcome::Success(_) => self.container_write_uncertain(
                document,
                "container write result does not match its request".into(),
            ),
        }
        if matches!(key.action, ContainerWriteAction::Save { .. }) {
            self.finish_save_feedback(document);
        }
    }

    fn container_write_uncertain(&mut self, document: DocumentId, detail: String) {
        if let Some(attempt) = self.io.container_writes.attempts.get_mut(&document) {
            attempt.unconfirmed = true;
        }
        let revision = self
            .io
            .container_writes
            .attempts
            .get(&document)
            .map(|attempt| attempt.revision)
            .or_else(|| self.docs.get(document).map(|doc| doc.buf.revision()))
            .unwrap_or_default();
        self.message =
            format!("container save outcome unconfirmed: {detail}; :w verifies before any rewrite");
        self.note_unconfirmed_save(document, false, revision, self.message.clone());
        self.finish_save_feedback(document);
    }

    fn accept_container_receipt(&mut self, key: ContainerWriteKey, version: ContainerWriteVersion) {
        if version.location != key.location || version.started_at != key.started_at {
            self.container_write_uncertain(
                key.document,
                "receipt belongs to another container file".into(),
            );
            return;
        }
        self.resolve_save_feedback(key.document, false);
        let current = {
            let Some(permit) = self.io.container_writes.permits.get_mut(&key.document) else {
                return;
            };
            if Some(permit.id) != key.permit {
                return;
            }
            permit.version = version;
            let mut document = self.doc_mut(key.document);
            document.external_change = false;
            document.buf.acknowledge_saved_revision(key.revision)
        };
        let close = match key.action {
            ContainerWriteAction::Save { close } => close,
            ContainerWriteAction::Verify => self
                .io
                .container_writes
                .attempts
                .get(&key.document)
                .is_some_and(|attempt| attempt.close),
            ContainerWriteAction::Enable => false,
        };
        self.io.container_writes.attempts.remove(&key.document);
        self.message = if current {
            "written in container"
        } else {
            "container snapshot written; newer edits remain unsaved"
        }
        .into();
        self.finish_save_feedback(key.document);
        if current
            && close
            && !self.docs.is_empty()
            && self.current() == key.document
            && self.focus_epoch == key.focus
        {
            self.close_pane_or_buffer(false);
        }
    }

    pub(crate) fn revoke_container_write(&mut self, document: DocumentId) {
        self.io.container_writes.permits.remove(&document);
        self.io.container_writes.attempts.remove(&document);
        if let Some(ticket) = self.io.container_writes.pending.remove(&document) {
            if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                handle.cancel(CancelReason::OwnerClosed);
            }
        }
        if let Some(document) = self.docs.get_mut(document) {
            if matches!(document.source, DocumentSource::Container { .. }) {
                document
                    .buf
                    .set_readonly(strop_core::ReadonlyReason::Container);
            }
        }
    }

    pub(crate) fn cancel_container_write(&mut self, document: DocumentId) {
        if let Some(ticket) = self.io.container_writes.pending.get(&document).cloned() {
            if ticket.key.action == ContainerWriteAction::Enable {
                self.io.container_writes.pending.remove(&document);
                self.message = "container edit admission cancelled".into();
            }
            if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                handle.cancel(CancelReason::Dismissed);
            }
        }
    }

    pub(crate) fn container_write_pending(&self, request: WorkerId) -> bool {
        self.io.container_writes.pending.values().any(|ticket| {
            ticket.request == request
                && matches!(
                    ticket.key.action,
                    ContainerWriteAction::Save { .. } | ContainerWriteAction::Verify
                )
        })
    }

    pub fn container_write_status(&self) -> Option<&'static str> {
        if self.docs.is_empty() {
            return None;
        }
        let pending = self.io.container_writes.pending.get(&self.current())?;
        Some(match pending.key.action {
            ContainerWriteAction::Enable => "checking edit",
            ContainerWriteAction::Save { .. } => "saving",
            ContainerWriteAction::Verify => "verifying save",
        })
    }

    pub(crate) fn container_unconfirmed_save_attempts(
        &self,
    ) -> impl Iterator<Item = (DocumentId, BufferRevision, &ResourceLocation)> {
        self.io
            .container_writes
            .attempts
            .iter()
            .filter(|(_, attempt)| attempt.unconfirmed)
            .map(|(document, attempt)| (*document, attempt.revision, &attempt.before.location))
    }
}
