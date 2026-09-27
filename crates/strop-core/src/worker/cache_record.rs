//! The on-disk cache-lease vocabulary shared by the deploying client and
//! the actual worker process. A deployment probe has its own short-lived
//! session; only the worker can record the lease it really serves.

use serde::{Deserialize, Serialize};

pub const CACHE_DIR_NAME: &str = "strop-worker";
pub const OBJECTS_DIR: &str = "objects";
pub const LEASES_DIR: &str = "leases";
pub const RECEIPTS_DIR: &str = "receipts";

/// Persistent inode shared by native lease admission and cache GC.
/// Never unlink it: file locks are scoped to the inode, not the name.
pub const CACHE_LOCK_FILE: &str = "cache.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub lease: u64,
    pub object_sha256: String,
}

/// Receipt schema version.
pub const RECEIPT_SCHEMA: u32 = 1;

/// Bound on one provenance or lease document, before deserialization.
pub const MAX_RECORD_BYTES: u64 = 16 * 1024;

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

/// One bounded native cache-retirement pass. Digest names only; no
/// object bytes, paths, source contents or credentials cross the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheGcReport {
    pub removed_objects: Vec<String>,
    pub removed_receipts: usize,
    pub kept_objects: usize,
}
