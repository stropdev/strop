use super::*;
pub fn prepare(
    intent: &OperationIntent,
    allow_occupied: bool,
    environment: &crate::Environment,
    context: &crate::ExecutionContext,
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
    // Edit admission (Store with a displayed-snapshot condition) digests
    // the destination: admission must prove the edited bytes ARE the
    // stored ones, never trust a name.
    let destination_digest = intent.kind == OperationKind::Store
        && intent
            .store
            .as_ref()
            .is_some_and(|policy| policy.displayed.is_some());
    let source = source
        .map(|location| {
            observation::observe(&location.path, digest, false, token)
                .map(|value| LocatedObservation { location, value })
        })
        .transpose()?;
    let destination = destination
        .map(|location| {
            observation::observe(
                &location.path,
                destination_digest,
                intent.kind == OperationKind::Store,
                token,
            )
            .map(|value| LocatedObservation { location, value })
        })
        .transpose()?;
    // Store's expected_content names the intended DESTINATION bytes (the
    // frozen save content), verified at effect time — it has no source.
    if intent.kind != OperationKind::Store {
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
    }
    match intent.kind {
        OperationKind::Store => {
            let policy = intent.store.ok_or_else(|| {
                failure(
                    FsFailureKind::Protocol,
                    "store intent carries no conditional policy",
                )
            })?;
            if policy.displayed.is_none() && intent.expected_content.is_none() {
                return Err(failure(
                    FsFailureKind::Protocol,
                    "store requires a displayed baseline or intended content",
                ));
            }
            if source.is_some() || destination.is_none() {
                return Err(failure(
                    FsFailureKind::InvalidPath,
                    "store requires exactly one destination and no source",
                ));
            }
            let observed = destination
                .as_ref()
                .and_then(|destination| destination.value.as_ref());
            if let Some(observation) = observed {
                if observation.kind != EntryKind::File {
                    return Err(failure(
                        FsFailureKind::Unsupported,
                        "store replaces only regular files",
                    ));
                }
                if observation.links != Some(1) {
                    return Err(failure(
                        FsFailureKind::Unsupported,
                        "hard-linked file mutation requires explicit alias handling",
                    ));
                }
            }
            if policy.baseline_object.is_some()
                && observed.and_then(|value| value.identity) != policy.baseline_object
            {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "stored file identity changed since edit admission",
                ));
            }
            if policy.baseline_attributes.is_some()
                && observed.and_then(|value| value.attributes) != policy.baseline_attributes
            {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "stored file attributes changed since edit admission",
                ));
            }
            if let Some(displayed) = policy.displayed {
                if observed.and_then(|value| value.digest) != Some(displayed) {
                    return Err(failure(
                        FsFailureKind::Conflict,
                        "displayed snapshot differs from the stored content; refresh before editing",
                    ));
                }
            }
            // Admission checks only the displayed bytes; an actual save
            // also retains the prior mtime, even when its content digest
            // is checked independently.
            if !policy.force && intent.expected_content.is_some() {
                if policy.expect_absent {
                    if observed.is_some() {
                        return Err(failure(
                            FsFailureKind::Conflict,
                            "file exists — :w! to overwrite",
                        ));
                    }
                } else if observed.and_then(|value| value.modified) != policy.baseline {
                    return Err(failure(
                        FsFailureKind::Conflict,
                        "file changed on disk — :w! to force",
                    ));
                }
            }
        }
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
    // A Store's occupied destination is its own baseline contract (checked
    // above); the vacancy refusal protects the create/rename family.
    if destination
        .as_ref()
        .is_some_and(|destination| destination.value.is_some())
        && !allow_occupied
        && intent.kind != OperationKind::Store
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
        let value = observation::observe(path, false, false, token)?;
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
                value: observation::observe(&root.path, false, false, token)?,
            });
        }
    }
    if intent.kind == OperationKind::Store {
        // Document saves never synthesize parents (no silent mkdir -p —
        // the local writer's ENOENT), so batch orchestration never sees a
        // store with a missing parent to synthesize.
        if parents.iter().any(|parent| parent.value.is_none()) {
            return Err(failure(
                FsFailureKind::Io,
                "destination's parent directory does not exist",
            ));
        }
        // Edit admission keeps the helper's ownership posture: the file
        // being armed for editing is owned by the worker's principal.
        if intent
            .store
            .as_ref()
            .is_some_and(|policy| policy.displayed.is_some())
        {
            let owned = destination
                .as_ref()
                .and_then(|destination| destination.value.as_ref())
                .and_then(|value| value.uid);
            let principal = context.capability()?.principal;
            if owned.is_none() || principal.is_none() || owned != principal {
                return Err(failure(
                    FsFailureKind::Permission,
                    "saving requires a file owned by the authenticated user",
                ));
            }
        }
    }
    let mut capability = context.capability()?;
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
