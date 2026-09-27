//! The deploy/cache state machine (0058 §5, WK06). One straight-line
//! machine with explicit recorded states; every interruption cleans up
//! only positively owned staging, retains old entries, and never reports
//! partial activation.
//!
//! States, in order (plan steps 1–6):
//!
//! 1. [`State::Decide`] — the WK05 deploy-vs-fallback decision plus
//!    endpoint/principal capture. [`State::ResolveCacheRoot`] validates
//!    the authorized private cache root.
//! 2. [`State::CheckCache`] — reuse an exact verified existing object
//!    (quietly, no fresh consent), retire a positively identified corrupt
//!    one, or require consent for a first upload.
//! 3. [`State::Upload`] + [`State::VerifyTransfer`] — bytes travel the
//!    authenticated provider into a uniquely owned private stage with a
//!    hard length bound; read-back verification proves the staged bytes.
//! 4. [`State::Publish`] — atomic rename to the content-addressed object
//!    path; [`State::VerifyObject`] re-verifies digest/mode/size at the
//!    final path, binding verification to the exact object that will be
//!    executed (never stage-then-blind-exec).
//! 5. [`State::WriteReceipt`] — provenance/binding facts only.
//! 6. [`State::Activate`] — a short-lived real handshake proves that the
//!    object can serve this release/target. Its process is stopped;
//!    the caller must connect the actual worker before publishing ready.

use std::path::{Path, PathBuf};

use sha2::Digest;
use strop_worker_protocol::PROTOCOL_VERSION;

use crate::cache::{self, CacheError, CacheLayout};
use crate::manifest::{ArtifactManifest, Compatibility, ReleaseCatalog};
use crate::provider::{DeployProvider, HandshakeReport, ProviderError, RemoteKind};
use crate::MAX_WORKER_BYTES;
use strop_core::worker::cache_record::{CacheReceipt, RECEIPT_SCHEMA};

/// The recorded states of one deploy run (the trace travels with the
/// outcome so an interrupted deploy is observable, never silent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Decide,
    StageLocal,
    ResolveCacheRoot,
    CheckCache,
    Upload,
    VerifyTransfer,
    Publish,
    VerifyObject,
    WriteReceipt,
    Activate,
}

/// Endpoint/principal-bound consent for a first deployment (0058 §5: an
/// explicit authorized worker-using action; browsing a read-only host
/// never uploads or executes code).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Consent {
    /// The authorized action the user took (for the receipt's audit
    /// trail), e.g. `"open-remote-workspace"`.
    Granted {
        action: String,
    },
    Absent,
}

/// Where the worker bytes come from. This crate never downloads on the
/// remote and never resolves `latest` — the local bytes must already be
/// the verified artifact (WK05 exact release/target binding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactSupply {
    /// Verified local worker binary: this install's own binary for a
    /// same-target endpoint, or the matching-target artifact obtained
    /// through the local release pipeline (digest-verified against the
    /// catalog by the caller's install/update machinery).
    LocalBinary { path: PathBuf },
    /// Explicit administrator-provisioned object at an exact endpoint
    /// path, validated under the same compatibility/ownership rules. An
    /// invalid override never chooses another file.
    Preinstalled { path: String },
    /// No verified local bytes.
    None,
}

/// One deployment request.
#[derive(Debug, Clone)]
pub struct DeployRequest {
    /// The catalog of this editor's own release (exact binding).
    pub catalog: ReleaseCatalog,
    /// This editor's version (== the catalog's, enforced by `decide`).
    pub editor_version: String,
    pub consent: Consent,
    pub supply: ArtifactSupply,
    /// Release-pipeline reachability, *not* endpoint connectivity: a
    /// remote host without internet still works via client upload.
    pub online: bool,
}

/// A worker object whose digest/mode/size verified at its final path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedObject {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// How the activated object got there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployOrigin {
    /// Compatible verified cache hit, quietly reused.
    Reused,
    /// Fresh consent-gated (or previously authorized) upload.
    Uploaded,
    /// Administrator-provisioned path, validated in place.
    Preinstalled,
}

/// Verified object and successful short-lived deployment probe. The
/// caller connects its own worker before publishing a live lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbedDeployment {
    pub object: VerifiedObject,
    pub origin: DeployOrigin,
    pub handshake: HandshakeReport,
}

