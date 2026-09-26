//! Container worker leases, captured from an explicit user-authorized
//! action. The selected engine, canonical id, StartedAt, user and cwd
//! come from one inspect; no name or analogous local path can retarget
//! an old worker. Browsing remains read-only without a lease.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use strop_containers::{ContainerIdentity, EngineRef};
use strop_core::worker::CancelToken;
use strop_worker_client::{ClientError, Worker};
use strop_worker_deploy::provider::{DeployProvider, ProviderError};
use strop_worker_deploy::{
    deploy, ArtifactSupply, Consent, ContainerProvider, DeployOrigin, DeployOutcome, DeployRequest,
    ShellPolicy, VerifiedObject,
};
use strop_workspace::operation::{FsFailure, FsFailureKind};
use strop_workspace::{ContainerId, DirectorySnapshot, Filesystem, ResourceLocation};

use crate::editor::namespace::map_client;
use crate::editor::worker_catalog::{catalog_for, map_refusal, select_binary};

fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}

fn provider_failure(error: ProviderError) -> FsFailure {
    let kind = match &error {
        ProviderError::Permission(_) => FsFailureKind::Permission,
        ProviderError::ReadOnly(_) | ProviderError::NoExec(_) | ProviderError::NotFound(_) => {
            FsFailureKind::Unsupported
        }
        ProviderError::Handshake(_) | ProviderError::CorruptCache(_) => FsFailureKind::Protocol,
        ProviderError::Transport(_) | ProviderError::DiskFull(_) => FsFailureKind::Io,
    };
    failure(kind, error.to_string())
}

#[derive(Default)]
struct Entry {
    ready: OnceLock<Arc<BoundWorker>>,
    admitting: Mutex<()>,
    /// Captured by the attach job, never re-probed in a later context.
    engine: Mutex<Option<EngineRef>>,
}
struct Incarnation {
    started_at: String,
    entry: Arc<Entry>,
}

impl Incarnation {
    fn new(identity: &ContainerIdentity) -> Self {
        Self {
            started_at: identity.started_at.clone(),
            entry: Arc::new(Entry::default()),
        }
    }
}

/// A lease and the one observed incarnation it can serve. The worker
/// factory independently revalidates this identity on every reconnect.
pub(crate) struct BoundWorker {
    id: ContainerId,
    started_at: String,
    worker: Worker,
    target: String,
    artifact: VerifiedObject,
}

impl BoundWorker {
    pub(crate) fn worker(&self) -> &Worker {
        &self.worker
    }

    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    pub(crate) fn artifact_path(&self) -> &str {
        &self.artifact.path
    }
    pub(crate) fn artifact_sha256(&self) -> &str {
        &self.artifact.sha256
    }

    pub(crate) fn matches(&self, identity: &ContainerIdentity) -> bool {
        identity.id == self.id.as_str() && identity.started_at == self.started_at
    }

    fn location(&self, location: &ResourceLocation) -> Result<ResourceLocation, ClientError> {
        if !matches!(&location.filesystem, Filesystem::Container(id) if id == &self.id) {
            return Err(ClientError::Protocol(
                strop_worker_protocol::ProtocolError::Unexpected {
                    message: "container location belongs to another worker namespace".into(),
                },
            ));
        }
        Ok(ResourceLocation::local(location.path.clone()))
    }

    pub(crate) fn observe(
        &self,
        token: &CancelToken,
        location: &ResourceLocation,
    ) -> Result<Option<strop_workspace::Observation>, ClientError> {
        let native = self.location(location)?;
        let observations = self.worker.observe(token, vec![native.clone()])?;
        let mut observations = observations.into_iter();
        let Some(observation) = observations.next() else {
            return Err(ClientError::Protocol(
                strop_worker_protocol::ProtocolError::Unexpected {
                    message: "container worker returned no observation".into(),
                },
            ));
        };
        if observation.location != native || observations.next().is_some() {
            return Err(ClientError::Protocol(
                strop_worker_protocol::ProtocolError::Unexpected {
                    message: "container worker returned a foreign observation".into(),
                },
            ));
        }
        Ok(observation.value)
    }

    pub(crate) fn list(
        &self,
        token: &CancelToken,
        location: &ResourceLocation,
    ) -> Result<DirectorySnapshot, ClientError> {
        let native = self.location(location)?;
        let mut snapshot = self.worker.list(token, native.clone())?;
        if snapshot.location != native {
            return Err(ClientError::Protocol(
                strop_worker_protocol::ProtocolError::Unexpected {
                    message: "container worker returned a foreign listing".into(),
                },
            ));
        }
        snapshot.location = location.clone();
        Ok(snapshot)
    }

    pub(crate) fn read(
        &self,
        token: &CancelToken,
        location: &ResourceLocation,
        length: u64,
    ) -> Result<strop_worker_client::ReadPayload, ClientError> {
        self.worker
            .read(token, self.location(location)?, 0, Some(length))
    }
}

/// Per-incarnation admission serialization; input-side lookups only
/// take the short table lock and never wait for Docker or upload I/O.
#[derive(Clone, Default)]
pub(crate) struct ContainerWorkers {
    inner: Arc<Mutex<HashMap<String, Incarnation>>>,
}

