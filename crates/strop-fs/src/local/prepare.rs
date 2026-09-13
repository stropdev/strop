use super::*;
pub fn prepare(
    intent: &OperationIntent,
    allow_occupied: bool,
    environment: &crate::Environment,
    token: &CancelToken,
) -> Result<PreparedOperation, FsFailure> {
    let buffer_copy =
        intent.kind == OperationKind::Copy && intent.copy_version == CopyVersion::Buffer;
    let resolve = |location: &ResourceLocation| -> Result<ResourceLocation, FsFailure> {
        Ok(ResourceLocation::local(guard::resolve(guard::local_path(
            location,
        )?)?))
    };
    let source = intent.source.as_ref().map(resolve).transpose()?;
    let destination = intent.destination.as_ref().map(resolve).transpose()?;
    if source.is_some() && source == destination {
        return Err(failure(
            FsFailureKind::Conflict,
            "source and destination are the same resource",
        ));
    }
    let digest =
        intent.expected_content.is_some() || (intent.kind == OperationKind::Copy && !buffer_copy);
    let source = source
        .map(|location| {
            observation::observe(&location.path, digest, token)
                .map(|value| LocatedObservation { location, value })
        })
        .transpose()?;
    let destination = destination
        .map(|location| {
            observation::observe(&location.path, false, token)
                .map(|value| LocatedObservation { location, value })
        })
        .transpose()?;
    if let Some(expected) = intent.expected_content {
        if source
            .as_ref()
            .and_then(|source| source.value.as_ref())
            .and_then(|value| value.digest)
            != Some(expected)
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "source content no longer matches the receipt's intended bytes",
            ));
        }
    }
    match intent.kind {
        OperationKind::CreateFile | OperationKind::CreateDirectory => {
            if source.is_some() || destination.is_none() {
                return Err(failure(
                    FsFailureKind::InvalidPath,
                    "creation requires one destination",
                ));
            }
        }
        _ => {
            let before = source.as_ref().ok_or_else(|| {
                failure(FsFailureKind::InvalidPath, "operation requires a source")
            })?;
            if before.value.is_none() && !buffer_copy {
                return Err(failure(FsFailureKind::Conflict, "source no longer exists"));
            }
            if let Some(observation) = &before.value {
                if !matches!(observation.kind, EntryKind::File | EntryKind::Directory) {
                    return Err(failure(
                        FsFailureKind::Unsupported,
                        "link and special-file mutations are unsupported",
                    ));
                }
                if intent.kind != OperationKind::Copy
                    && observation.kind == EntryKind::File
                    && observation.links != Some(1)
                {
                    return Err(failure(
                        FsFailureKind::Unsupported,
                        "hard-linked file mutation requires explicit alias handling",
                    ));
                }
                if intent.kind == OperationKind::Copy && observation.kind != EntryKind::File {
                    return Err(failure(
                        FsFailureKind::Unsupported,
                        "copy supports regular files, not recursive directories",
                    ));
                }
                if observation.kind == EntryKind::Directory
                    && destination
                        .as_ref()
                        .is_some_and(|after| after.location.path.starts_with(&before.location.path))
                {
                    return Err(failure(
                        FsFailureKind::InvalidPath,
                        "a directory cannot move into itself",
                    ));
                }
            }
        }
    }
    if matches!(
        intent.kind,
        OperationKind::Rename | OperationKind::Copy | OperationKind::Restore
    ) && destination.is_none()
    {
        return Err(failure(
            FsFailureKind::InvalidPath,
            "operation requires a destination",
        ));
    }
    if matches!(intent.kind, OperationKind::Trash | OperationKind::Remove) && destination.is_some()
    {
        return Err(failure(
            FsFailureKind::InvalidPath,
            "removal does not accept a destination",
        ));
    }
    if destination
        .as_ref()
        .is_some_and(|destination| destination.value.is_some())
        && !allow_occupied
    {
        return Err(failure(
            FsFailureKind::Conflict,
            "destination already exists; overwrite is not authorized",
        ));
    }
    let managed_source = source
        .as_ref()
        .map(|source| crate::trash::managed_root(&source.location.path, environment))
        .transpose()?
        .flatten();
    if managed_source.is_some()
        && !matches!(intent.kind, OperationKind::Copy | OperationKind::Restore)
    {
        return Err(failure(
            FsFailureKind::Unsupported,
            "native Trash storage is protected; use its checked restore action",
        ));
    }
    if intent.kind == OperationKind::Restore && managed_source.is_none() {
        return Err(failure(
            FsFailureKind::Unsupported,
            "restore source is not in the captured native Trash profile",
        ));
    }
    if destination
        .as_ref()
        .map(|destination| crate::trash::managed_root(&destination.location.path, environment))
        .transpose()?
        .flatten()
        .is_some()
    {
        return Err(failure(
            FsFailureKind::Unsupported,
            "native Trash storage is not a generic operation destination",
        ));
    }
    let mut parents = Vec::new();
    for resource in source
        .iter()
        .filter(|source| !buffer_copy || source.value.is_some())
        .chain(destination.iter())
    {
        let path = resource
            .location
            .path
            .parent()
            .ok_or_else(|| failure(FsFailureKind::InvalidPath, "resource has no parent"))?;
        if parents
            .iter()
            .any(|parent: &LocatedObservation| parent.location.path == path)
        {
            continue;
        }
        let value = observation::observe(path, false, token)?;
        if value
            .as_ref()
            .is_some_and(|value| value.kind != EntryKind::Directory)
        {
            return Err(failure(
                FsFailureKind::InvalidPath,
                "parent is not a directory",
            ));
        }
        parents.push(LocatedObservation {
            location: ResourceLocation::local(path.to_path_buf()),
            value,
        });
    }
    if let Some(root) = &managed_source {
        crate::trash::validate_private_directory(&root.path)?;
        if !parents.iter().any(|parent| parent.location == *root) {
            parents.push(LocatedObservation {
                location: root.clone(),
                value: observation::observe(&root.path, false, token)?,
            });
        }
    }
    let mut capability = capability()?;
    capability.trash_root = if intent.kind == OperationKind::Trash {
        Some(crate::trash::root_for(
            &source
                .as_ref()
                .ok_or_else(|| failure(FsFailureKind::InvalidPath, "Trash requires a source"))?
                .location
                .path,
            environment,
        )?)
    } else {
        managed_source
    };
    Ok(PreparedOperation {
        intent: intent.clone(),
        source,
        destination,
        parents,
        dependencies: Vec::new(),
        capability,
    })
}
