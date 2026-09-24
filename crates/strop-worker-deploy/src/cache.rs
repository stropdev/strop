//! The private per-user worker cache (0058 §5): one authorized native
//! cache root holding immutable version/target/content-addressed objects,
//! their provenance receipts, uniquely owned staging and live-lease
//! records.
//!
//! Layout (every component private 0700, owned by the principal, no
//! symlinks):
//!
//! ```text
//! <cache-base>/strop-worker/
//!   objects/<sha256>          immutable verified worker objects (0500)
//!   receipts/<sha256>.json    provenance/binding facts per object
//!   staging/<32-hex>          uniquely owned in-flight uploads
//!   leases/<lease-id>.json    live worker leases (GC's keep-set)
//! ```
//!
//! The cache never touches PATH, shell rc files, the project tree, global
//! `/tmp` names, container images or package databases — the provider
//! surface has no such operation.

use serde::{Deserialize, Serialize};

use crate::provider::{DeployProvider, ProviderError, RemoteKind, RemoteStat};
use crate::MAX_RECORD_BYTES;

/// The one component deployment owns under the principal's cache base.
pub const CACHE_DIR_NAME: &str = "strop-worker";

const OBJECTS: &str = "objects";
const RECEIPTS: &str = "receipts";
const STAGING: &str = "staging";
const LEASES: &str = "leases";

/// Receipt schema version.
pub const RECEIPT_SCHEMA: u32 = 1;

/// Provenance/binding facts for one cached object. Digests and identity
/// only — never source payloads or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheReceipt {
    pub schema: u32,
    /// Endpoint context this deployment was authorized for.
    pub context: String,
    /// Principal the deployment runs as.
    pub principal: String,
    /// Exact release the object was deployed from (== the client's).
    pub version: String,
    /// Target triple the object was built for (== the endpoint's).
    pub target: String,
    /// Content address: sha256 of the object bytes.
    pub object_sha256: String,
    pub object_bytes: u64,
    /// Provenance anchor: sha256 of the release-catalog tarball these
    /// bytes were verified against.
    pub tarball_sha256: String,
}

/// One live worker lease: GC never retires the referenced object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub lease: u64,
    pub object_sha256: String,
}

/// Cache resolution/validation failure — each a precise refusal reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    /// The cache base is not a native absolute path.
    #[error("non-native cache path spelling: {0}")]
    Spelling(String),
    /// A path component is a symlink; the parent/symlink policy fails
    /// closed rather than following it.
    #[error("symlink component in cache path: {0}")]
    SymlinkComponent(String),
    /// A required directory is some other kind of entry.
    #[error("not a directory: {0}")]
    NotDirectory(String),
    /// The entry belongs to a different principal.
    #[error("cache entry owned by {owner}, expected {expected}: {path}")]
    NotOwned {
        path: String,
        owner: String,
        expected: String,
    },
    /// A directory deployment owns is not private (0700). Deployment
    /// never chmods a pre-existing entry into compliance; it refuses.
    #[error("cache directory is mode {mode:o}, expected 700: {path}")]
    NotPrivate { path: String, mode: u32 },
    /// A non-file entry occupies a content-addressed object path.
    #[error("foreign entry at object path: {0}")]
    ForeignEntry(String),
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

/// Resolved, validated cache paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheLayout {
    root: String,
}

impl CacheLayout {
    pub fn root(&self) -> &str {
        &self.root
    }
    pub fn objects_dir(&self) -> String {
        format!("{}/{OBJECTS}", self.root)
    }
    pub fn receipts_dir(&self) -> String {
        format!("{}/{RECEIPTS}", self.root)
    }
    pub fn staging_dir(&self) -> String {
        format!("{}/{STAGING}", self.root)
    }
    pub fn leases_dir(&self) -> String {
        format!("{}/{LEASES}", self.root)
    }
    pub fn object(&self, sha256: &str) -> String {
        format!("{}/{sha256}", self.objects_dir())
    }
    pub fn receipt(&self, sha256: &str) -> String {
        format!("{}/{sha256}.json", self.receipts_dir())
    }
    pub fn staging(&self, unique: &str) -> String {
        format!("{}/{unique}", self.staging_dir())
    }
    pub fn lease(&self, lease: u64) -> String {
        format!("{}/{lease}.json", self.leases_dir())
    }
}

