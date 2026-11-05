//! Remote write authority is per document incarnation. Attempts outlive their
//! cancellable worker so a lost receipt can be reconciled without losing edits.
#[cfg(test)]
use crate::editor::Editor;
use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, Ticket, WorkerId};
#[cfg(test)]
use strop_core::worker::{Completion, Outcome};
use strop_worker_protocol::NamespaceIdentity;
#[cfg(test)]
use strop_workspace::operation::StepOutcome;
use strop_workspace::operation::{PreparedOperation, StepReceipt};
use strop_workspace::{FileTime, ObjectId, RemoteFile};

mod editor;
mod error;
pub(crate) mod store;
#[cfg(test)]
mod tests;

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
impl WritePermit {
    pub(crate) fn owns(&self, file: &RemoteFile) -> bool {
        self.version.file() == file
    }
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

    pub(crate) fn pending_for(&self, document: DocumentId) -> bool {
        self.pending.contains_key(&document)
    }

    pub(crate) fn unconfirmed_revision(&self, document: DocumentId) -> Option<BufferRevision> {
        self.attempts
            .get(&document)
            .filter(|attempt| attempt.unconfirmed)
            .map(|attempt| attempt.revision)
    }
}
