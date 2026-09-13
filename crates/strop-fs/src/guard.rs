//! Descriptor-pinned parents and deterministic cooperative name locks.
//! Final checks do not promise CAS against nonparticipating namespace writers.
use crate::{failure, io_failure, observation};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use strop_workspace::operation::{FsFailure, FsFailureKind};
use strop_workspace::{EntryKind, EntryName, ObjectId, ResourceLocation};
use unicode_normalization::UnicodeNormalization;

mod cleanup;
pub(crate) use cleanup::prune_directory_locks;

fn lock_identity(value: &rustix::fs::Stat, empty: bool) -> Result<ObjectId, FsFailure> {
    if rustix::fs::FileType::from_raw_mode(value.st_mode) != rustix::fs::FileType::RegularFile
        || value.st_nlink != 1
        || value.st_uid != rustix::process::geteuid().as_raw()
        || value.st_mode & 0o7777 != 0o600
        || (empty && value.st_size != 0)
    {
        return Err(failure(
            FsFailureKind::Permission,
            "operation lock is not a private owned single-link file with the required contents",
        ));
    }
    Ok(ObjectId {
        device: value.st_dev as _,
        inode: value.st_ino as _,
    })
}

fn open_lock(
    parent: &Parent,
    name: &std::ffi::OsStr,
    create: bool,
) -> Result<(File, ObjectId), FsFailure> {
    use rustix::fs::{FlockOperation, Mode, OFlags};
    let mut flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    if create {
        flags |= OFlags::CREATE;
    }
    let file = rustix::fs::openat(&parent.file, name, flags, Mode::from_raw_mode(0o600))
        .map(File::from)
        .map_err(|error| io_failure(error.into()))?;
    let observed = rustix::fs::fstat(&file).map_err(|error| io_failure(error.into()))?;
    let identity = lock_identity(&observed, !create)?;
    rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
        if error == rustix::io::Errno::WOULDBLOCK {
            failure(
                FsFailureKind::Busy,
                "another cooperating operation owns this name",
            )
        } else {
            io_failure(error.into())
        }
    })?;
    Ok((file, identity))
}

pub(crate) fn reserved(path: &Path) -> bool {
    path.components().any(|part| {
        let Component::Normal(name) = part else {
            return false;
        };
        let normalized: String = name
            .to_string_lossy()
            .nfkc()
            .flat_map(char::to_lowercase)
            .collect();
        normalized.starts_with(".strop-lock-")
            || normalized.starts_with(".strop-save-")
            || normalized.starts_with(".strop-fs-")
    })
}

/// Resolve existing ancestors but never follow the final entry. A missing suffix
/// is retained for a reviewed creation chain, not created by this function.
pub(crate) fn resolve(path: &Path) -> Result<PathBuf, FsFailure> {
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0) || reserved(path) {
        return Err(failure(
            FsFailureKind::InvalidPath,
            "an absolute non-control resource path is required",
        ));
    }
    let name = path.file_name().ok_or_else(|| {
        failure(
            FsFailureKind::InvalidPath,
            "filesystem roots cannot be operation targets",
        )
    })?;
    EntryName::new(name.into())
        .map_err(|error| failure(FsFailureKind::InvalidPath, error.to_string()))?;
    let mut ancestor = path
        .parent()
        .ok_or_else(|| failure(FsFailureKind::InvalidPath, "target has no parent"))?
        .to_path_buf();
    let mut missing = vec![name.to_owned()];
    let base = loop {
        match std::fs::canonicalize(&ancestor) {
            Ok(base) => break base,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let leaf = ancestor.file_name().ok_or_else(|| {
                    failure(
                        FsFailureKind::InvalidPath,
                        "missing parent chain is not canonical",
                    )
                })?;
                if ancestor
                    .components()
                    .any(|part| matches!(part, Component::ParentDir))
                {
                    return Err(failure(
                        FsFailureKind::InvalidPath,
                        "resolve .. before requesting missing-parent creation",
                    ));
                }
                missing.push(leaf.to_owned());
                if !ancestor.pop() {
                    return Err(io_failure(error));
                }
            }
            Err(error) => return Err(io_failure(error)),
        }
    };
    if reserved(&base) {
        return Err(failure(
            FsFailureKind::InvalidPath,
            "operation targets protected filesystem state",
        ));
    }
    let mut resolved = base;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

