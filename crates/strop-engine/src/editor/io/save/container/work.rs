use super::{
    store, ContainerSaveError, ContainerWriteAction, ContainerWriteResult, ContainerWriteVersion,
    FrozenStore, VerifiedContainerWrite,
};
use crate::editor::containers::BoundWorker;
use ropey::Rope;
use std::sync::Arc;
use strop_core::worker;
use strop_worker_protocol::NamespaceIdentity;
use strop_workspace::operation::StepReceipt;
use strop_workspace::ResourceLocation;

pub(super) enum Work {
    Enable {
        location: ResourceLocation,
        started_at: String,
        contents: Rope,
        worker: Arc<BoundWorker>,
    },
    Save {
        before: ContainerWriteVersion,
        contents: Rope,
        worker: Arc<BoundWorker>,
        prepared: FrozenStore,
    },
    Verify {
        before: ContainerWriteVersion,
        contents: Rope,
        store: Option<Box<StepReceipt>>,
        namespace: Option<NamespaceIdentity>,
        worker: Arc<BoundWorker>,
    },
}

impl Work {
    pub(super) fn action(&self) -> ContainerWriteAction {
        match self {
            Self::Enable { .. } => ContainerWriteAction::Enable,
            Self::Save { .. } => ContainerWriteAction::Save { close: false },
            Self::Verify { .. } => ContainerWriteAction::Verify,
        }
    }

    pub(super) fn location(&self) -> &ResourceLocation {
        match self {
            Self::Enable { location, .. } => location,
            Self::Save { before, .. } | Self::Verify { before, .. } => &before.location,
        }
    }

    pub(super) fn started_at(&self) -> &str {
        match self {
            Self::Enable { started_at, .. } => started_at,
            Self::Save { before, .. } | Self::Verify { before, .. } => &before.started_at,
        }
    }

    pub(super) fn before(&self) -> Option<&ContainerWriteVersion> {
        match self {
            Self::Enable { .. } => None,
            Self::Save { before, .. } | Self::Verify { before, .. } => Some(before),
        }
    }

    pub(super) fn contents(&self) -> &Rope {
        match self {
            Self::Enable { contents, .. }
            | Self::Save { contents, .. }
            | Self::Verify { contents, .. } => contents,
        }
    }

    pub(super) fn execute(self, token: &worker::CancelToken) -> ContainerWriteResult {
        match self {
            Self::Enable {
                location,
                started_at,
                contents,
                worker,
            } => store::prepare_edit(worker, &location, &started_at, &contents, token)
                .map(ContainerWriteResult::Enabled)
                .unwrap_or_else(ContainerWriteResult::Refused),
            Self::Save {
                before,
                contents,
                worker,
                prepared,
            } => match store::save(worker, &before, &contents, &prepared, token) {
                Ok(store::WorkerSave::Saved(version)) => ContainerWriteResult::Saved(version),
                Ok(store::WorkerSave::Unconfirmed { detail, receipt }) => {
                    ContainerWriteResult::StoreUncertain { detail, receipt }
                }
                Err(error) => ContainerWriteResult::Refused(error),
            },
            Self::Verify {
                before,
                store: receipt,
                namespace,
                worker,
                ..
            } => {
                let (Some(receipt), Some(namespace)) = (receipt, namespace) else {
                    return ContainerWriteResult::Refused(ContainerSaveError::Unconfirmed {
                        detail: "no frozen container Store evidence to verify".into(),
                    });
                };
                match store::verify(worker, &before, *receipt, namespace, token) {
                    Ok(store::WorkerVerification::Committed(version)) => {
                        ContainerWriteResult::Verified(VerifiedContainerWrite::Committed(version))
                    }
                    Ok(store::WorkerVerification::Unchanged) => {
                        ContainerWriteResult::Verified(VerifiedContainerWrite::Unchanged)
                    }
                    Err(error) => ContainerWriteResult::Refused(error),
                }
            }
        }
    }
}