/// The honest outcome of one deploy run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployOutcome {
    /// Object verified and the temporary probe's handshake accepted.
    /// This is not the editor's live worker session.
    Probed(ProbedDeployment),
    /// The object was published, but the probe launch, handshake or
    /// cleanup failed: installed ≠ usable. The object remains cached.
    PublishedNotReady {
        object: VerifiedObject,
        origin: DeployOrigin,
        reason: String,
    },
    /// Deploy-vs-fallback: no worker here; the namespace's non-worker
    /// capabilities remain. Not an error.
    Fallback(crate::manifest::Fallback),
    /// Truthful refusal with the precise reason.
    Refused(DeployRefusal),
}

/// The full report: outcome plus the states the machine reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployReport {
    pub outcome: DeployOutcome,
    pub trace: Vec<State>,
}

fn report(outcome: DeployOutcome, trace: Vec<State>) -> DeployReport {
    DeployReport { outcome, trace }
}

/// Truthful refusal reasons. Each names the precise cause (and, where
/// the user must act, the selected target, artifact version, destination
/// and reason, per 0058 §5).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeployRefusal {
    /// First upload to this endpoint needs an explicit authorized action.
    #[error("first worker deployment to {context} (as {principal}) requires an explicit authorized action; selected target {target}, worker {version}, destination {destination}")]
    ConsentRequired {
        context: String,
        principal: String,
        version: String,
        target: String,
        destination: String,
    },
    /// Both ends offline and nothing verified to run: a useful refusal,
    /// never a remote network/install attempt.
    #[error(
        "offline and no locally cached or preinstalled verified worker for {version}/{target}"
    )]
    OfflineNoArtifact { version: String, target: String },
    /// Online, but the caller has not obtained the verified local
    /// artifact through the release pipeline yet.
    #[error("no verified local worker artifact for {version}/{target}; obtain it through the release pipeline first")]
    ArtifactNotStaged { version: String, target: String },
    /// The explicit preinstalled override failed validation. Never
    /// chooses another file.
    #[error("preinstalled worker at {path} refused: {reason}")]
    PreinstalledInvalid { path: String, reason: String },
    /// The handshake's reported identity does not bind to this client's
    /// exact release/target — a refusal, never a negotiated downgrade.
    #[error("worker identity mismatch: {field} is {reported}, expected {expected}")]
    IdentityMismatch {
        field: &'static str,
        expected: String,
        reported: String,
    },
    /// The local artifact exceeds the stage's bounded length.
    #[error("worker artifact is {bytes} bytes, over the {max}-byte bound")]
    Oversized { bytes: u64, max: u64 },
    /// The local verified artifact cannot be read.
    #[error("cannot read local artifact {path}: {reason}")]
    LocalUnreadable { path: String, reason: String },
    /// Staged bytes did not survive the transfer intact.
    #[error("staged bytes failed read-back verification at {path}")]
    CorruptTransfer { path: String },
    /// The published object failed verification at its final path.
    #[error("published object failed verification at {path}")]
    CorruptPublished { path: String },
    #[error(transparent)]
    Cache(#[from] CacheError),
    /// A provider operation failed at the named state (disk full,
    /// read-only/noexec cache, transport loss, interrupted launch — the
    /// classification travels in the error).
    #[error("provider failure during {state:?}: {error}")]
    Transfer { state: State, error: ProviderError },
}

fn at(state: State) -> impl Fn(ProviderError) -> DeployRefusal {
    move |error| DeployRefusal::Transfer { state, error }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn unique_staging_name() -> Result<String, DeployRefusal> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| DeployRefusal::Transfer {
        state: State::Upload,
        error: ProviderError::Transport(format!("staging name rng: {error}")),
    })?;
    let mut hex = String::with_capacity(32);
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(hex)
}

fn hash_local(path: &Path) -> Result<(String, u64), DeployRefusal> {
    let unreadable = |reason: std::io::Error| DeployRefusal::LocalUnreadable {
        path: path.display().to_string(),
        reason: reason.to_string(),
    };
    let len = std::fs::metadata(path).map_err(unreadable)?.len();
    if len > MAX_WORKER_BYTES {
        return Err(DeployRefusal::Oversized {
            bytes: len,
            max: MAX_WORKER_BYTES,
        });
    }
    let bytes = std::fs::read(path).map_err(unreadable)?;
    Ok((sha256_hex(&bytes), bytes.len() as u64))
}

