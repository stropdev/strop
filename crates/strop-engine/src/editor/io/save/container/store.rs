//! Conditional container Store operations on the admitted worker pipe.

use super::{ContainerSaveError, ContainerWriteVersion, FrozenStore};
use crate::editor::containers::BoundWorker;
use crate::editor::namespace::{StoreDispatch, StoreOutcome};
use ropey::Rope;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use strop_core::worker::CancelToken;
use strop_workspace::operation::{
    FsFailure, FsFailureKind, OperationIntent, OperationKind, StepOutcome, StepReceipt, StorePolicy,
};
use strop_workspace::ResourceLocation;

pub(super) fn digest_of(text: &Rope) -> [u8; 32] {
    let mut hash = Sha256::new();
    for chunk in text.chunks() {
        hash.update(chunk.as_bytes());
    }
    hash.finalize().into()
}

fn refused(failure: FsFailure) -> ContainerSaveError {
    ContainerSaveError::Refused {
        kind: failure.kind,
        detail: failure.detail,
    }
}

fn version(
    location: &ResourceLocation,
    started_at: &str,
    observed: &strop_workspace::Observation,
    content: [u8; 32],
) -> Result<ContainerWriteVersion, ContainerSaveError> {
    let metadata = |detail: &str| ContainerSaveError::Refused {
        kind: FsFailureKind::Protocol,
        detail: detail.to_owned(),
    };
    Ok(ContainerWriteVersion {
        location: location.clone(),
        started_at: started_at.to_owned(),
        modified: observed
            .modified
            .ok_or_else(|| metadata("container file has no mtime evidence"))?,
        size: observed
            .size
            .ok_or_else(|| metadata("container file has no size evidence"))?,
        identity: Some(
            observed
                .identity
                .ok_or_else(|| metadata("container file has no object identity"))?,
        ),
        attributes: Some(
            observed
                .attributes
                .ok_or_else(|| metadata("container file has no attribute baseline"))?,
        ),
        content,
    })
}

/// Verify that the bytes displayed by this document are still the bytes in
/// the captured container incarnation. Preparation has no write effect.
pub(super) fn prepare_edit(
    worker: Arc<BoundWorker>,
    location: &ResourceLocation,
    started_at: &str,
    contents: &Rope,
    token: &CancelToken,
) -> Result<ContainerWriteVersion, ContainerSaveError> {
    let displayed = digest_of(contents);
    let intent = OperationIntent {
        kind: OperationKind::Store,
        source: None,
        destination: Some(location.clone()),
        copy_version: strop_workspace::operation::CopyVersion::Stored,
        expected_content: None,
        store: Some(StorePolicy {
            baseline: None,
            baseline_object: None,
            baseline_attributes: None,
            force: false,
            expect_absent: false,
            displayed: Some(displayed),
        }),
    };
    let dispatch = StoreDispatch::Container(worker);
    let operation = dispatch.prepare_store(intent, token).map_err(refused)?;
    let observed = operation
        .operation
        .destination
        .as_ref()
        .and_then(|destination| destination.value.as_ref())
        .ok_or_else(|| ContainerSaveError::Refused {
            kind: FsFailureKind::Conflict,
            detail: "container file vanished during edit admission".into(),
        })?;
    version(location, started_at, observed, displayed)
}

pub(super) enum WorkerSave {
    Saved(ContainerWriteVersion),
    Unconfirmed {
        detail: String,
        receipt: Box<StepReceipt>,
    },
}

