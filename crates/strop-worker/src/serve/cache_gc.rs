//! Native cache maintenance is executed by the admitted worker, never by
//! an SFTP/container listing followed by an unlocked client-side unlink.
//! The persistent OS lock excludes new worker leases from the complete
//! lease snapshot through retirement; a crash releases the lock without
//! making an old lease record disappear or guessing its liveness.

use std::collections::HashSet;
use std::fs::{self, DirEntry};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

use strop_core::worker::cache_record::{
    CacheGcReport, CacheReceipt, LeaseRecord, LEASES_DIR, OBJECTS_DIR, RECEIPTS_DIR, RECEIPT_SCHEMA,
};
use strop_core::worker::deploy_policy::{is_content_address, keep_object};
use strop_core::worker::CancelToken;
use strop_worker_protocol::ResultOutcome;
use strop_workspace::operation::{FsFailure, FsFailureKind};

use super::cache_lease::{record_bytes as read_private_record, CacheLeaseGuard};
use super::{failure, SessionState};

const MAX_LISTING: usize = 4096;

fn io_failure(stage: &'static str, error: io::Error) -> FsFailure {
    failure(FsFailureKind::Io, format!("cache {stage}: {error}"))
}

fn private_directory(path: &Path, uid: u32) -> Result<(), FsFailure> {
    let stat = fs::symlink_metadata(path).map_err(|error| io_failure("directory", error))?;
    if !stat.file_type().is_dir() || stat.uid() != uid || stat.mode() & 0o077 != 0 {
        return Err(failure(
            FsFailureKind::Permission,
            "cache directory is not private and owned by this worker",
        ));
    }
    Ok(())
}

fn entries(path: &Path) -> Result<Vec<DirEntry>, FsFailure> {
    let mut found = Vec::new();
    for entry in fs::read_dir(path).map_err(|error| io_failure("listing", error))? {
        if found.len() == MAX_LISTING {
            return Err(failure(
                FsFailureKind::Incomplete,
                "cache listing exceeds its bounded pass",
            ));
        }
        found.push(entry.map_err(|error| io_failure("listing entry", error))?);
    }
    Ok(found)
}

fn record_bytes(path: &Path, uid: u32) -> Result<Option<Vec<u8>>, FsFailure> {
    read_private_record(path, uid).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidData {
            failure(FsFailureKind::Protocol, error.to_string())
        } else {
            io_failure("record", error)
        }
    })
}

fn leases(path: &Path, uid: u32) -> Result<HashSet<String>, FsFailure> {
    let mut pinned = HashSet::new();
    for entry in entries(path)? {
        let Ok(name) = entry.file_name().into_string() else {
            continue; // not our numeric lease record
        };
        let Some(id) = name
            .strip_suffix(".json")
            .and_then(|number| number.parse::<u64>().ok())
        else {
            continue;
        };
        let Some(bytes) = record_bytes(&entry.path(), uid)? else {
            continue; // a worker finished and removed its own record
        };
        let record: LeaseRecord = serde_json::from_slice(&bytes)
            .map_err(|error| failure(FsFailureKind::Protocol, format!("cache lease: {error}")))?;
        if record.lease != id || !is_content_address(record.object_sha256.as_bytes()) {
            return Err(failure(
                FsFailureKind::Protocol,
                "cache lease id or object address is invalid",
            ));
        }
        pinned.insert(record.object_sha256);
    }
    Ok(pinned)
}

