//! Copy metadata is explicit and bounded. Security metadata is never silently lost.
use crate::{failure, io_failure};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use strop_workspace::operation::{FsFailure, FsFailureKind};

const ATTRIBUTE_LIMIT: usize = 64 * 1024;
const TOTAL_LIMIT: usize = 1024 * 1024;

pub(crate) fn read(file: &File) -> Result<Vec<(OsString, Vec<u8>)>, FsFailure> {
    let mut names = vec![0_u8; ATTRIBUTE_LIMIT];
    let count = rustix::fs::flistxattr(file, names.as_mut_slice()).map_err(|error| {
        failure(
            FsFailureKind::Unsupported,
            format!("extended-attribute inspection failed: {error}"),
        )
    })?;
    names.truncate(count);
    let mut result = Vec::new();
    let mut value = vec![0_u8; ATTRIBUTE_LIMIT];
    let mut total = 0;
    for name in names
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let name = std::ffi::OsStr::from_bytes(name);
        let count = rustix::fs::fgetxattr(file, name, value.as_mut_slice()).map_err(|error| {
            failure(
                FsFailureKind::Unsupported,
                format!("attribute cannot be preserved within its bound: {error}"),
            )
        })?;
        total += name.len() + count;
        if total > TOTAL_LIMIT {
            return Err(failure(
                FsFailureKind::Unsupported,
                "extended attributes exceed the 1 MiB preservation bound",
            ));
        }
        result.push((name.to_owned(), value[..count].to_vec()));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}
/// Length-delimited, sorted xattr inventory; the empty set has an
/// explicit digest. Digesting only names or concatenated values could
/// alias distinct security metadata.
pub(crate) fn fingerprint(attributes: &[(OsString, Vec<u8>)]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for (name, value) in attributes {
        let name = name.as_os_str().as_bytes();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value);
    }
    hash.finalize().into()
}

pub(crate) fn digest(file: &File) -> Result<[u8; 32], FsFailure> {
    read(file).map(|attributes| fingerprint(&attributes))
}

pub(crate) fn apply(
    file: &File,
    source: &std::fs::Metadata,
    attributes: &[(OsString, Vec<u8>)],
    preserve_mtime: bool,
) -> Result<(), FsFailure> {
    let mode = source.permissions().mode();
    if mode & 0o6000 != 0 {
        return Err(failure(
            FsFailureKind::Unsupported,
            "copy refuses set-id source permissions",
        ));
    }
    file.set_permissions(std::fs::Permissions::from_mode(mode & 0o777))
        .map_err(io_failure)?;
    // Remove inherited attributes not present on the source before applying the
    // exact source set, so an ACL/quarantine flag is not guessed from the target.
    for (name, _) in read(file)? {
        if !attributes.iter().any(|(expected, _)| expected == &name) {
            rustix::fs::fremovexattr(file, &name).map_err(|error| {
                failure(
                    FsFailureKind::Unsupported,
                    format!("inherited metadata could not be reconciled: {error}"),
                )
            })?;
        }
    }
    for (name, value) in attributes {
        rustix::fs::fsetxattr(file, name, value, rustix::fs::XattrFlags::empty()).map_err(
            |error| {
                failure(
                    FsFailureKind::Unsupported,
                    format!("source metadata could not be preserved: {error}"),
                )
            },
        )?;
    }
    if read(file)? != attributes
        || file.metadata().map_err(io_failure)?.permissions().mode() & 0o777 != mode & 0o777
    {
        return Err(failure(
            FsFailureKind::Unsupported,
            "copy metadata verification failed",
        ));
    }
    if preserve_mtime {
        file.set_times(
            std::fs::FileTimes::new().set_modified(source.modified().map_err(io_failure)?),
        )
        .map_err(io_failure)?;
    }
    Ok(())
}

/// Restore mode, mtime and bounded attributes of a replaced file
/// (0058 WK09 store). Unlike [`apply`], special bits are preserved
/// exactly. Mtime is the original source's timestamp, not the stage's
/// creation time; changed content is distinguished by the digest.
pub(crate) fn restore(
    file: &File,
    source: &std::fs::Metadata,
    attributes: &[(OsString, Vec<u8>)],
) -> Result<(), FsFailure> {
    let stage = file.metadata().map_err(io_failure)?;
    if stage.uid() != source.uid() || stage.gid() != source.gid() {
        rustix::fs::fchown(
            file,
            Some(rustix::process::Uid::from_raw(source.uid())),
            Some(rustix::process::Gid::from_raw(source.gid())),
        )
        .map_err(|error| {
            failure(
                FsFailureKind::Unsupported,
                format!("store owner/group cannot be preserved: {error}"),
            )
        })?;
    }
    let mode = source.permissions().mode() & 0o7777;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
        .map_err(io_failure)?;
    for (name, _) in read(file)? {
        if !attributes.iter().any(|(expected, _)| expected == &name) {
            rustix::fs::fremovexattr(file, &name).map_err(|error| {
                failure(
                    FsFailureKind::Unsupported,
                    format!("inherited metadata could not be reconciled: {error}"),
                )
            })?;
        }
    }
    for (name, value) in attributes {
        rustix::fs::fsetxattr(file, name, value, rustix::fs::XattrFlags::empty()).map_err(
            |error| {
                failure(
                    FsFailureKind::Unsupported,
                    format!("destination metadata could not be preserved: {error}"),
                )
            },
        )?;
    }
    let modified = source.modified().map_err(io_failure)?;
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .map_err(io_failure)?;
    let actual = file.metadata().map_err(io_failure)?;
    if read(file)? != attributes
        || actual.permissions().mode() & 0o7777 != mode
        || actual.modified().map_err(io_failure)? != modified
        || actual.uid() != source.uid()
        || actual.gid() != source.gid()
    {
        return Err(failure(
            FsFailureKind::Unsupported,
            "store metadata verification failed",
        ));
    }
    Ok(())
}