pub(crate) struct Parent {
    pub path: PathBuf,
    pub file: File,
    pub identity: ObjectId,
}
impl Parent {
    pub fn open(path: &Path) -> Result<Self, FsFailure> {
        use rustix::fs::{Mode, OFlags};
        let file = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|error| io_failure(error.into()))?;
        let metadata = observation::metadata(&file.metadata().map_err(io_failure)?);
        let identity = metadata.identity.ok_or_else(|| {
            failure(
                FsFailureKind::Unsupported,
                "parent object identity is unavailable",
            )
        })?;
        if metadata.kind != EntryKind::Directory {
            return Err(failure(
                FsFailureKind::InvalidPath,
                "parent is not a directory",
            ));
        }
        let parent = Self {
            path: path.to_path_buf(),
            file,
            identity,
        };
        parent.revalidate()?;
        Ok(parent)
    }
    pub fn revalidate(&self) -> Result<(), FsFailure> {
        let current =
            observation::metadata(&std::fs::symlink_metadata(&self.path).map_err(io_failure)?);
        if current.kind != EntryKind::Directory || current.identity != Some(self.identity) {
            return Err(failure(
                FsFailureKind::Conflict,
                "parent directory changed identity",
            ));
        }
        Ok(())
    }
    pub fn exact_existing_name(&self, name: &std::ffi::OsStr) -> Result<(), FsFailure> {
        for entry in std::fs::read_dir(&self.path).map_err(io_failure)? {
            if entry.map_err(io_failure)?.file_name() == name {
                return Ok(());
            }
        }
        Err(failure(
            FsFailureKind::Conflict,
            "source spelling is absent or aliases another native name",
        ))
    }
}

pub(crate) struct NameLock {
    file: File,
    parent: PathBuf,
    name: OsString,
    identity: ObjectId,
}
impl NameLock {
    pub fn acquire(parent: &Parent, name: &std::ffi::OsStr) -> Result<Self, FsFailure> {
        let digest = Sha256::digest(name.as_encoded_bytes());
        let mut lock_name = String::from(".strop-lock-");
        use std::fmt::Write as _;
        for byte in digest {
            let _ = write!(lock_name, "{byte:02x}");
        }
        let (file, identity) = open_lock(parent, std::ffi::OsStr::new(&lock_name), true)?;
        let lock = Self {
            file,
            parent: parent.path.clone(),
            name: lock_name.into(),
            identity,
        };
        if let Err(mut error) = lock.revalidate() {
            if let Err(release) = lock.release() {
                error.detail.push_str(&format!("; lock release: {release}"));
            }
            return Err(error);
        }
        Ok(lock)
    }
    /// Closing alone is insufficient: a concurrent fork may hold the same
    /// open-file description until exec. End our ownership explicitly.
    pub fn release(self) -> Result<(), FsFailure> {
        rustix::fs::flock(&self.file, rustix::fs::FlockOperation::Unlock)
            .map_err(|error| io_failure(error.into()))
    }
    pub fn revalidate(&self) -> Result<(), FsFailure> {
        let current = observation::metadata(
            &std::fs::symlink_metadata(self.parent.join(&self.name)).map_err(io_failure)?,
        );
        let held = observation::metadata(&self.file.metadata().map_err(io_failure)?);
        if current.identity != Some(self.identity)
            || held.identity != Some(self.identity)
            || current.links != Some(1)
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "operation lock changed identity",
            ));
        }
        Ok(())
    }
}

pub(crate) fn local_path(location: &ResourceLocation) -> Result<&Path, FsFailure> {
    location.local_path().ok_or_else(|| {
        failure(
            FsFailureKind::Unsupported,
            "native operation cannot cross into another filesystem namespace",
        )
    })
}

/// Pin the approved object without adding read permission to rename authority.
pub(crate) fn pin_identity(
    parent: &Parent,
    name: &std::ffi::OsStr,
    approved: &strop_workspace::Observation,
) -> Result<File, FsFailure> {
    use rustix::fs::{Mode, OFlags};
    #[cfg(target_os = "linux")]
    let access = OFlags::PATH;
    #[cfg(target_os = "macos")]
    let access = OFlags::from_bits_retain(libc::O_EVTONLY as _);
    let file = rustix::fs::openat(
        &parent.file,
        name,
        access | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| io_failure(error.into()))?;
    let observed = observation::metadata(&file.metadata().map_err(io_failure)?);
    if approved.identity.is_none() || !approved.same_metadata(&observed) {
        return Err(failure(
            FsFailureKind::Conflict,
            "source changed before its publication identity was pinned",
        ));
    }
    Ok(file)
}