impl ContainerWorkers {
    pub(crate) fn note_inspect(
        &self,
        identity: &ContainerIdentity,
        engine: EngineRef,
    ) -> Result<(), FsFailure> {
        let mut entries = self.inner.lock();
        let slot = entries
            .entry(identity.id.clone())
            .or_insert_with(|| Incarnation::new(identity));
        if slot.started_at != identity.started_at {
            return Err(failure(
                FsFailureKind::Conflict,
                "container restarted; old attachment cannot bind the new incarnation",
            ));
        }
        let mut captured = slot.entry.engine.lock();
        if captured
            .as_ref()
            .is_some_and(|prior| !prior.same_connection(&engine))
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "selected Docker engine changed; old attachment cannot bind another engine",
            ));
        }
        *captured = Some(engine);
        Ok(())
    }

    pub(crate) fn get(&self, identity: &ContainerIdentity) -> Option<Arc<BoundWorker>> {
        self.inner.lock().get(&identity.id).and_then(|slot| {
            (slot.started_at == identity.started_at)
                .then(|| slot.entry.ready.get().cloned())
                .flatten()
        })
    }

    /// Match a captured repository/service request to exactly the
    /// attached container incarnation without copying its environment.
    pub(crate) fn get_for(&self, id: &ContainerId, started_at: &str) -> Option<Arc<BoundWorker>> {
        self.inner.lock().get(id.as_str()).and_then(|slot| {
            (slot.started_at == started_at)
                .then(|| slot.entry.ready.get().cloned())
                .flatten()
        })
    }

    /// Only an explicit worker-using action may call this. A shellless
    /// image takes the administrator's preinstalled object path;
    /// otherwise a matching local release artifact is uploaded into
    /// the selected principal's private cache, never into the image.
    pub(crate) fn admit(
        &self,
        identity: &ContainerIdentity,
        action: &str,
        token: &CancelToken,
    ) -> Result<Arc<BoundWorker>, FsFailure> {
        let id = ContainerId::canonical(identity.id.clone())
            .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?;
        let entry = self
            .inner
            .lock()
            .get(&identity.id)
            .filter(|slot| slot.started_at == identity.started_at)
            .map(|slot| Arc::clone(&slot.entry))
            .ok_or_else(|| {
                failure(
                    FsFailureKind::Conflict,
                    "attach this container incarnation before admitting its worker",
                )
            })?;
        if let Some(worker) = entry.ready.get() {
            return Ok(Arc::clone(worker));
        }
        let _admitting = entry.admitting.lock();
        if let Some(worker) = entry.ready.get() {
            return Ok(Arc::clone(worker));
        }
        let engine = entry.engine.lock().clone().ok_or_else(|| {
            failure(
                FsFailureKind::Unsupported,
                "attach this container before admitting its worker",
            )
        })?;
        let preinstalled = match std::env::var_os("STROP_CONTAINER_WORKER_PATH") {
            Some(path) => Some(path.into_string().map_err(|_| {
                failure(
                    FsFailureKind::InvalidPath,
                    "STROP_CONTAINER_WORKER_PATH must be UTF-8 container path text",
                )
            })?),
            None => None,
        };
        let shell = if preinstalled.is_some() {
            ShellPolicy::Absent
        } else {
            ShellPolicy::Required
        };
        let provider = ContainerProvider::capture(&engine, identity, shell, token)
            .map_err(provider_failure)?;
        let binary = select_binary(&provider.endpoint().target)?;
        let catalog = catalog_for(&provider.endpoint().target, &binary)?;
        let supply = match preinstalled {
            Some(path) => ArtifactSupply::Preinstalled { path },
            None => ArtifactSupply::LocalBinary { path: binary },
        };
        let report = deploy(
            &provider,
            &DeployRequest {
                catalog,
                editor_version: env!("CARGO_PKG_VERSION").into(),
                consent: Consent::Granted {
                    action: action.to_string(),
                },
                supply,
                online: false,
            },
        );
        let ready = match report.outcome {
            DeployOutcome::Probed(ready) => ready,
            DeployOutcome::Fallback(reason) => {
                return Err(failure(FsFailureKind::Unsupported, reason.to_string()));
            }
            DeployOutcome::Refused(reason) => return Err(map_refusal(reason)),
            DeployOutcome::PublishedNotReady { reason, .. } => {
                return Err(failure(FsFailureKind::Io, reason));
            }
        };
        let worker_lease = provider.worker(&ready.object.path);
        // A successful deployment probe is not the editor's own worker.
        // Its first real handshake registers the lease before this
        // incarnation is published as ready.
        worker_lease.capabilities().map_err(|error| {
            failure(
                FsFailureKind::Io,
                format!("container live worker could not start: {error}"),
            )
        })?;
        if ready.origin != DeployOrigin::Preinstalled {
            let maintenance = worker_lease
                .collect_cache(token, &provider.endpoint().context)
                .map_err(map_client)?;
            if let Some(error) = maintenance.failure {
                return Err(failure(
                    error.kind,
                    format!(
                        "cache maintenance after {} retirements: {}",
                        maintenance.report.removed_objects.len(),
                        error.detail
                    ),
                ));
            }
        }
        let worker = Arc::new(BoundWorker {
            id,
            started_at: identity.started_at.clone(),
            target: provider.endpoint().target.clone(),
            artifact: ready.object,
            worker: worker_lease,
        });
        let published = entry.ready.set(Arc::clone(&worker));
        debug_assert!(published.is_ok(), "one container admission per identity");
        Ok(worker)
    }
}
