//! Private stages never expose half-written final files. Cleanup uses the pinned
//! parent and validates each owned name before removal; descriptors close first.
use crate::guard::Parent;
use crate::{failure, io_failure, observation};
use std::fs::File;
use strop_workspace::operation::{FsFailure, FsFailureKind};
use strop_workspace::ObjectId;

pub(crate) fn nonce() -> Result<String, FsFailure> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| failure(FsFailureKind::Io, error.to_string()))?;
    use std::fmt::Write as _;
    let mut text = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}

pub(crate) struct Stage<'a> {
    pub parent: &'a Parent,
    pub directory: File,
    pub file: File,
    pub name: String,
    directory_identity: ObjectId,
    file_identity: ObjectId,
}
impl<'a> Stage<'a> {
    pub fn create(parent: &'a Parent) -> Result<Self, FsFailure> {
        use rustix::fs::{mkdirat, openat, AtFlags, Mode, OFlags};
        parent.revalidate()?;
        let name = format!(".strop-fs-{}", nonce()?);
        mkdirat(&parent.file, name.as_str(), Mode::from_raw_mode(0o700))
            .map_err(|error| io_failure(error.into()))?;
        let directory = match openat(
            &parent.file,
            name.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(directory) => File::from(directory),
            Err(error) => {
                return Err(failure(
                    FsFailureKind::Io,
                    format!(
                        "private stage could not open ({error}); retained {}",
                        parent.path.join(&name).display()
                    ),
                ))
            }
        };
        let directory_info = observation::metadata(&directory.metadata().map_err(io_failure)?);
        if directory_info.uid != Some(rustix::process::geteuid().as_raw())
            || directory_info
                .permissions
                .is_none_or(|permissions| permissions.bits() & 0o777 != 0o700)
        {
            return Err(failure(
                FsFailureKind::Permission,
                "stage is not a private owned directory",
            ));
        }
        let directory_identity = directory_info.identity.ok_or_else(|| {
            failure(
                FsFailureKind::Unsupported,
                "stage directory identity unavailable",
            )
        })?;
        let file = match openat(
            &directory,
            "contents",
            OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        ) {
            Ok(file) => File::from(file),
            Err(error) => {
                drop(directory);
                let cleanup = rustix::fs::unlinkat(&parent.file, name.as_str(), AtFlags::REMOVEDIR);
                return Err(failure(
                    FsFailureKind::Io,
                    format!("stage creation failed: {error}; cleanup: {cleanup:?}"),
                ));
            }
        };
        let file_identity = observation::metadata(&file.metadata().map_err(io_failure)?)
            .identity
            .ok_or_else(|| {
                failure(
                    FsFailureKind::Unsupported,
                    "stage file identity unavailable",
                )
            })?;
        Ok(Self {
            parent,
            directory,
            file,
            name,
            directory_identity,
            file_identity,
        })
    }

    pub fn cleanup(self) -> Result<(), FsFailure> {
        use rustix::fs::AtFlags;
        let Self {
            parent,
            directory,
            file,
            name,
            directory_identity,
            file_identity,
        } = self;
        parent.revalidate()?;
        let stage_path = parent.path.join(&name);
        let current =
            observation::metadata(&std::fs::symlink_metadata(&stage_path).map_err(io_failure)?);
        if current.identity != Some(directory_identity) {
            return Err(failure(
                FsFailureKind::Conflict,
                "private stage changed identity; cleanup refused",
            ));
        }
        // NFS must see the file handle close before unlink/rmdir (0040).
        drop(file);
        match std::fs::symlink_metadata(stage_path.join("contents")) {
            Ok(metadata) if observation::metadata(&metadata).identity == Some(file_identity) => {
                rustix::fs::unlinkat(&directory, "contents", AtFlags::empty())
                    .map_err(|error| io_failure(error.into()))?;
            }
            Ok(_) => {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "private stage contents changed identity; cleanup refused",
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_failure(error)),
        }
        drop(directory);
        for attempt in 0..20 {
            parent.revalidate()?;
            if observation::metadata(&std::fs::symlink_metadata(&stage_path).map_err(io_failure)?)
                .identity
                != Some(directory_identity)
            {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "private stage changed identity during cleanup",
                ));
            }
            match rustix::fs::unlinkat(&parent.file, name.as_str(), AtFlags::REMOVEDIR) {
                Ok(()) => return Ok(()),
                Err(error)
                    if matches!(
                        error,
                        rustix::io::Errno::NOTEMPTY | rustix::io::Errno::EXIST
                    ) && attempt < 19 =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(failure(
                        FsFailureKind::Io,
                        format!("cleanup retained {}: {error}", stage_path.display()),
                    ))
                }
            }
        }
        Err(failure(
            FsFailureKind::Io,
            "private stage cleanup did not finish",
        ))
    }
}
