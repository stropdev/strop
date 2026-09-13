//! Native local Trash storage. Linux follows the freedesktop layout; macOS uses
//! .Trash/.Trashes. Receipts retain the real recovery path; no unlink fallback.
mod restore;
#[cfg(all(test, target_os = "linux"))]
mod tests;
use crate::guard::Parent;
use crate::{failure, io_failure, observation, stage, Environment};
pub(crate) use restore::{managed_root, prepare_restore};
use std::fs::File;
#[cfg(target_os = "linux")]
use std::io::Write;
use std::path::Path;
use strop_core::worker::CancelToken;
use strop_workspace::operation::*;
use strop_workspace::{EntryKind, ObjectId, Observation, ResourceLocation};

pub(crate) fn root_for(
    source: &Path,
    environment: &Environment,
) -> Result<ResourceLocation, FsFailure> {
    let home = environment
        .home
        .as_ref()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            failure(
                FsFailureKind::Unsupported,
                "native Trash needs a captured absolute home directory",
            )
        })?;
    #[cfg(target_os = "macos")]
    let home_trash = home.join(".Trash");
    #[cfg(not(target_os = "macos"))]
    let home_trash = environment
        .data_home
        .as_ref()
        .filter(|path| path.is_absolute())
        .cloned()
        .unwrap_or_else(|| home.join(".local/share"))
        .join("Trash");
    let source_device = device(source)?;
    let root = if nearest_device(&home_trash)? == source_device {
        home_trash
    } else {
        let mut mount = source
            .parent()
            .ok_or_else(|| failure(FsFailureKind::InvalidPath, "source has no mount parent"))?
            .to_path_buf();
        while let Some(parent) = mount.parent() {
            if device(parent)? != source_device {
                break;
            }
            mount = parent.to_path_buf();
        }
        #[cfg(target_os = "macos")]
        let root = mount
            .join(".Trashes")
            .join(rustix::process::geteuid().as_raw().to_string());
        #[cfg(not(target_os = "macos"))]
        let root = {
            let shared = mount.join(".Trash");
            if valid_shared(&shared)? {
                shared.join(rustix::process::geteuid().as_raw().to_string())
            } else {
                mount.join(format!(".Trash-{}", rustix::process::geteuid().as_raw()))
            }
        };
        root
    };
    Ok(ResourceLocation::local(crate::guard::resolve(&root)?))
}
fn device(path: &Path) -> Result<u64, FsFailure> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::metadata(path).map_err(io_failure)?.dev())
}
fn nearest_device(path: &Path) -> Result<u64, FsFailure> {
    use std::os::unix::fs::MetadataExt;
    let mut at = path;
    loop {
        match std::fs::metadata(at) {
            Ok(metadata) => return Ok(metadata.dev()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                at = at.parent().ok_or_else(|| io_failure(error))?;
            }
            Err(error) => return Err(io_failure(error)),
        }
    }
}
#[cfg(target_os = "linux")]
fn valid_shared(path: &Path) -> Result<bool, FsFailure> {
    Ok(observation::stat(path)?.is_some_and(|value| {
        value.kind == EntryKind::Directory
            && value
                .permissions
                .is_some_and(|mode| mode.bits() & 0o1000 != 0)
    }))
}
fn private_directory(path: &Path) -> Result<(), FsFailure> {
    use rustix::fs::Mode;
    if observation::stat(path)?.is_none() {
        let parent = path
            .parent()
            .ok_or_else(|| failure(FsFailureKind::InvalidPath, "Trash directory has no parent"))?;
        if observation::stat(parent)?.is_none() {
            private_directory(parent)?;
        }
        let held = Parent::open(parent)?;
        let name = path
            .file_name()
            .ok_or_else(|| failure(FsFailureKind::InvalidPath, "Trash directory has no name"))?;
        match rustix::fs::mkdirat(&held.file, name, Mode::from_raw_mode(0o700)) {
            Ok(()) => {
                held.file.sync_all().map_err(io_failure)?;
            }
            Err(error) if error == rustix::io::Errno::EXIST => {}
            Err(error) => return Err(io_failure(error.into())),
        }
    }
    validate_private_directory(path)
}
pub(crate) fn validate_private_directory(path: &Path) -> Result<(), FsFailure> {
    let current = observation::stat(path)?
        .ok_or_else(|| failure(FsFailureKind::Conflict, "Trash directory disappeared"))?;
    if current.kind != EntryKind::Directory
        || current.uid != Some(rustix::process::geteuid().as_raw())
        || current
            .permissions
            .is_none_or(|mode| mode.bits() & 0o077 != 0)
    {
        return Err(failure(
            FsFailureKind::Permission,
            "Trash directory is not private and owned; no permanent-delete fallback",
        ));
    }
    Ok(())
}