pub(super) fn save(
    worker: Arc<BoundWorker>,
    before: &ContainerWriteVersion,
    contents: &Rope,
    prepared: &FrozenStore,
    token: &CancelToken,
) -> Result<WorkerSave, ContainerSaveError> {
    let identity = before.identity.ok_or_else(|| ContainerSaveError::Refused {
        kind: FsFailureKind::Protocol,
        detail: "container write permit has no object identity".into(),
    })?;
    let attributes = before
        .attributes
        .ok_or_else(|| ContainerSaveError::Refused {
            kind: FsFailureKind::Protocol,
            detail: "container write permit has no attribute baseline".into(),
        })?;
    let digest = digest_of(contents);
    let intent = OperationIntent {
        kind: OperationKind::Store,
        source: None,
        destination: Some(before.location.clone()),
        copy_version: strop_workspace::operation::CopyVersion::Stored,
        expected_content: Some(digest),
        store: Some(StorePolicy {
            baseline: Some(before.modified),
            baseline_object: Some(identity),
            baseline_attributes: Some(attributes),
            // Container :w! may override buffer readonly, never a moved
            // conditional-save baseline.
            force: false,
            expect_absent: false,
            displayed: Some(before.content),
        }),
    };
    let dispatch = StoreDispatch::Container(Arc::clone(&worker));
    let operation = dispatch.prepare_store(intent, token).map_err(refused)?;
    let namespace = worker
        .worker()
        .namespace()
        .map_err(|error| ContainerSaveError::Refused {
            kind: FsFailureKind::Io,
            detail: error.to_string(),
        })?;
    if worker.worker().session() != Some(operation.session) {
        return Err(ContainerSaveError::Refused {
            kind: FsFailureKind::Conflict,
            detail: "the container worker session changed during save admission".into(),
        });
    }
    *prepared.lock() = Some((namespace, operation.operation.clone()));
    let mut bytes = Vec::with_capacity(contents.len_bytes());
    for chunk in contents.chunks() {
        bytes.extend_from_slice(chunk.as_bytes());
    }
    let receipt = match dispatch.apply_store(operation, &bytes, token) {
        StoreOutcome::Refused(failure) => return Err(refused(failure)),
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
                    detail: "container receipt does not prove the intended bytes".into(),
                    receipt,
                });
            }
            let Some(after) = destination_after.as_ref() else {
                return Ok(WorkerSave::Unconfirmed {
                    detail: "committed container Store has no destination observation".into(),
                    receipt,
                });
            };
            match version(&before.location, &before.started_at, after, digest) {
                Ok(saved) => Ok(WorkerSave::Saved(saved)),
                Err(ContainerSaveError::Refused { detail, .. }) => {
                    Ok(WorkerSave::Unconfirmed { detail, receipt })
                }
                Err(ContainerSaveError::Unconfirmed { detail }) => {
                    Ok(WorkerSave::Unconfirmed { detail, receipt })
                }
            }
        }
        StepOutcome::Unconfirmed { detail, .. } => Ok(WorkerSave::Unconfirmed {
            detail: detail.clone(),
            receipt,
        }),
        StepOutcome::Refused(failure) => Err(refused(failure.clone())),
        StepOutcome::Cancelled { detail } => Err(ContainerSaveError::Refused {
            kind: FsFailureKind::Cancelled,
            detail: detail.clone(),
        }),
    }
}

pub(super) enum WorkerVerification {
    Committed(Box<ContainerWriteVersion>),
    Unchanged,
}

pub(super) fn verify(
    worker: Arc<BoundWorker>,
    before: &ContainerWriteVersion,
    receipt: StepReceipt,
    namespace: strop_worker_protocol::NamespaceIdentity,
    token: &CancelToken,
) -> Result<WorkerVerification, ContainerSaveError> {
    let intended = receipt.operation.intent.expected_content;
    let dispatch = StoreDispatch::Container(worker);
    match dispatch
        .verify_store_recovered(receipt, namespace, token)
        .map_err(refused)?
    {
        strop_workspace::operation::VerifiedOutcome::Unchanged => Ok(WorkerVerification::Unchanged),
        strop_workspace::operation::VerifiedOutcome::Unknown { detail } => {
            Err(ContainerSaveError::Unconfirmed { detail })
        }
        strop_workspace::operation::VerifiedOutcome::Committed(change) => {
            let after = change.destination_after.as_ref().ok_or_else(|| {
                ContainerSaveError::Unconfirmed {
                    detail: "verified container Store has no destination observation".into(),
                }
            })?;
            let saved = match version(
                &before.location,
                &before.started_at,
                after,
                intended.unwrap_or(before.content),
            ) {
                Ok(saved) => saved,
                Err(ContainerSaveError::Refused { detail, .. })
                | Err(ContainerSaveError::Unconfirmed { detail }) => {
                    return Err(ContainerSaveError::Unconfirmed { detail })
                }
            };
            Ok(WorkerVerification::Committed(Box::new(saved)))
        }
    }
}
