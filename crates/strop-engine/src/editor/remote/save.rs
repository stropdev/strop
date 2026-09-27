//! Remote write authority is per document incarnation. Attempts outlive their
//! cancellable worker so a lost receipt can be reconciled without losing edits.
use super::RemoteEvent;
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
use strop_workspace::operation::{PreparedOperation, StepOutcome, StepReceipt};
use strop_workspace::{FileTime, ObjectId, RemoteFile};

mod error;
pub(crate) mod store;

pub use error::{RefusalKind, RemoteSaveError};

/// One admitted remote document's worker Store baseline. Relocation
/// retains identity and stored-byte evidence but never transfers a
/// write permit to a new name without fresh admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteVersion {
    file: RemoteFile,
    modified: FileTime,
    attributes: Option<[u8; 32]>,
    size: u64,
    identity: Option<ObjectId>,
    content: [u8; 32],
}

impl WriteVersion {
    fn file(&self) -> &RemoteFile {
        &self.file
    }
    fn size(&self) -> strop_remote::RemoteSize {
        strop_remote::RemoteSize::new(self.size)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerifiedWrite {
    Committed(WriteVersion),
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WritePermit {
    id: WorkerId,
    version: WriteVersion,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum WriteAction {
    Enable,
    Save { close: bool },
    Verify,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWriteKey {
    document: DocumentId,
    revision: BufferRevision,
    file: RemoteFile,
    permit: Option<WorkerId>,
    focus: u64,
    action: WriteAction,
}
#[derive(Serialize, Deserialize)]
pub enum RemoteWriteResult {
    Enabled(WriteVersion),
    Saved(WriteVersion),
    /// A worker-routed save whose outcome is unconfirmed; the frozen
    /// receipt crosses so the attempt retains its verify evidence.
    StoreUncertain {
        detail: String,
        receipt: Box<StepReceipt>,
    },
    Verified(VerifiedWrite),
    Refused(RemoteSaveError),
}

/// The worker publishes the prepared read-only verification evidence
/// before apply; the editor retains it even if its reply is lost.
type FrozenStore = Arc<parking_lot::Mutex<Option<(NamespaceIdentity, PreparedOperation)>>>;
#[derive(Default)]
pub(crate) struct WriteState {
    pending: HashMap<DocumentId, Ticket<RemoteWriteKey>>,
    attempts: HashMap<DocumentId, Attempt>,
    /// Stored-byte evidence retained after relocation; never an active permit.
    relocations: HashMap<DocumentId, WriteVersion>,
}
struct Attempt {
    permit: WorkerId,
    revision: BufferRevision,
    before: WriteVersion,
    contents: Rope,
    unconfirmed: bool,
    /// A worker-routed attempt's frozen receipt for `:remote verify`.
    store: Option<Box<StepReceipt>>,
    namespace: Option<NamespaceIdentity>,
    prepared: FrozenStore,
}
enum Work {
    Enable {
        file: RemoteFile,
        contents: Rope,
        baseline: Option<WriteVersion>,
    },
    Save {
        before: WriteVersion,
        contents: Rope,
        close: bool,
        prepared: FrozenStore,
    },
    Verify {
        before: WriteVersion,
        contents: Rope,
        store: Option<Box<StepReceipt>>,
        namespace: Option<NamespaceIdentity>,
    },
}
impl Work {
    fn action(&self) -> WriteAction {
        match self {
            Self::Enable { .. } => WriteAction::Enable,
            Self::Save { close, .. } => WriteAction::Save { close: *close },
            Self::Verify { .. } => WriteAction::Verify,
        }
    }
    fn file(&self) -> &RemoteFile {
        match self {
            Self::Enable { file, .. } => file,
            Self::Save { before, .. } | Self::Verify { before, .. } => before.file(),
        }
    }
    fn before(&self) -> Option<&WriteVersion> {
        match self {
            Self::Enable { baseline, .. } => baseline.as_ref(),
            Self::Save { before, .. } | Self::Verify { before, .. } => Some(before),
        }
    }
    fn contents(&self) -> &Rope {
        match self {
            Self::Enable { contents, .. }
            | Self::Save { contents, .. }
            | Self::Verify { contents, .. } => contents,
        }
    }
    fn execute(
        self,
        workers: &super::workers::RemoteWorkers,
        token: &worker::CancelToken,
    ) -> RemoteWriteResult {
        match self {
            Self::Enable {
                file,
                contents,
                baseline,
            } => enable(file, contents, baseline, workers, token),
            Self::Save {
                before,
                contents,
                prepared,
                ..
            } => {
                let Some(lease) = workers.get(before.file().endpoint()) else {
                    return RemoteWriteResult::Refused(RemoteSaveError::Refused {
                        kind: RefusalKind::Io,
                        detail:
                            "the admitted worker lease is gone; never retried through another path"
                                .into(),
                    });
                };
                match store::save(&lease, &before, &contents, &prepared, token) {
                    Ok(store::WorkerSave::Saved(version)) => RemoteWriteResult::Saved(version),
                    Ok(store::WorkerSave::Unconfirmed { detail, receipt }) => {
                        RemoteWriteResult::StoreUncertain { detail, receipt }
                    }
                    Err(error) => RemoteWriteResult::Refused(error),
                }
            }
            Self::Verify {
                before,
                store,
                namespace,
                ..
            } => {
                let Some(lease) = workers.get(before.file().endpoint()) else {
                    return RemoteWriteResult::Refused(RemoteSaveError::Unconfirmed {
                        detail: "the admitted worker lease is gone; outcome stays unconfirmed"
                            .into(),
                    });
                };
                let (Some(receipt), Some(namespace)) = (store, namespace) else {
                    return RemoteWriteResult::Refused(RemoteSaveError::Unconfirmed {
                        detail: "no frozen store attempt or namespace to verify".into(),
                    });
                };
                match store::verify(&lease, &before, *receipt, namespace, token) {
                    Ok(store::WorkerVerification::Committed(version)) => {
                        RemoteWriteResult::Verified(VerifiedWrite::Committed(version))
                    }
                    Ok(store::WorkerVerification::Unchanged) => {
                        RemoteWriteResult::Verified(VerifiedWrite::Unchanged)
                    }
                    Err(error) => RemoteWriteResult::Refused(error),
                }
            }
        }
    }
}

/// Edit admission: the user's `:remote edit` authorizes the worker
/// deployment and a fresh Store baseline. A restricted/SFTP-only host
/// remains read-only; no Python, SFTP or local write fallback exists.
fn enable(
    file: RemoteFile,
    contents: Rope,
    baseline: Option<WriteVersion>,
    workers: &super::workers::RemoteWorkers,
    token: &worker::CancelToken,
) -> RemoteWriteResult {
    workers
        .admit(file.endpoint(), "remote document save", token)
        .map_err(store::map_failure)
        .and_then(|lease| store::prepare_edit(&lease, &file, &contents, baseline.as_ref(), token))
        .map(RemoteWriteResult::Enabled)
        .unwrap_or_else(RemoteWriteResult::Refused)
}
impl WriteState {
    pub(super) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

impl Editor {
    pub(super) fn enable_remote_edit(&mut self) -> Result<(), String> {
        let document = self.current();
        if self.filesystem_blocks_document(document) {
            return Err("filesystem operation pending or unconfirmed; verify its receipt before edit admission".into());
        }
        if self.remote_refresh_pending(document) {
            return Err("remote refresh is pending".into());
        }
        let source = self
            .cur()
            .remote_metadata()
            .ok_or("open a remote regular file first")?;
        if source.write.is_some() {
            return Err("remote editing is already enabled".into());
        }
        if !self.remote_window_complete() {
            return Err("remote editing requires a complete, non-following snapshot".into());
        }
        if self.remote.writes.pending.contains_key(&document) {
            return Err("remote write operation already pending".into());
        }
        let work = Work::Enable {
            file: source.file.clone(),
            contents: self.buf().snapshot(),
            baseline: self.remote.writes.relocations.get(&document).cloned(),
        };
        self.start_remote_write(document, self.buf().revision(), None, work)
    }

    pub(crate) fn request_remote_save(
        &mut self,
        document: DocumentId,
        target: Option<std::path::PathBuf>,
        force: bool,
        close: bool,
    ) -> bool {
        match self.prepare_remote_save(document, target, force, close) {
            Ok(()) => true,
            Err(error) => {
                self.message = error;
                false
            }
        }
    }
    fn prepare_remote_save(
        &mut self,
        document: DocumentId,
        target: Option<std::path::PathBuf>,
        force: bool,
        close: bool,
    ) -> Result<(), String> {
        if self.filesystem_blocks_document(document) {
            return Err(
                "filesystem operation pending or unconfirmed; verify its receipt before saving"
                    .into(),
            );
        }
        if target.is_some() {
            return Err("remote save-as is unsupported; no local fallback".into());
        }
        if self.remote_refresh_pending(document) {
            return Err("remote refresh is pending".into());
        }
        if self.remote.writes.pending.contains_key(&document) {
            return Err("remote write operation already pending".into());
        }
        if self
            .remote
            .writes
            .attempts
            .get(&document)
            .is_some_and(|attempt| attempt.unconfirmed)
        {
            return Err(
                "remote save outcome is unconfirmed; use :remote verify before another save".into(),
            );
        }
        let doc = self.docs.get(document).ok_or("no such remote document")?;
        let source = doc
            .remote_metadata()
            .ok_or("remote directories cannot be saved")?;
        let permit = source
            .write
            .as_ref()
            .ok_or("remote file is read-only; use :remote edit first")?;
        if doc.buf.readonly && !force {
            return Err("readonly buffer; remote write authority remains available via :w!".into());
        }
        if !source.window.is_complete() || self.remote_following(document) {
            return Err("partial/following remote windows cannot be saved".into());
        }
        if permit.version.file() != &source.file {
            return Err("remote write permit belongs to another file".into());
        }
        let id = permit.id;
        let before = permit.version.clone();
        let contents = doc.buf.snapshot();
        let revision = doc.buf.revision();
        let prepared = FrozenStore::default();
        self.remote.writes.attempts.insert(
            document,
            Attempt {
                permit: id,
                revision,
                before: before.clone(),
                contents: contents.clone(),
                unconfirmed: false,
                store: None,
                namespace: None,
                prepared: Arc::clone(&prepared),
            },
        );
        let result = self.start_remote_write(
            document,
            revision,
            Some(id),
            Work::Save {
                before,
                contents,
                close,
                prepared,
            },
        );
        if result.is_err() {
            self.remote.writes.attempts.remove(&document);
        }
        result
    }

    pub(super) fn verify_remote_save(&mut self) -> Result<(), String> {
        let document = self.current();
        if self.remote.writes.pending.contains_key(&document) {
            return Err("remote write operation already pending".into());
        }
        let attempt = self
            .remote
            .writes
            .attempts
            .get(&document)
            .filter(|attempt| attempt.unconfirmed)
            .ok_or("no unconfirmed remote save to verify")?;
        let permit = self
            .cur()
            .remote_metadata()
            .and_then(|source| source.write.as_ref())
            .ok_or("remote write permit was revoked")?;
        if permit.id != attempt.permit {
            return Err("remote save attempt belongs to an old permit".into());
        }
        let work = Work::Verify {
            before: attempt.before.clone(),
            contents: attempt.contents.clone(),
            store: attempt.store.clone(),
            namespace: attempt.namespace.clone(),
        };
        self.start_remote_write(document, attempt.revision, Some(attempt.permit), work)
    }

    fn start_remote_write(
        &mut self,
        document: DocumentId,
        revision: BufferRevision,
        permit: Option<WorkerId>,
        work: Work,
    ) -> Result<(), String> {
        let request = self.worker_ids.allocate().map_err(|error| error.message)?;
        let ticket = Ticket {
            request,
            key: RemoteWriteKey {
                document,
                revision,
                file: work.file().clone(),
                permit,
                focus: self.focus_epoch,
                action: work.action(),
            },
        };
        self.remote.writes.pending.insert(document, ticket.clone());
        self.message = match ticket.key.action {
            WriteAction::Enable => "checking remote edit authority",
            WriteAction::Save { .. } => "saving remotely",
            WriteAction::Verify => "verifying remote save",
        }
        .into();
        match self.tape.request(
            "remote.write",
            &(&ticket, work.before(), work.contents().len_bytes()),
        ) {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => {
                self.remote.writes.pending.remove(&document);
                return Err(format!("remote write replay diverged: {error}"));
            }
        }
        let sender = self.io.tx.clone();
        let workers = self.remote.workers.clone();
        let handle = worker::spawn(
            "remote-write",
            move |outcome| {
                let _ = sender.send(IoEvent::Remote(RemoteEvent::Write(Box::new(Completion {
                    ticket,
                    outcome,
                }))));
            },
            move |token| Outcome::Success(work.execute(&workers, &token)),
        );
        self.worker_handles.insert(request, handle);
        Ok(())
    }

    pub(super) fn remote_write_done(
        &mut self,
        completion: Completion<RemoteWriteKey, RemoteWriteResult>,
    ) {
        let document = completion.ticket.key.document;
        if self.remote.writes.pending.get(&document) != Some(&completion.ticket) {
            return;
        }
        self.remote.writes.pending.remove(&document);
        self.worker_handles.remove(&completion.ticket.request);
        let request = completion.ticket.request;
        let key = completion.ticket.key;
        let valid = self
            .docs
            .get(document)
            .and_then(|doc| doc.remote_metadata())
            .is_some_and(|source| {
                source.file == key.file
                    && match key.action {
                        WriteAction::Enable => {
                            source.write.is_none()
                                && source.window.is_complete()
                                && !self.remote_following(document)
                                && self.doc(document).buf.revision() == key.revision
                        }
                        _ => source
                            .write
                            .as_ref()
                            .is_some_and(|permit| Some(permit.id) == key.permit),
                    }
            });
        if !valid {
            self.collection_save_progress(document, false);
            if key.action == WriteAction::Enable
                && !self.docs.is_empty()
                && self.current() == document
            {
                self.message =
                    "remote edit admission cancelled: snapshot or follow state changed".into();
            }
            if matches!(key.action, WriteAction::Save { .. }) {
                self.message = "remote save result discarded: source authority changed; disk outcome unconfirmed".into();
                self.finish_save_feedback(document);
            }
            return;
        }
        if matches!(key.action, WriteAction::Save { .. }) {
            if let Some(attempt) = self.remote.writes.attempts.get_mut(&document) {
                if let Some((namespace, operation)) = attempt.prepared.lock().take() {
                    attempt.namespace = Some(namespace);
                    if !matches!(
                        &completion.outcome,
                        Outcome::Success(
                            RemoteWriteResult::Saved(_) | RemoteWriteResult::StoreUncertain { .. }
                        )
                    ) {
                        attempt.store = Some(Box::new(StepReceipt {
                            step: 0,
                            operation,
                            outcome: StepOutcome::Unconfirmed {
                                detail: "store reply lost after preparation".into(),
                                observed_destination: None,
                                recovery: None,
                                publication: None,
                            },
                        }));
                    }
                }
            }
        }
        match completion.outcome {
            Outcome::Success(RemoteWriteResult::Enabled(version))
                if key.action == WriteAction::Enable && version.file() == &key.file =>
            {
                self.remote.writes.relocations.remove(&document);
                let mut doc = self.doc_mut(document);
                if let DocumentSource::Remote(source) = &mut doc.source {
                    source.write = Some(WritePermit {
                        id: request,
                        version,
                    });
                    source.selection = strop_remote::ReadSelection::Full;
                    doc.buf.clear_readonly();
                }
                drop(doc);
                self.message =
                    "remote editing enabled (cooperative locks; other programs can still race)"
                        .into();
            }
            Outcome::Success(RemoteWriteResult::Saved(version))
                if matches!(key.action, WriteAction::Save { .. }) =>
            {
                return self.accept_remote_receipt(key, version);
            }
            Outcome::Success(RemoteWriteResult::StoreUncertain { detail, receipt })
                if matches!(key.action, WriteAction::Save { .. }) =>
            {
                // Frozen verify evidence crosses with the uncertain
                // outcome; dirty text is preserved either way.
                if let Some(attempt) = self.remote.writes.attempts.get_mut(&document) {
                    attempt.store = Some(receipt);
                }
                self.remote_write_uncertain(document, detail);
            }
            Outcome::Success(RemoteWriteResult::Verified(VerifiedWrite::Committed(version)))
                if key.action == WriteAction::Verify =>
            {
                return self.accept_remote_receipt(key, version);
            }
            Outcome::Success(RemoteWriteResult::Verified(VerifiedWrite::Unchanged))
                if key.action == WriteAction::Verify =>
            {
                self.remote.writes.attempts.remove(&document);
                self.message = "remote original is unchanged; local edits remain unsaved".into();
            }
            Outcome::Success(RemoteWriteResult::Refused(error)) => {
                if error.is_unconfirmed() {
                    self.remote_write_uncertain(document, error.to_string());
                } else {
                    if key.action != WriteAction::Verify
                        || matches!(
                            &error,
                            RemoteSaveError::Refused {
                                kind: RefusalKind::Conflict,
                                ..
                            }
                        )
                    {
                        self.remote.writes.attempts.remove(&document);
                    }
                    self.message = error.to_string();
                }
            }
            Outcome::Cancelled(_) if key.action == WriteAction::Enable => {
                self.message = "remote edit admission cancelled".into()
            }
            Outcome::Failed { failure, .. } if key.action == WriteAction::Enable => {
                self.message = format!("remote edit admission failed: {}", failure.message)
            }
            Outcome::Cancelled(_) => self.remote_write_uncertain(
                document,
                "cancelled; the remote outcome is unconfirmed — :remote verify".into(),
            ),
            Outcome::Failed { failure, .. } => self.remote_write_uncertain(
                document,
                format!(
                    "{}; remote outcome unconfirmed — :remote verify",
                    failure.message
                ),
            ),
            Outcome::Success(_) => self.remote_write_uncertain(
                document,
                "remote write result does not match its request".into(),
            ),
        }
        if matches!(key.action, WriteAction::Save { .. }) {
            self.finish_save_feedback(document);
            self.collection_save_progress(document, false);
        }
    }

    fn remote_write_uncertain(&mut self, document: DocumentId, message: String) {
        if let Some(attempt) = self.remote.writes.attempts.get_mut(&document) {
            attempt.unconfirmed = true;
        }
        self.message = message;
        self.finish_save_feedback(document);
        self.collection_save_progress(document, false);
    }
    fn accept_remote_receipt(&mut self, key: RemoteWriteKey, version: WriteVersion) {
        if version.file() != &key.file {
            self.remote_write_uncertain(
                key.document,
                "receipt belongs to a different remote file".into(),
            );
            return;
        }
        let current = {
            let mut doc = self.doc_mut(key.document);
            let DocumentSource::Remote(source) = &mut doc.source else {
                return;
            };
            let Some(permit) = source.write.as_mut() else {
                return;
            };
            let size = version.size();
            if Some(permit.id) != key.permit {
                return;
            }
            permit.version = version;
            source.window =
                strop_remote::RemoteWindow::resolve(&strop_remote::ReadSelection::Full, size);
            source.selection = strop_remote::ReadSelection::Full;
            // Remote identity never becomes a local filesystem identity.
            debug_assert!(doc.buf.path.is_none());
            doc.buf.acknowledge_saved_revision(key.revision)
        };
        self.remote.writes.attempts.remove(&key.document);
        self.message = if current {
            "written remotely"
        } else {
            "remote snapshot written; newer edits remain unsaved"
        }
        .into();
        self.finish_save_feedback(key.document);
        self.collection_save_progress(key.document, current);
        if current
            && matches!(key.action, WriteAction::Save { close: true })
            && !self.docs.is_empty()
            && self.current() == key.document
            && self.focus_epoch == key.focus
        {
            self.close_pane_or_buffer(false);
        }
    }

    pub(crate) fn revoke_remote_write(&mut self, document: DocumentId) {
        self.remote.writes.relocations.remove(&document);
        if let Some(doc) = self.docs.get_mut(document) {
            if let DocumentSource::Remote(source) = &mut doc.source {
                source.write = None;
                doc.buf
                    .set_readonly(strop_core::ReadonlyReason::RemoteAuthority);
            }
        }
        self.remote.writes.attempts.remove(&document);
        if let Some(ticket) = self.remote.writes.pending.remove(&document) {
            if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                handle.cancel(CancelReason::OwnerClosed);
            }
        }
    }

    pub(crate) fn relocate_remote_binding(&mut self, document: DocumentId, file: RemoteFile) {
        let baseline = self
            .docs
            .get(document)
            .and_then(|doc| doc.remote_metadata())
            .and_then(|source| source.write.as_ref())
            .map(|permit| permit.version.clone())
            .or_else(|| self.remote.writes.relocations.get(&document).cloned());
        self.revoke_remote_write(document);
        if let Some(doc) = self.docs.get_mut(document) {
            if let DocumentSource::Remote(source) = &mut doc.source {
                doc.buf.name = Some(file.to_string());
                source.file = file;
                if let Some(baseline) = baseline {
                    self.remote.writes.relocations.insert(document, baseline);
                }
            }
        }
    }
    pub(crate) fn remote_write_blocks_refresh(&self, document: DocumentId) -> bool {
        self.remote.writes.pending.contains_key(&document)
            || self
                .remote
                .writes
                .attempts
                .get(&document)
                .is_some_and(|attempt| attempt.unconfirmed)
    }
    pub(crate) fn cancel_remote_write(&mut self, document: DocumentId) {
        if let Some(ticket) = self.remote.writes.pending.get(&document).cloned() {
            if ticket.key.action == WriteAction::Enable {
                self.remote.writes.pending.remove(&document);
                self.message = "remote edit admission cancelled".into();
            }
            if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                handle.cancel(CancelReason::Dismissed);
            }
        }
    }
    pub(crate) fn remote_write_pending(&self, request: WorkerId) -> bool {
        self.remote.writes.pending.values().any(|ticket| {
            ticket.request == request
                && matches!(
                    ticket.key.action,
                    WriteAction::Save { .. } | WriteAction::Verify
                )
        })
    }
    pub fn remote_write_status(&self) -> Option<&'static str> {
        if self.docs.is_empty() {
            return None;
        }
        let pending = self.remote.writes.pending.get(&self.current())?;
        Some(match pending.key.action {
            WriteAction::Enable => "checking edit",
            WriteAction::Save { .. } => "saving",
            WriteAction::Verify => "verifying save",
        })
    }
    pub(crate) fn remote_edit_authorized(&self) -> bool {
        self.cur()
            .remote_metadata()
            .is_some_and(|source| source.write.is_some())
    }
}

#[cfg(test)]
mod tests;
