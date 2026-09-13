//! Retire captured quiescent bookkeeping, never arrivals or ordinary directory data.
use super::{lock_identity, open_lock, Parent};
use crate::{failure, io_failure, observation};
use rustix::fs::{AtFlags, Mode, OFlags};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    os::unix::ffi::OsStrExt,
};
use strop_core::worker::CancelToken;
use strop_workspace::{
    operation::{FsFailure, FsFailureKind},
    EntryKind, ObjectId, Observation,
};

const LOCK_LIMIT: usize = 100_000;
const NAME_BYTES: usize = 16 * 1024 * 1024;
struct CapturedLock {
    name: OsString,
    identity: ObjectId,
}
struct DirectoryLocks<'a> {
    directory: Parent,
    entries: Vec<CapturedLock>,
    approved: &'a Observation,
}

fn protocol_name(name: &[u8]) -> bool {
    name.strip_prefix(b".strop-lock-").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    })
}
fn identity(value: &rustix::fs::Stat) -> ObjectId {
    ObjectId {
        device: value.st_dev as _,
        inode: value.st_ino as _,
    }
}

impl<'a> DirectoryLocks<'a> {
    fn capture(
        parent: &Parent,
        name: &OsStr,
        approved: &'a Observation,
        token: &CancelToken,
    ) -> Result<Self, FsFailure> {
        parent.revalidate()?;
        let file = rustix::fs::openat(
            &parent.file,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|error| io_failure(error.into()))?;
        let observed = observation::metadata(&file.metadata().map_err(io_failure)?);
        let expected = approved
            .identity
            .ok_or_else(|| failure(FsFailureKind::Unsupported, "directory identity unavailable"))?;
        if observed.kind != EntryKind::Directory || observed.identity != Some(expected) {
            return Err(failure(
                FsFailureKind::Conflict,
                "removal directory changed identity",
            ));
        }
        let directory = Parent {
            path: parent.path.join(name),
            file,
            identity: expected,
        };
        let mut entries = Vec::new();
        let mut bytes = 0usize;
        let iterator = rustix::fs::Dir::read_from(&directory.file)
            .map_err(|error| io_failure(error.into()))?;
        for entry in iterator {
            if token.is_cancelled() {
                return Err(failure(
                    FsFailureKind::Cancelled,
                    "directory cleanup cancelled before unlink",
                ));
            }
            let entry = entry.map_err(|error| io_failure(error.into()))?;
            let name = entry.file_name();
            let raw = name.to_bytes();
            if raw == b"." || raw == b".." {
                continue;
            }
            if !protocol_name(raw) {
                return Err(failure(
                    FsFailureKind::Unsupported,
                    "non-empty directory removal is not authorized",
                ));
            }
            bytes = bytes.saturating_add(raw.len());
            if entries.len() >= LOCK_LIMIT || bytes > NAME_BYTES {
                return Err(failure(
                    FsFailureKind::Unsupported,
                    "directory lock cleanup exceeds 100000 names or 16 MiB",
                ));
            }
            let observed = rustix::fs::statat(&directory.file, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| io_failure(error.into()))?;
            entries.push(CapturedLock {
                name: OsStr::from_bytes(raw).to_owned(),
                identity: lock_identity(&observed, true)?,
            });
        }
        let captured = Self {
            directory,
            entries,
            approved,
        };
        captured.revalidate(parent, name)?;
        Ok(captured)
    }

    fn revalidate(&self, parent: &Parent, name: &OsStr) -> Result<(), FsFailure> {
        parent.revalidate()?;
        self.directory.revalidate()?;
        let named = rustix::fs::statat(&parent.file, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| io_failure(error.into()))?;
        let held = observation::metadata(&self.directory.file.metadata().map_err(io_failure)?);
        // Only directory size/timestamps may reflect our own lock-entry removals.
        if identity(&named) != self.directory.identity
            || !self.approved.same_object(&held)
            || self.approved.uid != held.uid
            || self.approved.gid != held.gid
            || self.approved.permissions != held.permissions
            || self.approved.links != held.links
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "removal directory binding or permissions changed",
            ));
        }
        Ok(())
    }

    fn prune(
        self,
        parent: &Parent,
        name: &OsStr,
        token: &CancelToken,
    ) -> Result<(File, usize), FsFailure> {
        let mut confirmed = 0;
        let result = (|| {
            self.revalidate(parent, name)?;
            for entry in &self.entries {
                if token.is_cancelled() {
                    return Err(failure(
                        FsFailureKind::Cancelled,
                        "directory cleanup cancelled",
                    ));
                }
                let (file, opened) = open_lock(&self.directory, &entry.name, false)?;
                let current = rustix::fs::statat(
                    &self.directory.file,
                    &entry.name,
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .map_err(|error| io_failure(error.into()))?;
                let held = rustix::fs::fstat(&file).map_err(|error| io_failure(error.into()))?;
                if opened != entry.identity
                    || lock_identity(&current, true)? != entry.identity
                    || lock_identity(&held, true)? != entry.identity
                {
                    return Err(failure(
                        FsFailureKind::Conflict,
                        "captured operation lock changed identity",
                    ));
                }
                self.revalidate(parent, name)?;
                rustix::fs::unlinkat(&self.directory.file, &entry.name, AtFlags::empty())
                    .map_err(|error| io_failure(error.into()))?;
                confirmed += 1;
                // Every participant checks the pathname after flock. A waiter on
                // this unlinked inode therefore refuses. Close before rmdir so
                // our own descriptor cannot retain an NFS silly-rename entry.
                drop(file);
            }
            self.revalidate(parent, name)
        })();
        if let Err(mut error) = result {
            error.detail = format!(
                "directory removal not attempted; {confirmed} protocol lock removals confirmed; {}",
                error.detail
            );
            return Err(error);
        }
        Ok((self.directory.file, confirmed))
    }
}

pub(crate) fn prune_directory_locks(
    parent: &Parent,
    name: &OsStr,
    approved: &Observation,
    token: &CancelToken,
) -> Result<(File, usize), FsFailure> {
    DirectoryLocks::capture(parent, name, approved, token)?.prune(parent, name, token)
}

#[cfg(test)]
mod tests;
