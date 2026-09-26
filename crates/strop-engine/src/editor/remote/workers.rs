//! Per-endpoint worker leases for SSH workspaces (0058 WK07).
//!
//! A host "admits a worker" once a consent-gated deployment through
//! strop-worker-deploy's state machine has landed: discovery over the
//! audited bootstrap line, the SFTP upload provider, verified-object
//! activation, a real handshake. Only then are reads, listings,
//! observations, mutations and notify subscriptions for that endpoint
//! routed through the lease ([`strop_remote::worker_transport::worker`])
//! — the same protocol as the local worker. Before that, and on hosts
//! that refuse deployment (restricted/SFTP-only accounts, wrong target,
//! read-only cache), the namespace keeps today's read-only SFTP path
//! and every mutation is the state machine's honest typed refusal —
//! never an alternate write path.
//!
//! Consent: the first deployment to an endpoint requires an explicit
//! authorized worker-using action. Browsing never deploys; the action
//! that triggers [`RemoteWorkers::admit`] is the user's own mutation
//! (a reviewed filesystem operation on the remote workspace), recorded
//! on the receipt. A previously authorized endpoint (its cache holds a
//! receipt for this context+principal) is quietly reused.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;

use strop_core::worker::CancelToken;
use strop_remote::worker_transport::RemoteWorker;
use strop_worker_deploy::deploy::{deploy, ArtifactSupply, Consent, DeployOutcome, DeployRequest};
use strop_worker_deploy::provider::DeployProvider;
use strop_worker_deploy::VerifiedObject;
use strop_workspace::operation::{FsFailure, FsFailureKind};
use strop_workspace::RemoteEndpoint;

use crate::editor::namespace::map_client;
use crate::editor::worker_catalog::{catalog_for, map_refusal, select_binary};

pub(crate) struct WorkerReady {
    pub worker: RemoteWorker,
    pub target: String,
    pub artifact: VerifiedObject,
}

/// Deployment serializes only callers for the same endpoint. A ready
/// lease is readable without taking that deployment lock, so editor
/// input never waits behind SSH/bootstrap/upload on another thread.
#[derive(Default)]
struct EndpointLease {
    ready: OnceLock<Arc<WorkerReady>>,
    deploying: Mutex<()>,
}

/// Per-endpoint worker leases shared with filesystem jobs.
#[derive(Clone, Default)]
pub(crate) struct RemoteWorkers {
    inner: Arc<Mutex<HashMap<RemoteEndpoint, Arc<EndpointLease>>>>,
}

fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}

impl RemoteWorkers {
    /// The admitted lease for one endpoint, when the host admits a
    /// worker. Read paths route through this; its absence means the
    /// read-only SFTP path serves exactly as before.
    pub(crate) fn get(&self, endpoint: &RemoteEndpoint) -> Option<RemoteWorker> {
        self.get_ready(endpoint).map(|ready| ready.worker.clone())
    }

    pub(crate) fn get_ready(&self, endpoint: &RemoteEndpoint) -> Option<Arc<WorkerReady>> {
        self.inner
            .lock()
            .get(endpoint)
            .and_then(|lease| lease.ready.get())
            .cloned()
    }

    /// Input-side observation only: never wait on the endpoint's SSH
    /// deployment lock while rendering an explain buffer.
    pub(crate) fn admitting(&self, endpoint: &RemoteEndpoint) -> bool {
        self.inner
            .lock()
            .get(endpoint)
            .is_some_and(|lease| lease.deploying.try_lock().is_none())
    }

