//! Worker deployment and cache state machine (0058 WK06; the normative
//! contract is `plans/0058-unified-native-worker.md` §5).
//!
//! Deployment is part of the product, not a shell script: the first deploy
//! to an endpoint is consent-gated and endpoint/principal-bound, artifacts
//! live in a private (0700) per-user content-addressed cache, bytes travel
//! over the already-authenticated provider channel (SFTP upload / container
//! tar are thin provider implementations), and an object is activated only
//! after its digest, provenance, target and executable mode verify against
//! the WK05 release-catalog manifest — at its final published path, never
//! stage-then-blind-exec. Activation, execution and readiness are separate
//! outcomes: upload finishing never reports installed/ready; only a real
//! protocol handshake whose reported identity matches the client's exact
//! release/target captures the lease.
//!
//! Hard rules held by construction:
//! - no PATH/shell-rc/project-tree/image/package-database mutation — the
//!   provider surface has no such operation;
//! - no noexec/upload-policy evasion (no memfd, remount or chmod-away
//!   tricks): a noexec cache or refused launch is a truthful refusal;
//! - offline (release-pipeline unreachable) with no verified local or
//!   preinstalled artifact is a useful refusal, never a remote network or
//!   install attempt;
//! - source payloads and credentials never appear in receipts — receipts
//!   record endpoint, principal, version, target and digests only.
//!
//! The providers are thin; the state machine in [`deploy`] is the product.
//! Cache garbage collection ([`gc`]) is receipt-scoped and lease-aware: a
//! live lease's object is never retired, and cleanup only ever touches
//! positively identified content-addressed entries.

pub mod cache;
pub mod container;
pub mod deploy;
pub mod gc;
pub mod manifest;
pub mod provider;

pub use container::{ContainerProvider, ShellPolicy};
pub use deploy::{
    deploy, ArtifactSupply, Consent, DeployOrigin, DeployOutcome, DeployRefusal, DeployReport,
    DeployRequest, State, VerifiedObject,
};
pub use manifest::{Compatibility, Fallback, ReleaseCatalog, WorkerManifest};

/// Oldest editor release that can drive a worker at all (0058 WK05). The
/// release workflow extracts this constant into the catalog's worker
/// manifest (`.github/workflows/release.yml` greps this exact line); an
/// older editor falls back to its non-worker capabilities instead of
/// attempting a deploy.
pub const MIN_EDITOR_VERSION: &str = "0.35.0";

/// Hard ceiling on worker object bytes accepted for upload and read-back
/// (0058 §5: the stage has a bounded length before any byte is accepted).
/// Release binaries are an order of magnitude under this.
pub const MAX_WORKER_BYTES: u64 = 256 * 1024 * 1024;

/// Ceiling on receipt/lease payloads — these are tiny facts documents.
pub const MAX_RECORD_BYTES: u64 = 16 * 1024;