/// Native-spelling policy: absolute, no empty/`.`/`..` components, no
/// trailing slash. The remote path is never shell-interpreted, but its
/// spelling is still validated before any provider call.
fn validate_spelling(path: &str) -> Result<(), CacheError> {
    let native = path.starts_with('/')
        && !path.ends_with('/')
        && path
            .split('/')
            .skip(1) // the empty component before the leading slash
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if native {
        Ok(())
    } else {
        Err(CacheError::Spelling(path.to_string()))
    }
}

/// Step 1 of the install state machine: resolve the authorized native
/// cache root and validate ownership, privacy and the parent/symlink
/// policy. Missing components are created private; pre-existing
/// non-compliant components refuse — deployment never chmods a foreign
/// or loosened entry into compliance.
pub fn resolve(provider: &impl DeployProvider) -> Result<CacheLayout, CacheError> {
    let principal = provider.endpoint().principal.clone();
    let base = provider.cache_base()?;
    validate_spelling(&base)?;
    ensure_dir(
        provider, &base, &principal, /* must_be_private */ false,
    )?;
    let root = format!("{base}/{CACHE_DIR_NAME}");
    ensure_dir(provider, &root, &principal, true)?;
    for sub in [OBJECTS, RECEIPTS, STAGING, LEASES] {
        ensure_dir(provider, &format!("{root}/{sub}"), &principal, true)?;
    }
    Ok(CacheLayout { root })
}

/// One directory component: create private when absent, validate when
/// present. `must_be_private` holds for every component deployment owns;
/// the cache base itself may carry the platform's default mode.
fn ensure_dir(
    provider: &impl DeployProvider,
    path: &str,
    principal: &str,
    must_be_private: bool,
) -> Result<(), CacheError> {
    match provider.lstat(path)? {
        None => Ok(provider.mkdir_private(path)?),
        Some(stat) => validate_dir(path, &stat, principal, must_be_private),
    }
}

fn validate_dir(
    path: &str,
    stat: &RemoteStat,
    principal: &str,
    must_be_private: bool,
) -> Result<(), CacheError> {
    if stat.kind == RemoteKind::Symlink {
        return Err(CacheError::SymlinkComponent(path.to_string()));
    }
    if stat.kind != RemoteKind::Dir {
        return Err(CacheError::NotDirectory(path.to_string()));
    }
    if stat.owner != principal {
        return Err(CacheError::NotOwned {
            path: path.to_string(),
            owner: stat.owner.clone(),
            expected: principal.to_string(),
        });
    }
    if must_be_private && stat.mode & 0o077 != 0 {
        return Err(CacheError::NotPrivate {
            path: path.to_string(),
            mode: stat.mode,
        });
    }
    Ok(())
}

/// Read and parse one receipt; a missing/malformed receipt is an honest
/// non-authorization, never a guessed one (the install-receipt rule).
pub fn read_receipt(
    provider: &impl DeployProvider,
    layout: &CacheLayout,
    sha256: &str,
) -> Result<Option<CacheReceipt>, ProviderError> {
    let path = layout.receipt(sha256);
    if provider.lstat(&path)?.is_none() {
        return Ok(None);
    }
    let bytes = provider.fetch(&path, MAX_RECORD_BYTES)?;
    Ok(serde_json::from_slice(&bytes).ok())
}

/// Whether any receipt authorizes deployment for this endpoint
/// context+principal (previously authorized deployment may quietly reuse
/// and extend the verified cache).
pub fn authorized(
    provider: &impl DeployProvider,
    layout: &CacheLayout,
    context: &str,
    principal: &str,
) -> Result<bool, ProviderError> {
    for name in provider.list(&layout.receipts_dir())? {
        let Some(sha256) = name.strip_suffix(".json") else {
            continue;
        };
        if let Some(receipt) = read_receipt(provider, layout, sha256)? {
            if receipt.context == context && receipt.principal == principal {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_spelling_policy() {
        assert!(validate_spelling("/home/alice/.cache").is_ok());
        for bad in [
            "home/alice",
            "/home//alice",
            "/home/./alice",
            "/home/../alice",
            "/home/alice/",
            "",
        ] {
            assert!(validate_spelling(bad).is_err(), "{bad}");
        }
    }
}
