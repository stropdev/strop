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
use std::sync::{Arc, Mutex};

use sha2::Digest as _;

use strop_core::worker::CancelToken;
use strop_remote::worker_transport::RemoteWorker;
use strop_worker_deploy::deploy::{
    deploy, ArtifactSupply, Consent, DeployOutcome, DeployRefusal, DeployRequest,
};
use strop_worker_deploy::manifest::ReleaseCatalog;
use strop_workspace::operation::{FsFailure, FsFailureKind};
use strop_workspace::RemoteEndpoint;

/// The session's remote worker leases, shared with the filesystem jobs
/// that route through them. Cheap to clone; one deployment per
/// endpoint per session.
#[derive(Clone, Default)]
pub(crate) struct RemoteWorkers {
    inner: Arc<Mutex<HashMap<RemoteEndpoint, RemoteWorker>>>,
}

fn lock(
    workers: &Mutex<HashMap<RemoteEndpoint, RemoteWorker>>,
) -> std::sync::MutexGuard<'_, HashMap<RemoteEndpoint, RemoteWorker>> {
    workers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}

impl RemoteWorkers {
    /// The admitted lease for one endpoint, when the host admits a
    /// worker. Read paths route through this; its absence means the
    /// read-only SFTP path serves exactly as before.
    pub(crate) fn get(&self, endpoint: &RemoteEndpoint) -> Option<RemoteWorker> {
        lock(&self.inner).get(endpoint).cloned()
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
        // One admission per endpoint at a time: concurrent mutations
        // must not race two deployments of the same artifact.
        let mut workers = lock(&self.inner);
        if let Some(worker) = workers.get(endpoint) {
            return Ok(worker.clone());
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
        // Supply: this install's own binary for a same-target endpoint.
        // STROP_WORKER_BINARY is the explicit administrator-provisioned
        // override (0058 §5; the test lanes use it for the stripped or
        // static worker build of this exact checkout): the override's
        // bytes are hashed, bound by the catalog entry and proven by
        // the remote handshake exactly like the default — an invalid
        // override never chooses another file.
        let binary = match std::env::var_os("STROP_WORKER_BINARY") {
            Some(override_path) => std::path::PathBuf::from(override_path),
            None => std::env::current_exe().map_err(|error| {
                failure(
                    FsFailureKind::Io,
                    format!("this install's own binary is unavailable: {error}"),
                )
            })?,
        };
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
            DeployOutcome::Ready(ready) => {
                let worker = RemoteWorker::connect(endpoint, &ready.object.path, target);
                workers.insert(endpoint.clone(), worker.clone());
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

/// The deploy refusal mapped onto the filesystem taxonomy, keeping the
/// precise reason text (selected target, version, destination).
fn map_refusal(refusal: DeployRefusal) -> FsFailure {
    let kind = match &refusal {
        DeployRefusal::ConsentRequired { .. } => FsFailureKind::Permission,
        DeployRefusal::OfflineNoArtifact { .. }
        | DeployRefusal::ArtifactNotStaged { .. }
        | DeployRefusal::PreinstalledInvalid { .. } => FsFailureKind::Unsupported,
        DeployRefusal::IdentityMismatch { .. } => FsFailureKind::Protocol,
        _ => FsFailureKind::Io,
    };
    failure(kind, refusal.to_string())
}

/// The catalog of this build's own release for the same-binary supply:
/// the artifact entry's digest and size are computed over the exact
/// local bytes — the caller-side supply verification the deployment
/// contract requires (0058 §5: this install's own binary for a
/// same-target endpoint).
fn catalog_for(target: &str, binary: &std::path::Path) -> Result<ReleaseCatalog, FsFailure> {
    let unreadable = |error: std::io::Error| {
        failure(
            FsFailureKind::Io,
            format!("cannot read this install's binary: {error}"),
        )
    };
    let bytes = std::fs::read(binary).map_err(unreadable)?;
    let version = env!("CARGO_PKG_VERSION");
    let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    let sha256 = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let body = serde_json::json!({
        "schema": 1,
        "product": "strop",
        "version": version,
        "tag": format!("v{version}"),
        "published_at": "1970-01-01T00:00:00Z",
        "artifacts": [{
            "target": target,
            "name": format!("strop-{version}-{target}.tar.gz"),
            "sha256": sha256,
            "bytes": bytes.len(),
            "url": "",
        }],
        "worker": {
            "protocol": strop_worker_protocol::PROTOCOL_VERSION,
            "min_editor": strop_worker_deploy::MIN_EDITOR_VERSION,
            "targets": [target],
        },
    })
    .to_string();
    ReleaseCatalog::parse(body.as_bytes()).map_err(|error| {
        failure(
            FsFailureKind::Protocol,
            format!("this build's worker catalog does not parse: {error}"),
        )
    })
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
}
