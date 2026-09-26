//! Worker-routed remote save transport (0058 WK09): the Store intent's
//! protected save through the endpoint's admitted worker lease — the same
//! kernel the local worker serves. The editor-side state machine
//! (permits, frozen attempts, `:remote verify`) retains one Store
//! baseline. SFTP-only hosts remain read-only without an admitted worker.
use super::{FrozenStore, RefusalKind, RemoteSaveError, WriteVersion};
use ropey::Rope;
use sha2::{Digest, Sha256};
use strop_core::worker::CancelToken;
use strop_remote::worker_transport::RemoteWorker;
use strop_workspace::operation::{FsFailure, FsFailureKind, StepOutcome, StepReceipt};
use strop_workspace::{RemoteFile, ResourceLocation};

use crate::editor::namespace::{StoreDispatch, StoreOutcome};

pub(crate) fn digest_of(text: &Rope) -> [u8; 32] {
    let mut hash = Sha256::new();
    for chunk in text.chunks() {
        hash.update(chunk.as_bytes());
    }
    hash.finalize().into()
}

fn refused(kind: RefusalKind, detail: impl Into<String>) -> RemoteSaveError {
    RemoteSaveError::Refused {
        kind,
        detail: detail.into(),
    }
}

pub(super) fn map_failure(failure: FsFailure) -> RemoteSaveError {
    let kind = match failure.kind {
        FsFailureKind::Conflict => RefusalKind::Conflict,
        FsFailureKind::Busy => RefusalKind::Busy,
        FsFailureKind::Unsupported => RefusalKind::Unsupported,
        FsFailureKind::Permission => RefusalKind::Permission,
        FsFailureKind::InvalidPath => RefusalKind::InvalidPath,
        FsFailureKind::Protocol => RefusalKind::Protocol,
        FsFailureKind::Cancelled => RefusalKind::Cancelled,
        FsFailureKind::Incomplete | FsFailureKind::Io => RefusalKind::Io,
    };
    refused(kind, failure.detail)
}

fn location(file: &RemoteFile) -> ResourceLocation {
    ResourceLocation::remote(file.endpoint().clone(), file.path().to_path_buf())
}

/// Edit admission (`:remote edit`) through the worker: the Store
/// prepare's displayed-digest condition proves the edited snapshot IS
/// the stored file; the destination observation becomes the baseline.
/// A relocation baseline re-admits only the same object with the same
/// stored bytes — evidence of bytes, never transferable authority.
pub(crate) fn prepare_edit(
    worker: &RemoteWorker,
    file: &RemoteFile,
    contents: &Rope,
    relocated: Option<&WriteVersion>,
    token: &CancelToken,
) -> Result<WriteVersion, RemoteSaveError> {
    if let Some(before) = relocated {
        if file.endpoint() != before.file.endpoint() {
            return Err(refused(
                RefusalKind::Conflict,
                "relocation crossed an endpoint",
            ));
        }
    }
    let proof = relocated
        .map(|before| before.content)
        .unwrap_or_else(|| digest_of(contents));
    let intent = strop_workspace::operation::OperationIntent {
        kind: strop_workspace::operation::OperationKind::Store,
        source: None,
        destination: Some(location(file)),
        copy_version: strop_workspace::operation::CopyVersion::Stored,
        expected_content: None,
        store: Some(strop_workspace::operation::StorePolicy {
            baseline: None,
            baseline_object: None,
            baseline_attributes: None,
            force: false,
            expect_absent: false,
            displayed: Some(proof),
        }),
    };
    let dispatch = StoreDispatch::Remote(worker.clone());
    let operation = dispatch.prepare_store(intent, token).map_err(map_failure)?;
    let observed = operation
        .destination
        .as_ref()
        .and_then(|destination| destination.value.as_ref())
        .ok_or_else(|| {
            refused(
                RefusalKind::Conflict,
                "remote file vanished during admission",
            )
        })?;
    if let Some(before) = relocated {
        if observed.identity != before.identity
            || observed.size != Some(before.size)
            || observed.attributes != before.attributes
        {
            return Err(refused(
                RefusalKind::Conflict,
                "relocated file no longer matches the stored source baseline",
            ));
        }
    }
    let modified = observed
        .modified
        .ok_or_else(|| refused(RefusalKind::Metadata, "remote mtime evidence unavailable"))?;
    let size = observed
        .size
        .ok_or_else(|| refused(RefusalKind::Metadata, "remote size evidence unavailable"))?;
    let identity = observed
        .identity
        .ok_or_else(|| refused(RefusalKind::Metadata, "remote object identity unavailable"))?;
    let attributes = observed
        .attributes
        .ok_or_else(|| refused(RefusalKind::Metadata, "remote xattr inventory unavailable"))?;
    Ok(WriteVersion {
        file: file.clone(),
        modified,
        size,
        identity: Some(identity),
        attributes: Some(attributes),
        content: proof,
    })
}

/// The worker-path save result: a committed new baseline, or honest
/// uncertainty with the frozen receipt retained for `:remote verify`.
pub(crate) enum WorkerSave {
    Saved(WriteVersion),
    Unconfirmed {
        detail: String,
        receipt: Box<StepReceipt>,
    },
}