    /// Admit a worker for one endpoint, deploying consent-gated on
    /// first use. `action` is the explicit authorized worker-using
    /// action the user took (recorded on the deployment receipt).
    /// Every failure is the state machine's typed outcome mapped onto
    /// the filesystem taxonomy — the operation never falls through to
    /// the legacy helper, SFTP or a guessed local path.
    pub(crate) fn admit(
        &self,
        endpoint: &RemoteEndpoint,
        action: &str,
        token: &CancelToken,
    ) -> Result<RemoteWorker, FsFailure> {
        let lease = {
            let mut endpoints = self.inner.lock();
            Arc::clone(
                endpoints
                    .entry(endpoint.clone())
                    .or_insert_with(|| Arc::new(EndpointLease::default())),
            )
        };
        if let Some(ready) = lease.ready.get() {
            return Ok(ready.worker.clone());
        }
        // The network/deployment work holds ONLY this endpoint's lock.
        // Lookups and unrelated endpoints remain nonblocking.
        let _deploying = lease.deploying.lock();
        if let Some(ready) = lease.ready.get() {
            return Ok(ready.worker.clone());
        }
        let facts = strop_remote::bootstrap::discover(endpoint, token).map_err(|error| {
            failure(
                FsFailureKind::Permission,
                format!("worker unavailable on {endpoint}: {error}"),
            )
        })?;
        let Some(target) = facts.local_binary_target() else {
            return Err(failure(
                FsFailureKind::Unsupported,
                format!(
                    "no worker artifact for {}/{} on this build; obtain the matching artifact through the release pipeline",
                    facts.system, facts.machine
                ),
            ));
        };
        // A same-target install supplies its own executable; a foreign
        // endpoint needs the explicit administrator-provisioned artifact.
        // Either path still binds exact bytes and target at activation.
        let binary = select_binary(target)?;
        let catalog = catalog_for(target, &binary)?;
        let provider =
            strop_remote::deploy_provider::SftpDeployProvider::connect(endpoint, &facts, token)
                .map_err(|error| {
                    failure(
                        FsFailureKind::Io,
                        format!("deployment channel to {endpoint}: {error}"),
                    )
                })?;
        let report = deploy(
            &provider,
            &DeployRequest {
                catalog,
                editor_version: env!("CARGO_PKG_VERSION").to_string(),
                consent: Consent::Granted {
                    action: action.to_string(),
                },
                supply: ArtifactSupply::LocalBinary { path: binary },
                // The client upload works without remote internet; the
                // release pipeline is not consulted for a same-binary
                // supply.
                online: false,
            },
        );
        match report.outcome {
            DeployOutcome::Probed(ready) => {
                let worker = RemoteWorker::connect(endpoint, &ready.object.path, target);
                // The deployment handshake was a stopped probe. Do not
                // publish this endpoint as ready until the actual worker
                // has handshaken and registered its own cache lease.
                worker.worker().capabilities().map_err(|error| {
                    failure(
                        FsFailureKind::Io,
                        format!("live worker at {endpoint} could not start: {error}"),
                    )
                })?;
                let maintenance = worker
                    .worker()
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
                let info = Arc::new(WorkerReady {
                    worker: worker.clone(),
                    target: target.to_owned(),
                    artifact: ready.object,
                });
                let published = lease.ready.set(info);
                debug_assert!(published.is_ok(), "one deployment per endpoint lock");
                Ok(worker)
            }
            DeployOutcome::Fallback(fallback) => {
                Err(failure(FsFailureKind::Unsupported, fallback.to_string()))
            }
            DeployOutcome::Refused(refusal) => Err(map_refusal(refusal)),
            DeployOutcome::PublishedNotReady { reason, .. } => Err(failure(
                FsFailureKind::Io,
                format!("worker published on {endpoint} but not ready: {reason}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    /// The synthesized same-binary catalog parses and binds this build:
    /// exact version, protocol, target and the artifact entry's digest
    /// over the exact supply bytes (WK07 — a malformed catalog here
    /// silently refused every remote mutation behind one Protocol
    /// message).
    #[test]
    fn synthesized_catalog_parses_and_binds_this_build() {
        let directory = tempfile::tempdir().expect("tempdir");
        let binary = directory.path().join("strop");
        std::fs::write(&binary, b"worker bytes").expect("write");
        let target = strop_remote::bootstrap::local_target().expect("a shipping target");
        let catalog = super::catalog_for(target, &binary).expect("catalog parses");
        assert_eq!(catalog.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            catalog.worker.min_editor,
            strop_worker_deploy::MIN_EDITOR_VERSION
        );
        match catalog.compatibility(
            env!("CARGO_PKG_VERSION"),
            strop_worker_protocol::PROTOCOL_VERSION,
            target,
        ) {
            strop_worker_deploy::Compatibility::Compatible(artifact) => {
                assert_eq!(artifact.target, target);
                assert_eq!(artifact.bytes, 12);
            }
            other => panic!("the catalog must bind this build, got {other:?}"),
        }
        // A foreign editor version is an honest fallback, never a deploy.
        assert!(matches!(
            catalog.compatibility("0.0.0", strop_worker_protocol::PROTOCOL_VERSION, target),
            strop_worker_deploy::Compatibility::Fallback(_)
        ));
    }

    /// An editor event may check a lease while that endpoint is still
    /// deploying. It must get "not admitted" immediately rather than
    /// waiting behind the SSH/upload job on the input→render path.
    #[test]
    fn lookup_does_not_park_behind_an_endpoint_deployment() {
        use super::*;
        let workers = RemoteWorkers::default();
        let endpoint = RemoteEndpoint::parse("ssh://test.example").unwrap();
        let lease = Arc::new(EndpointLease::default());
        workers
            .inner
            .lock()
            .insert(endpoint.clone(), Arc::clone(&lease));
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let deployment = std::thread::spawn(move || {
            let _guard = lease.deploying.lock();
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        held_rx.recv().unwrap();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let lookup = std::thread::spawn(move || reply_tx.send(workers.get(&endpoint)).unwrap());
        assert!(reply_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("lookup waited for the deployment")
            .is_none());
        release_tx.send(()).unwrap();
        deployment.join().unwrap();
        lookup.join().unwrap();
    }
}
