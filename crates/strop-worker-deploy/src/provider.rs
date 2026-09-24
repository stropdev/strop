//! The authenticated provider surface deployment runs over (0058 WK06).
//!
//! Providers are thin: an SFTP upload session and a container scoped
//! file-transfer/exec facility both implement this trait with near-direct
//! mappings (`put`/`get`/`rename`/`chmod`/`stat`/`readdir`, tar-in/tar-out
//! plus scoped exec). Authentication, host-key policy and connection
//! ownership stay in the established transports (WK07/WK08 wire the real
//! providers); this crate never opens a connection of its own.
//!
//! The surface deliberately has no operation for PATH/rc/image/package
//! mutation, no recursive delete and no unbounded read: what the state
//! machine cannot ask for, a provider cannot be tricked into doing.

use std::path::Path;

use strop_worker_protocol::{EndpointInfo, Session};

/// Where the cache lives and who owns it. The context binds consent,
/// receipts and leases to one admitted endpoint incarnation (SSH alias or
/// canonical container ID + StartedAt per 0056); a renamed/restarted
/// context is a different endpoint and fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointIdentity {
    /// Stable connection context (SSH alias, `docker:<id>@<started-at>`).
    pub context: String,
    /// Remote principal the deployment runs as.
    pub principal: String,
    /// Remote target triple the worker artifact must match exactly.
    pub target: String,
}

/// Remote entry kind, as reported by the provider's `lstat` equivalent
/// (never follows symlinks — the symlink policy is enforced on this fact).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// Metadata for one remote path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteStat {
    pub kind: RemoteKind,
    /// Unix permission bits.
    pub mode: u32,
    /// Owning principal name.
    pub owner: String,
    pub len: u64,
}

impl RemoteStat {
    pub fn owner_executable(&self) -> bool {
        self.kind == RemoteKind::File && self.mode & 0o100 != 0
    }
}

/// Classified provider failures. The classification is load-bearing: the
/// state machine turns each kind into its explicit outcome (disk full,
/// read-only/noexec cache, corrupt transfer, interrupted launch) instead
/// of one opaque "deploy failed".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// Authenticated channel failure (connection dropped, remote error).
    #[error("transport: {0}")]
    Transport(String),
    /// The path does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The cache filesystem rejects writes (read-only mount, policy).
    #[error("read-only cache: {0}")]
    ReadOnly(String),
    /// No space for the staged bytes.
    #[error("disk full: {0}")]
    DiskFull(String),
    /// Ownership/permission refusal from the remote filesystem.
    #[error("permission denied: {0}")]
    Permission(String),
    /// Launch refused: noexec mount, restricted account or policy. Never
    /// evaded, always reported.
    #[error("launch refused: {0}")]
    NoExec(String),
    /// The process ran but the protocol handshake did not complete or
    /// failed to decode. Shell/interpreter diagnostics can never
    /// impersonate a worker Welcome through this channel.
    #[error("handshake failed: {0}")]
    Handshake(String),
}

/// What the worker's real handshake reported (WK02 Welcome facts plus the
/// granted session authority). Deployment binds these to the client's
/// exact release/target — see [`crate::deploy::activate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeReport {
    pub protocol: u32,
    pub worker: EndpointInfo,
    pub session: Session,
}

/// The authenticated transfer/exec surface for one endpoint. Every method
/// is a thin primitive the SFTP and container-tar providers can honor;
/// byte counts are always bounded by the caller.
pub trait DeployProvider {
    /// The admitted endpoint this provider is bound to.
    fn endpoint(&self) -> &EndpointIdentity;

    /// The principal's per-user cache base in native spelling
    /// (e.g. `/home/alice/.cache`). Deployment never invents it.
    fn cache_base(&self) -> Result<String, ProviderError>;

    /// Symlink-conscious metadata; `None` when the path is absent.
    fn lstat(&self, path: &str) -> Result<Option<RemoteStat>, ProviderError>;

    /// Create a private (0700) directory, parents included; existing
    /// components are left untouched (the caller validates them).
    fn mkdir_private(&self, path: &str) -> Result<(), ProviderError>;

    /// Authenticated byte transfer from a verified local file to a remote
    /// staging path (SFTP `put`, container tar-in). The source is always
    /// the client's own verified artifact; payloads never appear in
    /// command arguments or logs.
    fn upload(&self, source: &Path, dest: &str) -> Result<(), ProviderError>;

    /// Write a small facts payload (receipt, lease record).
    fn write(&self, dest: &str, bytes: &[u8]) -> Result<(), ProviderError>;

    /// Bounded read-back for verification (SFTP `get`, container tar-out).
    /// Fails rather than returning more than `max` bytes.
    fn fetch(&self, path: &str, max: u64) -> Result<Vec<u8>, ProviderError>;

    /// Set permission bits on one positively identified path.
    fn set_mode(&self, path: &str, mode: u32) -> Result<(), ProviderError>;

    /// Atomic rename within the cache filesystem (publish). Content-
    /// addressed destinations make a concurrent identical publish
    /// harmless; a partial stage is never renamed into place.
    fn rename(&self, from: &str, to: &str) -> Result<(), ProviderError>;

    /// Remove one positively identified file. Never recursive, never a
    /// glob — GC and failure cleanup name exact paths only.
    fn remove(&self, path: &str) -> Result<(), ProviderError>;

    /// Entry names (not paths) in one directory, for bounded GC.
    fn list(&self, dir: &str) -> Result<Vec<String>, ProviderError>;

    /// Launch the published object directly into worker mode
    /// (`--worker-stdio`, no shell supervisor) and run the real protocol
    /// handshake, returning the reported identity and granted session.
    fn handshake(&self, object: &str) -> Result<HandshakeReport, ProviderError>;
}