/// Conditional atomic replacement through the worker: baseline-mtime
/// conflict semantics, permission preservation, staged publication.
pub(crate) fn save(
    worker: &RemoteWorker,
    before: &WriteVersion,
    contents: &Rope,
    prepared: &FrozenStore,
    token: &CancelToken,
) -> Result<WorkerSave, RemoteSaveError> {
    let identity = before
        .identity
        .ok_or_else(|| refused(RefusalKind::Metadata, "remote object identity unavailable"))?;
    let attributes = before
        .attributes
        .ok_or_else(|| refused(RefusalKind::Metadata, "remote xattr baseline unavailable"))?;
    let digest = digest_of(contents);
    let intent = strop_workspace::operation::OperationIntent {
        kind: strop_workspace::operation::OperationKind::Store,
        source: None,
        destination: Some(location(&before.file)),
        copy_version: strop_workspace::operation::CopyVersion::Stored,
        expected_content: Some(digest),
        store: Some(strop_workspace::operation::StorePolicy {
            baseline: Some(before.modified),
            baseline_object: Some(identity),
            baseline_attributes: Some(attributes),
            // Protected remote saves never bypass a moved baseline,
            // including via :w!.
            force: false,
            expect_absent: false,
            // A saved baseline includes its bytes, not only its mtime.
            displayed: Some(before.content),
        }),
    };
    let dispatch = StoreDispatch::Remote(worker.clone());
    let operation = dispatch.prepare_store(intent, token).map_err(map_failure)?;
    let namespace = worker
        .worker()
        .namespace()
        .map_err(|error| refused(RefusalKind::Io, error.to_string()))?;
    *prepared.lock() = Some((namespace, operation.clone()));
    let mut bytes = Vec::with_capacity(contents.len_bytes());
    for chunk in contents.chunks() {
        bytes.extend_from_slice(chunk.as_bytes());
    }
    let receipt = match dispatch.apply_store(operation, &bytes, token) {
        // The dispatch only returns a refusal when the worker proved
        // admission/content validation failed before any effect. Lost
        // transport/cancel replies carry an Unconfirmed StepReceipt.
        StoreOutcome::Refused(failure) => return Err(map_failure(failure)),
        StoreOutcome::Receipt(receipt) => receipt,
    };
    match &receipt.outcome {
        StepOutcome::Committed {
            destination_after,
            publication,
            ..
        } => {
            if (*publication).and_then(|witness| witness.content) != Some(digest) {
                return Ok(WorkerSave::Unconfirmed {
                    detail: "receipt does not match the intended bytes and metadata".into(),
                    receipt,
                });
            }
            let after = destination_after.as_ref().ok_or_else(|| {
                refused(
                    RefusalKind::Protocol,
                    "committed store carries no destination observation",
                )
            })?;
            let Some(modified) = after.modified else {
                return Ok(WorkerSave::Unconfirmed {
                    detail: "committed store carries no mtime evidence".into(),
                    receipt: receipt.clone(),
                });
            };
            let Some(attributes) = after.attributes else {
                return Ok(WorkerSave::Unconfirmed {
                    detail: "committed store carries no xattr evidence".into(),
                    receipt: receipt.clone(),
                });
            };
            Ok(WorkerSave::Saved(WriteVersion {
                file: before.file.clone(),
                modified,
                size: after.size.unwrap_or(bytes.len() as u64),
                identity: after.identity,
                attributes: Some(attributes),
                content: digest,
            }))
        }
        StepOutcome::Unconfirmed { detail, .. } => Ok(WorkerSave::Unconfirmed {
            detail: detail.clone(),
            receipt: receipt.clone(),
        }),
        StepOutcome::Refused(failure) => Err(map_failure(failure.clone())),
        StepOutcome::Cancelled { detail } => Err(refused(RefusalKind::Cancelled, detail.clone())),
    }
}

/// What verification proved about an unconfirmed worker save.
pub(crate) enum WorkerVerification {
    Committed(WriteVersion),
    Unchanged,
}

/// Reconcile one frozen unconfirmed store through the worker. Never a
/// retry: verification observes and syncs, it never writes.
pub(crate) fn verify(
    worker: &RemoteWorker,
    before: &WriteVersion,
    receipt: StepReceipt,
    namespace: strop_worker_protocol::NamespaceIdentity,
    token: &CancelToken,
) -> Result<WorkerVerification, RemoteSaveError> {
    // The verified-committed baseline's content is the attempt's intended
    // digest, proven by the kernel's digest comparison.
    let intended = receipt.operation.intent.expected_content;
    let dispatch = StoreDispatch::Remote(worker.clone());
    match dispatch
        .verify_store_recovered(receipt, namespace, token)
        .map_err(map_failure)?
    {
        strop_workspace::operation::VerifiedOutcome::Unchanged => Ok(WorkerVerification::Unchanged),
        strop_workspace::operation::VerifiedOutcome::Unknown { detail } => {
            Err(RemoteSaveError::Unconfirmed { detail })
        }
        strop_workspace::operation::VerifiedOutcome::Committed(change) => {
            let after = change.destination_after.as_ref().ok_or_else(|| {
                refused(
                    RefusalKind::Protocol,
                    "verified store carries no destination observation",
                )
            })?;
            let Some(modified) = after.modified else {
                return Err(RemoteSaveError::Unconfirmed {
                    detail: "verified store carries no mtime evidence".into(),
                });
            };
            let Some(attributes) = after.attributes else {
                return Err(RemoteSaveError::Unconfirmed {
                    detail: "verified store carries no xattr evidence".into(),
                });
            };
            Ok(WorkerVerification::Committed(WriteVersion {
                file: before.file.clone(),
                modified,
                size: after.size.unwrap_or(before.size),
                identity: after.identity,
                attributes: Some(attributes),
                content: intended.unwrap_or(before.content),
            }))
        }
    }
}