/// Verify a remote object against its expected digest: regular file,
/// owner-executable, exact size and exact bytes, read back over the
/// authenticated channel. Used both for cache hits and — load-bearing —
/// for the published object at its final path before activation.
fn verify_remote(
    provider: &impl DeployProvider,
    path: &str,
    expected_sha256: &str,
) -> Result<VerifiedObject, DeployRefusal> {
    let stat = provider
        .lstat(path)
        .map_err(at(State::VerifyObject))?
        .ok_or_else(|| DeployRefusal::CorruptPublished {
            path: path.to_string(),
        })?;
    if stat.kind != RemoteKind::File || !stat.owner_executable() {
        return Err(DeployRefusal::CorruptPublished {
            path: path.to_string(),
        });
    }
    let bytes = provider
        .fetch(path, MAX_WORKER_BYTES)
        .map_err(at(State::VerifyObject))?;
    if bytes.len() as u64 != stat.len || sha256_hex(&bytes) != expected_sha256 {
        return Err(DeployRefusal::CorruptPublished {
            path: path.to_string(),
        });
    }
    Ok(VerifiedObject {
        path: path.to_string(),
        sha256: expected_sha256.to_string(),
        bytes: stat.len,
    })
}

/// Run one deployment. Never fails silently: every path lands in a
/// `Ready`, `PublishedNotReady`, `Fallback` or `Refused` outcome with the
/// states the machine reached.
pub fn deploy(provider: &impl DeployProvider, request: &DeployRequest) -> DeployReport {
    let mut trace = vec![State::Decide];
    let endpoint = provider.endpoint().clone();
    let artifact = match request.catalog.compatibility(
        &request.editor_version,
        PROTOCOL_VERSION,
        &endpoint.target,
    ) {
        Compatibility::Fallback(fallback) => {
            return report(DeployOutcome::Fallback(fallback), trace);
        }
        Compatibility::Compatible(artifact) => artifact.clone(),
    };
    let outcome = match &request.supply {
        ArtifactSupply::None => {
            let (version, target) = (request.catalog.version.clone(), endpoint.target.clone());
            DeployOutcome::Refused(if request.online {
                DeployRefusal::ArtifactNotStaged { version, target }
            } else {
                DeployRefusal::OfflineNoArtifact { version, target }
            })
        }
        ArtifactSupply::Preinstalled { path } => {
            deploy_preinstalled(provider, request, path, &mut trace)
        }
        ArtifactSupply::LocalBinary { path } => {
            deploy_local(provider, request, &artifact, path, &mut trace)
        }
    };
    report(outcome, trace)
}

/// The cache-backed machine: reuse-or-upload, publish, verify, activate.
fn deploy_local(
    provider: &impl DeployProvider,
    request: &DeployRequest,
    artifact: &ArtifactManifest,
    local: &Path,
    trace: &mut Vec<State>,
) -> DeployOutcome {
    trace.push(State::StageLocal);
    let (sha256, bytes) = match hash_local(local) {
        Ok(facts) => facts,
        Err(refusal) => return DeployOutcome::Refused(refusal),
    };
    trace.push(State::ResolveCacheRoot);
    let layout = match cache::resolve(provider) {
        Ok(layout) => layout,
        Err(error) => return DeployOutcome::Refused(error.into()),
    };
    let endpoint = provider.endpoint();
    trace.push(State::CheckCache);
    let authorized =
        match cache::authorized(provider, &layout, &endpoint.context, &endpoint.principal) {
            Ok(authorized) => authorized,
            Err(error) => return DeployOutcome::Refused(at(State::CheckCache)(error)),
        };
    let object_path = layout.object(&sha256);
    match provider.lstat(&object_path) {
        Err(error) => return DeployOutcome::Refused(at(State::CheckCache)(error)),
        Ok(Some(stat)) if stat.kind != RemoteKind::File => {
            return DeployOutcome::Refused(CacheError::ForeignEntry(object_path).into());
        }
        Ok(Some(_)) => match verify_remote(provider, &object_path, &sha256) {
            Ok(object) => {
                // Previously authorized or not, an exact verified object in
                // the principal's own private cache is quietly reused.
                return publish_receipt_and_activate(
                    provider,
                    &layout,
                    object,
                    artifact,
                    request,
                    DeployOrigin::Reused,
                    trace,
                );
            }
            Err(DeployRefusal::CorruptPublished { .. }) => {
                // A positively identified corrupt cache object: retire it
                // and redeploy. Nothing else in the cache is touched.
                if let Err(error) = provider.remove(&object_path) {
                    return DeployOutcome::Refused(at(State::CheckCache)(error));
                }
            }
            Err(refusal) => return DeployOutcome::Refused(refusal),
        },
        Ok(None) => {}
    }
    if !authorized && matches!(request.consent, Consent::Absent) {
        return DeployOutcome::Refused(DeployRefusal::ConsentRequired {
            context: endpoint.context.clone(),
            principal: endpoint.principal.clone(),
            version: request.catalog.version.clone(),
            target: endpoint.target.clone(),
            destination: object_path,
        });
    }
    let staging_name = match unique_staging_name() {
        Ok(name) => name,
        Err(refusal) => return DeployOutcome::Refused(refusal),
    };
    let staging_path = layout.staging(&staging_name);
    match staged_upload(
        provider,
        &layout,
        local,
        &sha256,
        bytes,
        &staging_path,
        trace,
    ) {
        Ok(object) => publish_receipt_and_activate(
            provider,
            &layout,
            object,
            artifact,
            request,
            DeployOrigin::Uploaded,
            trace,
        ),
        Err(refusal) => {
            // Failure removes only positively owned staging; old entries
            // and concurrent publishers are never touched.
            let _ = provider.remove(&staging_path);
            DeployOutcome::Refused(refusal)
        }
    }
}

