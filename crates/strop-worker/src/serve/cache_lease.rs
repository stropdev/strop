//! The worker, not the deployment probe, owns its live cache lease.
//! Every fresh process records its own handshake session before Welcome;
//! normal teardown removes only that record. A crash leaves conservative
//! evidence for GC rather than guessing that another client is dead.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use strop_core::worker::cache_record::{
    CacheReceipt, LeaseRecord, CACHE_DIR_NAME, CACHE_LOCK_FILE, LEASES_DIR, MAX_RECORD_BYTES,
    OBJECTS_DIR, RECEIPTS_DIR, RECEIPT_SCHEMA,
};
use strop_core::worker::deploy_policy::is_content_address;
use strop_worker_protocol::Session;

/// All cache workers share this stable inode. Closing the handle also
/// releases the OS lock after a client or worker crash; no stale lock
/// name needs to be guessed away.
fn cache_lock(root: &Path, uid: u32) -> io::Result<File> {
    let path = root.join(CACHE_LOCK_FILE);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)?;
    let stat = fs::symlink_metadata(&path)?;
    let handle = file.metadata()?;
    if !stat.file_type().is_file()
        || stat.uid() != uid
        || stat.mode() & 0o077 != 0
        || stat.nlink() != 1
        || stat.dev() != handle.dev()
        || stat.ino() != handle.ino()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "worker cache lock is not a private owned regular file",
        ));
    }
    file.lock()?;
    Ok(file)
}

/// Exact, bounded facts from a private file; absent records are not
/// launch or retirement authority. Both admission and collection use
/// the same ownership and read bounds.
pub(super) fn record_bytes(path: &Path, uid: u32) -> io::Result<Option<Vec<u8>>> {
    let stat = match fs::symlink_metadata(path) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !stat.file_type().is_file()
        || stat.uid() != uid
        || stat.nlink() != 1
        || stat.mode() & 0o077 != 0
        || stat.len() > MAX_RECORD_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cache record is not a bounded owned private file",
        ));
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let handle = file.metadata()?;
    if stat.dev() != handle.dev() || stat.ino() != handle.ino() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cache record changed between observation and read",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cache record grew past its bound",
        ));
    }
    Ok(Some(bytes))
}

pub(super) struct CacheLeaseGuard {
    path: PathBuf,
    object_sha256: String,
    context: String,
    uid: u32,
}

impl CacheLeaseGuard {
    pub(super) fn register(session: Session) -> io::Result<Option<Self>> {
        let executable = std::env::current_exe()?;
        let Some(name) = executable.file_name().and_then(|name| name.to_str()) else {
            return Ok(None);
        };
        // Linux appends this suffix when the name used to exec the
        // inode was unlinked. It remains executable through /proc, but
        // no longer names a verified cache object and cannot be ready.
        let deleted = name.strip_suffix(" (deleted)");
        let address = deleted.unwrap_or(name);
        let Some(objects) = executable.parent() else {
            return Ok(None);
        };
        let Some(root) = objects.parent() else {
            return Ok(None);
        };
        if objects.file_name().is_none_or(|part| part != OBJECTS_DIR)
            || root.file_name().is_none_or(|part| part != CACHE_DIR_NAME)
            || !is_content_address(address.as_bytes())
        {
            // Installed/local and administrator-provisioned workers are
            // outside the managed object cache; GC never touches them.
            return Ok(None);
        }
        if deleted.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "verified worker object disappeared before Welcome",
            ));
        }
        let leases = root.join(LEASES_DIR);
        let root_stat = fs::symlink_metadata(root)?;
        let leases_stat = fs::symlink_metadata(&leases)?;
        // SAFETY: std has no effective-uid getter; libc::geteuid reads
        // only process credentials and accepts no pointers.
        let uid = unsafe { libc::geteuid() };
        if !root_stat.file_type().is_dir()
            || !leases_stat.file_type().is_dir()
            || root_stat.uid() != uid
            || leases_stat.uid() != uid
            || root_stat.mode() & 0o077 != 0
            || leases_stat.mode() & 0o077 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "worker cache directories are not private and owned",
            ));
        }
        let _lock = cache_lock(root, uid)?;
        let object_stat = fs::symlink_metadata(&executable)?;
        if !object_stat.file_type().is_file()
            || object_stat.uid() != uid
            || object_stat.mode() & 0o100 == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "worker cache object is not owned and executable",
            ));
        }
        #[cfg(target_os = "linux")]
        {
            let running = fs::metadata("/proc/self/exe")?;
            if object_stat.dev() != running.dev() || object_stat.ino() != running.ino() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "verified worker object no longer names the running inode",
                ));
            }
        }
        let receipt_path = root.join(RECEIPTS_DIR).join(format!("{address}.json"));
        let bytes = record_bytes(&receipt_path, uid)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "verified worker receipt is missing",
            )
        })?;
        let receipt: CacheReceipt = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if receipt.schema != RECEIPT_SCHEMA
            || receipt.context.is_empty()
            || receipt.principal != uid.to_string()
            || receipt.version != env!("CARGO_PKG_VERSION")
            || receipt.target != strop_worker_protocol::TARGET_TRIPLE
            || receipt.object_sha256 != address
            || receipt.object_bytes != object_stat.len()
            || receipt.tarball_sha256.is_empty()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "worker cache receipt does not bind this executable",
            ));
        }
        let path = leases.join(format!("{}.json", session.lease.0));
        let record = LeaseRecord {
            lease: session.lease.0,
            object_sha256: address.to_owned(),
        };
        let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        // Sync the record's bytes before granting the live session. A
        // power loss also kills the worker; directory durability is not
        // used as evidence that a dead lease is still live.
        let published = file.write_all(&bytes).and_then(|()| file.sync_all());
        if let Err(error) = published {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(Some(Self {
            path,
            object_sha256: record.object_sha256,
            context: receipt.context,
            uid,
        }))
    }

    pub(super) fn root(&self) -> io::Result<&Path> {
        self.path
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| io::Error::other("worker lease lost its cache root"))
    }

    pub(super) fn object_sha256(&self) -> &str {
        &self.object_sha256
    }
    pub(super) fn context(&self) -> &str {
        &self.context
    }

    pub(super) fn uid(&self) -> u32 {
        self.uid
    }

    pub(super) fn principal(&self) -> String {
        self.uid.to_string()
    }

    pub(super) fn lock(&self) -> io::Result<File> {
        cache_lock(self.root()?, self.uid)
    }
}

impl Drop for CacheLeaseGuard {
    fn drop(&mut self) {
        // A failed cleanup keeps the object pinned rather than risking
        // an unsafe retirement. The collector does not guess stale leases.
        let _ = fs::remove_file(&self.path);
    }
}
