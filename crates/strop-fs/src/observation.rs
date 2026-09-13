//! Stable native observations and bounded streaming digests, on workers only.
use crate::{failure, io_failure};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use strop_core::worker::CancelToken;
use strop_workspace::operation::{FsFailure, FsFailureKind};
use strop_workspace::{FileTime, Observation};

pub(crate) fn metadata(value: &std::fs::Metadata) -> Observation {
    let mut observation = Observation::unknown(crate::listing::file_kind(value.file_type()));
    observation.size = Some(value.len());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        observation.identity = Some(strop_workspace::ObjectId {
            device: value.dev(),
            inode: value.ino(),
        });
        observation.modified = time(value.mtime(), value.mtime_nsec());
        observation.changed = time(value.ctime(), value.ctime_nsec());
        observation.permissions = Some(strop_workspace::Permissions::from_mode(value.mode()));
        observation.uid = Some(value.uid());
        observation.gid = Some(value.gid());
        observation.links = Some(value.nlink());
    }
    #[cfg(not(unix))]
    {
        observation.modified = value.modified().ok().and_then(|stamp| {
            let duration = stamp.duration_since(std::time::UNIX_EPOCH).ok()?;
            Some(FileTime {
                seconds: i64::try_from(duration.as_secs()).ok()?,
                nanos: duration.subsec_nanos(),
            })
        });
    }
    observation
}

#[cfg(unix)]
fn time(seconds: i64, nanos: i64) -> Option<FileTime> {
    u32::try_from(nanos)
        .ok()
        .filter(|nanos| *nanos < 1_000_000_000)
        .map(|nanos| FileTime { seconds, nanos })
}

/// Post-publication stat must run even if cancellation arrived after the syscall.
pub(crate) fn stat(path: &Path) -> Result<Option<Observation>, FsFailure> {
    match std::fs::symlink_metadata(path) {
        Ok(value) => Ok(Some(metadata(&value))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_failure(error)),
    }
}

pub fn observe(
    path: &Path,
    digest: bool,
    token: &CancelToken,
) -> Result<Option<Observation>, FsFailure> {
    if token.is_cancelled() {
        return Err(failure(FsFailureKind::Cancelled, "inspection cancelled"));
    }
    let before = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_failure(error)),
    };
    let mut result = metadata(&before);
    if digest && before.is_file() {
        let mut file = open_read(path)?;
        let descriptor_before = metadata(&file.metadata().map_err(io_failure)?);
        if descriptor_before != result {
            return Err(failure(
                FsFailureKind::Conflict,
                "source changed before inspection",
            ));
        }
        let mut hash = Sha256::new();
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            if token.is_cancelled() {
                return Err(failure(FsFailureKind::Cancelled, "inspection cancelled"));
            }
            let count = file.read(&mut chunk).map_err(io_failure)?;
            if count == 0 {
                break;
            }
            hash.update(&chunk[..count]);
        }
        if metadata(&file.metadata().map_err(io_failure)?) != descriptor_before {
            return Err(failure(
                FsFailureKind::Conflict,
                "source changed during inspection",
            ));
        }
        result.digest = Some(hash.finalize().into());
    }
    Ok(Some(result))
}

pub(crate) fn open_read(path: &Path) -> Result<std::fs::File, FsFailure> {
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags};
        rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map(std::fs::File::from)
        .map_err(|error| io_failure(error.into()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(failure(
            FsFailureKind::Unsupported,
            "safe no-follow file inspection is not implemented on this platform",
        ))
    }
}