/// States 3–4: bounded staged upload, read-back transfer verification,
/// atomic publish, verification at the final path.
fn staged_upload(
    provider: &impl DeployProvider,
    layout: &CacheLayout,
    local: &Path,
    sha256: &str,
    bytes: u64,
    staging_path: &str,
    trace: &mut Vec<State>,
) -> Result<VerifiedObject, DeployRefusal> {
    trace.push(State::Upload);
    provider
        .upload(local, staging_path)
        .map_err(at(State::Upload))?;
    provider
        .set_mode(staging_path, 0o500)
        .map_err(at(State::Upload))?;
    trace.push(State::VerifyTransfer);
    let staged = provider
        .fetch(staging_path, MAX_WORKER_BYTES)
        .map_err(at(State::VerifyTransfer))?;
    if staged.len() as u64 != bytes || sha256_hex(&staged) != sha256 {
        return Err(DeployRefusal::CorruptTransfer {
            path: staging_path.to_string(),
        });
    }
    trace.push(State::Publish);
    let object_path = layout.object(sha256);
    provider
        .rename(staging_path, &object_path)
        .map_err(at(State::Publish))?;
    trace.push(State::VerifyObject);
    verify_remote(provider, &object_path, sha256)
}

/// State 5 (receipt) and 6 (activation): receipt first so a crash after
/// publish never leaves an undocumented object; then the handshake.
fn publish_receipt_and_activate(
    provider: &impl DeployProvider,
    layout: &CacheLayout,
    object: VerifiedObject,
    artifact: &ArtifactManifest,
    request: &DeployRequest,
    origin: DeployOrigin,
    trace: &mut Vec<State>,
) -> DeployOutcome {
    trace.push(State::WriteReceipt);
    let endpoint = provider.endpoint();
    let receipt = CacheReceipt {
        schema: RECEIPT_SCHEMA,
        context: endpoint.context.clone(),
        principal: endpoint.principal.clone(),
        version: request.catalog.version.clone(),
        target: endpoint.target.clone(),
        object_sha256: object.sha256.clone(),
        object_bytes: object.bytes,
        tarball_sha256: artifact.sha256.clone(),
    };
    let write_receipt = || -> Result<(), DeployRefusal> {
        if cache::read_receipt(provider, layout, &object.sha256)
            .map_err(at(State::WriteReceipt))?
            .as_ref()
            != Some(&receipt)
        {
            let bytes = serde_json::to_vec(&receipt).map_err(|error| DeployRefusal::Transfer {
                state: State::WriteReceipt,
                error: ProviderError::Transport(format!("receipt encoding: {error}")),
            })?;
            provider
                .write(&layout.receipt(&object.sha256), &bytes)
                .map_err(at(State::WriteReceipt))?;
        }
        Ok(())
    };
    if let Err(refusal) = write_receipt() {
        return DeployOutcome::Refused(refusal);
    }
    // The staged-activation kernel: a published, digest-verified object
    // with a matching receipt is the only launch permission. The
    // VerifiedObject carries the first two premises; re-reading the
    // receipt checks that the third is still present and matches this
    // release/target/object, not just that some JSON record parses.
    let receipted = match cache::read_receipt(provider, layout, &object.sha256) {
        Ok(Some(stored)) => stored == receipt,
        Ok(None) => false,
        Err(error) => return DeployOutcome::Refused(at(State::WriteReceipt)(error)),
    };
    if !strop_core::worker::deploy_policy::activation_admitted(true, true, receipted) {
        return DeployOutcome::Refused(DeployRefusal::Transfer {
            state: State::WriteReceipt,
            error: ProviderError::Transport("activation lost its matching receipt premise".into()),
        });
    }
    trace.push(State::Activate);
    match activate(provider, Some(layout), &object, &request.editor_version) {
        Ok(handshake) => DeployOutcome::Probed(ProbedDeployment {
            object,
            origin,
            handshake,
        }),
        Err(refusal) => DeployOutcome::PublishedNotReady {
            object,
            origin,
            reason: refusal.to_string(),
        },
    }
}

