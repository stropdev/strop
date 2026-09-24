//! Lease-aware cache garbage collection (0058 §5, WK06): bounded,
//! receipt-scoped cleanup that never retires a live lease's object and
//! never glob-deletes — only positively identified content-addressed
//! entries are touched.
//!
//! Keep-set, in order:
//! 1. the object this client currently runs (`current_object`);
//! 2. every object referenced by a live lease (the caller probes
//!    liveness; lease files whose ids are not live are stale records and
//!    are removed as positively identified `<id>.json` files).
//!
//! Everything else in `objects/` whose name is a 64-hex content address
//! is retired with its receipt. Concurrent client versions hold their
//! own current objects through their own leases, so one editor's GC
//! cannot retire another live editor's worker.

use crate::cache::{CacheLayout, LeaseRecord};
use crate::provider::{DeployProvider, ProviderError};
use crate::MAX_RECORD_BYTES;

/// Bounded cleanup: a cache with more entries than this in one directory
/// is pathological; GC refuses rather than running unbounded.
const MAX_LISTING: usize = 4096;

/// What GC may and must keep.
pub struct GcPolicy<'a> {
    /// The content address of the object this client currently uses.
    pub current_object: Option<String>,
    /// Lease ids probed live *right now* by the caller (a live worker
    /// holds its lease for its whole lifetime).
    pub live_leases: &'a [u64],
}

/// What one GC pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Content addresses of retired objects.
    pub removed_objects: Vec<String>,
    /// Receipts retired alongside their objects.
    pub removed_receipts: usize,
    /// Stale lease records removed.
    pub removed_stale_leases: usize,
    /// Objects kept (current or live-lease-referenced).
    pub kept_objects: usize,
}

fn is_content_address(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Run one bounded GC pass over the cache.
pub fn collect(
    provider: &impl DeployProvider,
    layout: &CacheLayout,
    policy: &GcPolicy<'_>,
) -> Result<GcReport, ProviderError> {
    let mut report = GcReport::default();

    // Live leases pin their objects; stale lease records are removed.
    let mut pinned: Vec<String> = Vec::new();
    let leases = provider.list(&layout.leases_dir())?;
    if leases.len() > MAX_LISTING {
        return Err(ProviderError::Transport(format!(
            "lease directory holds {} entries, over the {MAX_LISTING} bound",
            leases.len()
        )));
    }
    for name in leases {
        let Some(id) = name
            .strip_suffix(".json")
            .and_then(|id| id.parse::<u64>().ok())
        else {
            continue; // not a lease record; never touched
        };
        if !policy.live_leases.contains(&id) {
            provider.remove(&layout.lease(id))?;
            report.removed_stale_leases += 1;
            continue;
        }
        let bytes = provider.fetch(&layout.lease(id), MAX_RECORD_BYTES)?;
        if let Ok(record) = serde_json::from_slice::<LeaseRecord>(&bytes) {
            pinned.push(record.object_sha256);
        }
    }

    // Retire unreferenced content-addressed objects with their receipts.
    let objects = provider.list(&layout.objects_dir())?;
    if objects.len() > MAX_LISTING {
        return Err(ProviderError::Transport(format!(
            "object directory holds {} entries, over the {MAX_LISTING} bound",
            objects.len()
        )));
    }
    for name in objects {
        if !is_content_address(&name) {
            continue; // foreign entry: positively not ours, never touched
        }
        let keep = policy.current_object.as_deref() == Some(name.as_str())
            || pinned.iter().any(|sha| sha == &name);
        if keep {
            report.kept_objects += 1;
            continue;
        }
        provider.remove(&layout.object(&name))?;
        if provider.lstat(&layout.receipt(&name))?.is_some() {
            provider.remove(&layout.receipt(&name))?;
            report.removed_receipts += 1;
        }
        report.removed_objects.push(name);
    }
    Ok(report)
}