pub(crate) struct Info {
    parent: Parent,
    name: String,
    file: File,
    identity: ObjectId,
    expected: Option<Observation>,
}
impl Info {
    #[cfg(target_os = "linux")]
    fn create(root: &Path, name: &str) -> Result<Self, FsFailure> {
        use rustix::fs::{Mode, OFlags};
        let directory = root.join("info");
        private_directory(&directory)?;
        let parent = Parent::open(&directory)?;
        let name = format!("{name}.trashinfo");
        let file = rustix::fs::openat(
            &parent.file,
            name.as_str(),
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map(File::from)
        .map_err(|error| io_failure(error.into()))?;
        let identity = observation::metadata(&file.metadata().map_err(io_failure)?)
            .identity
            .ok_or_else(|| {
                failure(
                    FsFailureKind::Unsupported,
                    "Trash information-file identity unavailable",
                )
            })?;
        Ok(Self {
            parent,
            name,
            file,
            identity,
            expected: None,
        })
    }
    pub(crate) fn remove(self) -> Result<(), FsFailure> {
        self.parent.revalidate()?;
        let current = observation::stat(&self.parent.path.join(&self.name))?;
        if current.as_ref().and_then(|value| value.identity) != Some(self.identity) {
            return Err(failure(
                FsFailureKind::Conflict,
                "Trash information file changed identity; cleanup refused",
            ));
        }
        if self
            .expected
            .as_ref()
            .is_some_and(|expected| current.as_ref() != Some(expected))
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "Trash information file changed; cleanup refused",
            ));
        }
        drop(self.file);
        rustix::fs::unlinkat(
            &self.parent.file,
            self.name.as_str(),
            rustix::fs::AtFlags::empty(),
        )
        .map_err(|error| io_failure(error.into()))?;
        self.parent.file.sync_all().map_err(io_failure)
    }
}