fn sweep(
    guard: &CacheLeaseGuard,
    token: &CancelToken,
    context: &str,
    report: &mut CacheGcReport,
) -> Result<(), FsFailure> {
    if context.is_empty() {
        return Err(failure(
            FsFailureKind::InvalidPath,
            "cache context is empty",
        ));
    }
    if context != guard.context() {
        return Err(failure(
            FsFailureKind::Permission,
            "cache collection context does not match this worker's admitted endpoint",
        ));
    }
    #[cfg(not(strop_mutant))]
    let _lock = guard.lock().map_err(|error| io_failure("lock", error))?;
    #[cfg(strop_mutant)]
    let _lock = if std::env::var("STROP_MUTANT").as_deref() == Ok("cache-gc-unlocked-retirement") {
        None
    } else {
        Some(guard.lock().map_err(|error| io_failure("lock", error))?)
    };
    let root = guard.root().map_err(|error| io_failure("root", error))?;
    let leases_dir = root.join(LEASES_DIR);
    let objects_dir = root.join(OBJECTS_DIR);
    let receipts_dir = root.join(RECEIPTS_DIR);
    for dir in [&leases_dir, &objects_dir, &receipts_dir] {
        private_directory(dir, guard.uid())?;
    }
    if token.is_cancelled() {
        return Err(failure(
            FsFailureKind::Cancelled,
            "cache collection cancelled",
        ));
    }
    // The same lock is acquired before a cached worker can write its
    // session record or send Welcome. This snapshot is authoritative
    // until the final unlink below; a departing worker may remove its
    // own record, which only makes this keep set conservative.
    let pinned = leases(&leases_dir, guard.uid())?;
    let objects = entries(&objects_dir)?;
    let principal = guard.principal();
    for entry in objects {
        if token.is_cancelled() {
            return Err(failure(
                FsFailureKind::Cancelled,
                "cache collection cancelled",
            ));
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_content_address(name.as_bytes()) {
            continue; // foreign entry, never touched
        }
        let live_leases = u64::from(pinned.contains(&name));
        #[cfg(strop_mutant)]
        let live_leases =
            if std::env::var("STROP_MUTANT").as_deref() == Ok("cache-gc-ignore-foreign-lease") {
                0
            } else {
                live_leases
            };
        if keep_object(name == guard.object_sha256(), live_leases) {
            report.kept_objects += 1;
            continue;
        }
        let receipt_path = receipts_dir.join(format!("{name}.json"));
        let Some(bytes) = record_bytes(&receipt_path, guard.uid())? else {
            continue; // no positive receipt: not ours to retire
        };
        let receipt: CacheReceipt = serde_json::from_slice(&bytes)
            .map_err(|error| failure(FsFailureKind::Protocol, format!("cache receipt: {error}")))?;
        if receipt.schema != RECEIPT_SCHEMA
            || receipt.context != context
            || receipt.principal != principal
            || receipt.target != strop_worker_protocol::TARGET_TRIPLE
            || receipt.object_sha256 != name
        {
            continue;
        }
        let object = entry.path();
        let stat = match fs::symlink_metadata(&object) {
            Ok(stat) => stat,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_failure("object stat", error)),
        };
        if !stat.file_type().is_file()
            || stat.uid() != guard.uid()
            || stat.nlink() != 1
            || stat.mode() & 0o100 == 0
            || stat.mode() & 0o022 != 0
            || stat.len() != receipt.object_bytes
        {
            continue; // not the positively owned executable in the receipt
        }
        fs::remove_file(&object).map_err(|error| io_failure("object remove", error))?;
        report.removed_objects.push(name);
        fs::remove_file(&receipt_path).map_err(|error| io_failure("receipt remove", error))?;
        report.removed_receipts += 1;
    }
    Ok(())
}

pub(super) fn collect(
    shared: &Arc<SessionState>,
    token: &CancelToken,
    context: &str,
) -> ResultOutcome {
    let mut report = CacheGcReport::default();
    let failed = match shared.cache_lease.as_ref() {
        Some(guard) => sweep(guard, token, context, &mut report).err(),
        None => Some(failure(
            FsFailureKind::Unsupported,
            "this worker does not own a managed cache object",
        )),
    };
    let failure = failed.map(|error| {
        if report.removed_objects.is_empty() {
            error
        } else {
            failure(
                FsFailureKind::Incomplete,
                format!(
                    "cache pass stopped after {} object retirements: {error}",
                    report.removed_objects.len()
                ),
            )
        }
    });
    ResultOutcome::CacheCollected { report, failure }
}
