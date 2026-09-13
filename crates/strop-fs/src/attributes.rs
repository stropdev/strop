//! Copy metadata is explicit and bounded. Security metadata is never silently lost.
use crate::{failure, io_failure};
use std::ffi::OsString;
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
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