/// Preinstalled override: validate in place under the same
/// compatibility/ownership rules, then activate. No cache writes, no
/// substitute file, ever.
fn deploy_preinstalled(
    provider: &impl DeployProvider,
    request: &DeployRequest,
    path: &str,
    trace: &mut Vec<State>,
) -> DeployOutcome {
    let invalid = |reason: &str| {
        DeployOutcome::Refused(DeployRefusal::PreinstalledInvalid {
            path: path.to_string(),
            reason: reason.to_string(),
        })
    };
    trace.push(State::VerifyObject);
    let stat = match provider.lstat(path) {
        Ok(Some(stat)) => stat,
        Ok(None) => return invalid("no such file"),
        Err(error) => return DeployOutcome::Refused(at(State::VerifyObject)(error)),
    };
    if stat.kind == RemoteKind::Symlink {
        return invalid("a symlink is a replaceable path");
    }
    if stat.kind != RemoteKind::File {
        return invalid("not a regular file");
    }
    if !stat.owner_executable() || stat.mode & 0o022 != 0 {
        return invalid("must be owner-executable and not group/world-writable");
    }
    if stat.len > MAX_WORKER_BYTES {
        return DeployOutcome::Refused(DeployRefusal::Oversized {
            bytes: stat.len,
            max: MAX_WORKER_BYTES,
        });
    }
    let bytes = match provider.fetch(path, MAX_WORKER_BYTES) {
        Ok(bytes) => bytes,
        Err(error) => return DeployOutcome::Refused(at(State::VerifyObject)(error)),
    };
    let object = VerifiedObject {
        path: path.to_string(),
        sha256: sha256_hex(&bytes),
        bytes: bytes.len() as u64,
    };
    trace.push(State::Activate);
    match activate(provider, None, &object, &request.editor_version) {
        Ok(handshake) => DeployOutcome::Probed(ProbedDeployment {
            object,
            origin: DeployOrigin::Preinstalled,
            handshake,
        }),
        Err(refusal) => DeployOutcome::Refused(refusal),
    }
}

/// Probe the verified object with a real handshake, bind its reported
/// identity to the exact release/target/protocol, then retire only that
/// temporary session's lease. The caller creates a new worker process:
/// its fresh session registers its own lease before it is exposed.
pub fn activate(
    provider: &impl DeployProvider,
    layout: Option<&CacheLayout>,
    object: &VerifiedObject,
    editor_version: &str,
) -> Result<HandshakeReport, DeployRefusal> {
    let handshake = provider
        .handshake(&object.path)
        .map_err(at(State::Activate))?;
    if let Some(layout) = layout {
        // The probe has been stopped by the provider. It must not pin
        // the object as though it were the editor's later live worker.
        // Its own orderly teardown can race this removal (container rm
        // does not classify ENOENT); re-observe only on failure.
        let path = layout.lease(handshake.session.lease.0);
        if let Err(error) = provider.remove(&path) {
            if provider
                .lstat(&path)
                .map_err(at(State::Activate))?
                .is_some()
            {
                return Err(at(State::Activate)(error));
            }
        }
    }
    let endpoint = provider.endpoint();
    for (field, reported, expected) in [
        (
            "protocol",
            handshake.protocol.to_string(),
            PROTOCOL_VERSION.to_string(),
        ),
        (
            "version",
            handshake.worker.version.clone(),
            editor_version.to_string(),
        ),
        (
            "target",
            handshake.worker.target.clone(),
            endpoint.target.clone(),
        ),
    ] {
        if reported != expected {
            return Err(DeployRefusal::IdentityMismatch {
                field,
                expected,
                reported,
            });
        }
    }
    Ok(handshake)
}