pub(crate) fn execute(
    operation: &PreparedOperation,
    token: &CancelToken,
) -> Result<StepOutcome, FsFailure> {
    execute_with(
        operation,
        token,
        |source_parent, source_name, destination_parent, destination_name| {
            rustix::fs::renameat_with(
                source_parent,
                source_name,
                destination_parent,
                destination_name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(crate::local::rename_error)
        },
    )
}

fn execute_with(
    operation: &PreparedOperation,
    token: &CancelToken,
    rename: impl FnOnce(&File, &std::ffi::OsStr, &File, &std::ffi::OsStr) -> Result<(), FsFailure>,
) -> Result<StepOutcome, FsFailure> {
    let source = operation
        .source
        .as_ref()
        .ok_or_else(|| failure(FsFailureKind::InvalidPath, "Trash has no source"))?;
    let mut root = operation
        .capability
        .trash_root
        .clone()
        .ok_or_else(|| failure(FsFailureKind::Unsupported, "native Trash is unavailable"))?;
    let mut warnings = Vec::new();
    #[cfg(target_os = "linux")]
    if let Some(shared) = root
        .path
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == ".Trash"))
    {
        if !valid_shared(shared)? || private_directory(&root.path).is_err() {
            let mount = shared.parent().ok_or_else(|| {
                failure(FsFailureKind::InvalidPath, "shared Trash has no mount root")
            })?;
            root.path = mount.join(format!(".Trash-{}", rustix::process::geteuid().as_raw()));
            warnings.push("shared Trash unavailable; used the private per-volume Trash".into());
        }
    }
    private_directory(&root.path)?;
    #[cfg(target_os = "macos")]
    let files = root.path.clone();
    #[cfg(not(target_os = "macos"))]
    let files = root.path.join("files");
    private_directory(&files)?;
    let destination_parent = Parent::open(&files)?;
    let source_parent = Parent::open(
        source
            .location
            .path
            .parent()
            .ok_or_else(|| failure(FsFailureKind::InvalidPath, "Trash source has no parent"))?,
    )?;
    let source_name = source
        .location
        .path
        .file_name()
        .ok_or_else(|| failure(FsFailureKind::InvalidPath, "Trash source has no basename"))?;
    let approved = source
        .value
        .as_ref()
        .ok_or_else(|| failure(FsFailureKind::Conflict, "Trash source is absent"))?;
    let pinned_source = crate::guard::pin_identity(&source_parent, source_name, approved)?;
    let name = format!("strop-{}", stage::nonce()?);
    let recovery = ResourceLocation::local(files.join(&name));
    #[cfg(target_os = "linux")]
    let mut info = Some(Info::create(&root.path, &name)?);
    #[cfg(not(target_os = "linux"))]
    let info: Option<Info> = None;
    let mut attempted = false;
    let movement = (|| -> Result<(), FsFailure> {
        #[cfg(target_os = "linux")]
        if let Some(info) = info.as_mut() {
            let mount = if root
                .path
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().starts_with(b".Trash-"))
            {
                root.path.parent()
            } else {
                root.path
                    .parent()
                    .filter(|path| path.file_name().is_some_and(|name| name == ".Trash"))
                    .and_then(Path::parent)
            };
            let original = mount
                .and_then(|mount| source.location.path.strip_prefix(mount).ok())
                .unwrap_or(&source.location.path);
            let encoded = strop_workspace::addr::uri::encode_path(original);
            write!(
                info.file,
                "[Trash Info]\nPath={encoded}\nDeletionDate={}\n",
                deletion_date()?
            )
            .map_err(io_failure)?;
            info.file.sync_all().map_err(io_failure)?;
            info.parent.file.sync_all().map_err(io_failure)?;
        }
        if observation::observe(&source.location.path, approved.digest.is_some(), token)?
            != source.value
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "Trash source changed before publication",
            ));
        }
        source_parent.revalidate()?;
        destination_parent.revalidate()?;
        if token.is_cancelled() {
            return Err(failure(
                FsFailureKind::Cancelled,
                "Trash cancelled before publication",
            ));
        }
        attempted = true;
        rename(
            &source_parent.file,
            source_name,
            &destination_parent.file,
            std::ffi::OsStr::new(&name),
        )?;
        Ok(())
    })();
    let publication = if movement.is_ok()
        || (attempted
            && movement
                .as_ref()
                .err()
                .is_some_and(|error| error.kind == FsFailureKind::Io))
    {
        match crate::local::witness(&pinned_source, approved.digest) {
            Ok(owned) => Some(owned),
            Err(error) => {
                return Ok(StepOutcome::Unconfirmed {
                    detail: format!("Trash publication version unavailable: {error}"),
                    observed_destination: None,
                    recovery: Some(recovery.clone()),
                    publication: None,
                })
            }
        }
    } else {
        None
    };
    let uncertain = |detail: String, observed_destination| StepOutcome::Unconfirmed {
        detail,
        observed_destination,
        recovery: Some(recovery.clone()),
        publication,
    };
    if let Err(error) = movement {
        if attempted && error.kind == FsFailureKind::Io {
            return Ok(uncertain(
                format!("Trash publication acknowledgment is uncertain: {error}"),
                None,
            ));
        }
        let cleanup = info.map(Info::remove).transpose();
        return Err(failure(
            error.kind,
            format!(
                "{}{}",
                error.detail,
                cleanup
                    .err()
                    .map(|error| format!("; {error}"))
                    .unwrap_or_default()
            ),
        ));
    }
    if let Err(error) = source_parent
        .file
        .sync_all()
        .and_then(|_| destination_parent.file.sync_all())
    {
        return Ok(uncertain(
            format!("Trash published; synchronization failed: {error}"),
            None,
        ));
    }
    let after = match observation::stat(&recovery.path) {
        Ok(after) => after,
        Err(error) => return Ok(uncertain(error.to_string(), None)),
    };
    if !source
        .value
        .as_ref()
        .zip(after.as_ref())
        .is_some_and(|(before, after)| {
            before.same_object(after)
                && publication.is_some_and(|owned| owned.matches_metadata(after))
        })
    {
        return Ok(uncertain(
            "Trash outcome requires identity verification".into(),
            after,
        ));
    }
    let source_after = match observation::stat(&source.location.path) {
        Ok(after) => after,
        Err(error) => return Ok(uncertain(error.to_string(), after)),
    };
    Ok(StepOutcome::Committed {
        source_after,
        destination_after: after,
        recovery: Some(recovery),
        warnings,
        publication,
    })
}

#[cfg(target_os = "linux")]
fn deletion_date() -> Result<String, FsFailure> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| failure(FsFailureKind::Io, error.to_string()))?
        .as_secs();
    let seconds = seconds
        .try_into()
        .map_err(|_| failure(FsFailureKind::Io, "Trash timestamp is out of range"))?;
    let mut output = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: localtime_r receives live, correctly sized pointers and initializes
    // `tm` on non-null success. Rust std has no local calendar/timezone API.
    let converted = unsafe { libc::localtime_r(&seconds, output.as_mut_ptr()) };
    if converted.is_null() {
        return Err(failure(
            FsFailureKind::Io,
            "local Trash timestamp conversion failed",
        ));
    }
    // SAFETY: the successful localtime_r call initialized every tm field.
    let date = unsafe { output.assume_init() };
    Ok(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        date.tm_year + 1900,
        date.tm_mon + 1,
        date.tm_mday,
        date.tm_hour,
        date.tm_min,
        date.tm_sec
    ))
}
