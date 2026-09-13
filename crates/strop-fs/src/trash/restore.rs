use super::*;

/// Identify protected native Trash storage from the captured platform profile.
/// Names are only a cheap probe filter; the resolved profile decides authority.
pub(crate) fn managed_root(
    path: &Path,
    environment: &Environment,
) -> Result<Option<ResourceLocation>, FsFailure> {
    let looks_like_trash = path.components().any(|component| {
        let name = component.as_os_str().as_encoded_bytes();
        matches!(name, b"Trash" | b".Trash" | b".Trashes") || name.starts_with(b".Trash-")
    });
    if !looks_like_trash || environment.home.is_none() {
        return Ok(None);
    }
    let mut existing = path;
    while observation::stat(existing)?.is_none() {
        existing = existing.parent().ok_or_else(|| {
            failure(
                FsFailureKind::InvalidPath,
                "Trash profile has no existing ancestor",
            )
        })?;
    }
    let root = root_for(existing, environment)?;
    if path.starts_with(&root.path) {
        Ok(Some(root))
    } else {
        Ok(None)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn prepare_restore(operation: &PreparedOperation) -> Result<Option<Info>, FsFailure> {
    use rustix::fs::{Mode, OFlags};
    use std::io::Read;
    let source = operation
        .source
        .as_ref()
        .ok_or_else(|| failure(FsFailureKind::InvalidPath, "restore requires a source"))?;
    let destination = operation
        .destination
        .as_ref()
        .ok_or_else(|| failure(FsFailureKind::InvalidPath, "restore requires a destination"))?;
    let root = operation.capability.trash_root.as_ref().ok_or_else(|| {
        failure(
            FsFailureKind::Unsupported,
            "restore source has no native Trash profile",
        )
    })?;
    if source.location.path.parent() != Some(root.path.join("files").as_path()) {
        return Err(failure(
            FsFailureKind::Unsupported,
            "restore requires a whole native Trash entry",
        ));
    }
    validate_private_directory(&root.path)?;
    let directory = root.path.join("info");
    validate_private_directory(&directory)?;
    let parent = Parent::open(&directory)?;
    let source_name = source
        .location
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            failure(
                FsFailureKind::Unsupported,
                "Trash entry has no supported information-file name",
            )
        })?;
    let name = format!("{source_name}.trashinfo");
    let file = rustix::fs::openat(
        &parent.file,
        name.as_str(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| io_failure(error.into()))?;
    let before = observation::metadata(&file.metadata().map_err(io_failure)?);
    if before.kind != EntryKind::File
        || before.uid != Some(rustix::process::geteuid().as_raw())
        || before.links != Some(1)
        || before
            .permissions
            .is_none_or(|mode| mode.bits() & 0o077 != 0)
        || before.size.is_none_or(|size| size > 16 * 1024)
    {
        return Err(failure(
            FsFailureKind::Permission,
            "Trash information file is not bounded, private and owned",
        ));
    }
    let mut content = Vec::new();
    (&file)
        .take(16 * 1024 + 1)
        .read_to_end(&mut content)
        .map_err(io_failure)?;
    if content.len() > 16 * 1024
        || observation::metadata(&file.metadata().map_err(io_failure)?) != before
    {
        return Err(failure(
            FsFailureKind::Conflict,
            "Trash information changed while reading",
        ));
    }
    let text = std::str::from_utf8(&content)
        .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?;
    if text.lines().next() != Some("[Trash Info]") {
        return Err(failure(
            FsFailureKind::Protocol,
            "invalid Trash information header",
        ));
    }
    let mut paths = text.lines().filter_map(|line| line.strip_prefix("Path="));
    let encoded = paths.next().ok_or_else(|| {
        failure(
            FsFailureKind::Protocol,
            "Trash information omitted the original path",
        )
    })?;
    if paths.next().is_some() {
        return Err(failure(
            FsFailureKind::Protocol,
            "Trash information has duplicate original paths",
        ));
    }
    let original = strop_workspace::addr::uri::decode_path(encoded)
        .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?;
    let original = if original.is_absolute() {
        original
    } else {
        if original
            .components()
            .any(|component| component == std::path::Component::ParentDir)
        {
            return Err(failure(
                FsFailureKind::InvalidPath,
                "relative Trash paths cannot contain parent traversal",
            ));
        }
        let base = root
            .path
            .parent()
            .ok_or_else(|| failure(FsFailureKind::InvalidPath, "Trash has no path base"))?;
        let base = if base.file_name().is_some_and(|name| name == ".Trash") {
            base.parent().ok_or_else(|| {
                failure(
                    FsFailureKind::InvalidPath,
                    "shared Trash has no volume root",
                )
            })?
        } else {
            base
        };
        base.join(original)
    };
    if original != destination.location.path {
        return Err(failure(
            FsFailureKind::Conflict,
            "Trash information no longer names the approved restore destination",
        ));
    }
    let identity = before.identity.ok_or_else(|| {
        failure(
            FsFailureKind::Unsupported,
            "Trash information identity unavailable",
        )
    })?;
    Ok(Some(Info {
        parent,
        name,
        file,
        identity,
        expected: Some(before),
    }))
}

#[cfg(target_os = "macos")]
pub(crate) fn prepare_restore(_: &PreparedOperation) -> Result<Option<Info>, FsFailure> {
    // The macOS native Trash layout has no freedesktop .trashinfo sidecar.
    Ok(None)
}
