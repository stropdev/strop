use super::*;
use outcome::publication_error;
use sha2::{Digest, Sha256};
fn parent_expected<'a>(
    operation: &'a PreparedOperation,
    parent: &'a LocatedObservation,
    receipts: &'a [StepReceipt],
) -> Option<&'a Observation> {
    if let Some(value) = &parent.value {
        return Some(value);
    }
    operation.dependencies.iter().find_map(|step| {
        receipts
            .iter()
            .find(|receipt| receipt.step == *step)
            .and_then(|receipt| {
                if receipt.operation.destination.as_ref()?.location != parent.location {
                    return None;
                }
                match &receipt.outcome {
                    StepOutcome::Committed {
                        destination_after, ..
                    } => destination_after.as_ref(),
                    _ => None,
                }
            })
    })
}
pub fn execute(
    operation: &PreparedOperation,
    contents: Option<&ropey::Rope>,
    receipts: &[StepReceipt],
    token: &CancelToken,
) -> StepOutcome {
    match execute_checked(operation, contents, receipts, token) {
        Ok(outcome) => outcome,
        Err(error) if error.kind == FsFailureKind::Cancelled => StepOutcome::Cancelled {
            detail: error.detail,
        },
        Err(error) => StepOutcome::Refused(error),
    }
}
fn execute_checked(
    operation: &PreparedOperation,
    contents: Option<&ropey::Rope>,
    receipts: &[StepReceipt],
    token: &CancelToken,
) -> Result<StepOutcome, FsFailure> {
    if token.is_cancelled() {
        return Err(failure(
            FsFailureKind::Cancelled,
            "cancelled before publication",
        ));
    }
    let buffer_copy = operation.intent.kind == OperationKind::Copy
        && operation.intent.copy_version == CopyVersion::Buffer;
    if buffer_copy != contents.is_some() {
        return Err(failure(
            FsFailureKind::Protocol,
            "copy snapshot does not match the approved content version",
        ));
    }
    let current = capability()?;
    if operation.capability.principal != current.principal
        || operation.capability.incarnation != current.incarnation
    {
        return Err(failure(
            FsFailureKind::Conflict,
            "prepared process/principal capability changed",
        ));
    }
    for (input, observed) in [
        (operation.intent.source.as_ref(), operation.source.as_ref()),
        (
            operation.intent.destination.as_ref(),
            operation.destination.as_ref(),
        ),
    ] {
        if let (Some(input), Some(observed)) = (input, observed) {
            if input.filesystem != observed.location.filesystem
                || guard::resolve(guard::local_path(input)?)? != observed.location.path
            {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "a logical path alias or namespace changed since preparation",
                ));
            }
        }
    }
    let mut parents = Vec::new();
    for expected in &operation.parents {
        let held = Parent::open(guard::local_path(&expected.location)?)?;
        let expected = parent_expected(operation, expected, receipts).ok_or_else(|| {
            failure(
                FsFailureKind::Conflict,
                "a required parent creation did not commit",
            )
        })?;
        if expected.identity != Some(held.identity) {
            return Err(failure(
                FsFailureKind::Conflict,
                "parent changed since preparation",
            ));
        }
        parents.push(held);
    }
    let parent_of = |resource: &LocatedObservation| -> Result<&Parent, FsFailure> {
        parents
            .iter()
            .find(|parent| Some(parent.path.as_path()) == resource.location.path.parent())
            .ok_or_else(|| failure(FsFailureKind::Protocol, "prepared parent is missing"))
    };
    let mut names: Vec<_> = operation
        .source
        .iter()
        .filter(|source| !buffer_copy || source.value.is_some())
        .chain(operation.destination.iter())
        .collect();
    names.sort_by(|a, b| a.location.path.cmp(&b.location.path));
    names.dedup_by(|a, b| a.location == b.location);
    let mut locks = Vec::new();
    for resource in names {
        let managed_source = operation
            .source
            .as_ref()
            .is_some_and(|source| source.location == resource.location)
            && matches!(
                operation.intent.kind,
                OperationKind::Copy | OperationKind::Restore
            );
        if let Some(root) = operation
            .capability
            .trash_root
            .as_ref()
            .filter(|_| managed_source)
        {
            let parent = parents
                .iter()
                .find(|parent| parent.path == root.path)
                .ok_or_else(|| {
                    failure(
                        FsFailureKind::Protocol,
                        "native Trash coordination parent is missing",
                    )
                })?;
            locks.push(NameLock::acquire(
                parent,
                resource.location.path.as_os_str(),
            )?);
        } else {
            let name =
                resource.location.path.file_name().ok_or_else(|| {
                    failure(FsFailureKind::InvalidPath, "resource has no basename")
                })?;
            locks.push(NameLock::acquire(parent_of(resource)?, name)?);
        }
    }
    if let Some(source) = &operation.source {
        let actual = observation::observe(
            &source.location.path,
            source
                .value
                .as_ref()
                .is_some_and(|value| value.digest.is_some()),
            token,
        )?;
        if actual != source.value {
            return Err(failure(
                FsFailureKind::Conflict,
                "source changed since preparation",
            ));
        }
        if source.value.is_some() {
            parent_of(source)?.exact_existing_name(
                source
                    .location
                    .path
                    .file_name()
                    .ok_or_else(|| failure(FsFailureKind::InvalidPath, "source has no basename"))?,
            )?;
        }
    }
    if let Some(destination) = &operation.destination {
        if observation::stat(&destination.location.path)?.is_some() {
            return Err(failure(
                FsFailureKind::Conflict,
                "destination is occupied; no overwrite",
            ));
        }
        if let Some(expected) = &destination.value {
            let vacated = operation.dependencies.iter().any(|step| {
                receipts.iter().any(|receipt| {
                    receipt.step == *step
                        && receipt.outcome.is_committed()
                        && receipt.operation.source.as_ref().is_some_and(|source| {
                            source.location == destination.location
                                && source
                                    .value
                                    .as_ref()
                                    .is_some_and(|source| source.same_object(expected))
                        })
                })
            });
            if !vacated {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "destination vacancy was not established by this plan",
                ));
            }
        }
    }
    let restore_info = if operation.intent.kind == OperationKind::Restore {
        crate::trash::prepare_restore(operation)?
    } else {
        None
    };
    for lock in &locks {
        lock.revalidate()?;
    }
    for parent in &parents {
        parent.revalidate()?;
    }
    if token.is_cancelled() {
        return Err(failure(
            FsFailureKind::Cancelled,
            "cancelled before publication",
        ));
    }
    use rustix::fs::{AtFlags, Mode, OFlags, RenameFlags};
    let destination = operation.destination.as_ref();
    let source = operation.source.as_ref();
    let name = |resource: &LocatedObservation| {
        resource
            .location
            .path
            .file_name()
            .map(ToOwned::to_owned)
            .ok_or_else(|| failure(FsFailureKind::InvalidPath, "resource has no basename"))
    };
    let mut publication = None;
    // Hold created objects until their final pathname observation, so unlink and
    // inode reuse cannot substitute a different object before acknowledgement.
    let mut created = None;
    let mut removed_directory = None;
    let mut removed_locks = 0;
    match operation.intent.kind {
        OperationKind::CreateFile => {
            let destination = destination
                .ok_or_else(|| failure(FsFailureKind::Protocol, "missing creation destination"))?;
            let parent = parent_of(destination)?;
            let file = match rustix::fs::openat(
                &parent.file,
                name(destination)?,
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o666),
            ) {
                Ok(file) => File::from(file),
                Err(error) => {
                    return Ok(publication_error(operation, io_failure(error.into()), None))
                }
            };
            publication = match witness(&file, Some(Sha256::digest([]).into())) {
                Ok(value) => Some(value),
                Err(error) => return Ok(uncertain(operation, error.to_string(), None)),
            };
            if let Err(error) = file.sync_all() {
                return Ok(uncertain(operation, error.to_string(), publication));
            }
            created = Some(file);
        }
        OperationKind::CreateDirectory => {
            let destination = destination
                .ok_or_else(|| failure(FsFailureKind::Protocol, "missing creation destination"))?;
            let parent = parent_of(destination)?;
            if let Err(error) =
                rustix::fs::mkdirat(&parent.file, name(destination)?, Mode::from_raw_mode(0o777))
            {
                return Ok(publication_error(operation, io_failure(error.into()), None));
            }
            let file = match rustix::fs::openat(
                &parent.file,
                name(destination)?,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(file) => File::from(file),
                Err(error) => return Ok(uncertain(operation, error.to_string(), None)),
            };
            publication = match witness(&file, None) {
                Ok(value) => Some(value),
                Err(error) => return Ok(uncertain(operation, error.to_string(), None)),
            };
            if let Err(error) = file.sync_all() {
                return Ok(uncertain(operation, error.to_string(), publication));
            }
            created = Some(file);
        }
        OperationKind::Rename | OperationKind::Restore => {
            let source =
                source.ok_or_else(|| failure(FsFailureKind::Protocol, "missing rename source"))?;
            let destination = destination
                .ok_or_else(|| failure(FsFailureKind::Protocol, "missing rename destination"))?;
            let source_parent = parent_of(source)?;
            let source_name = name(source)?;
            let approved = source
                .value
                .as_ref()
                .ok_or_else(|| failure(FsFailureKind::Conflict, "rename source is absent"))?;
            let file = guard::pin_identity(source_parent, &source_name, approved)?;
            let movement = rustix::fs::renameat_with(
                &source_parent.file,
                &source_name,
                &parent_of(destination)?.file,
                name(destination)?,
                RenameFlags::NOREPLACE,
            )
            .map_err(rename_error);
            let movement_error = match movement {
                Ok(()) => None,
                Err(error) if error.kind != FsFailureKind::Io => {
                    return Ok(StepOutcome::Refused(error))
                }
                Err(error) => Some(error),
            };
            publication = match witness(&file, approved.digest) {
                Ok(owned) => Some(owned),
                Err(error) => {
                    return Ok(uncertain(
                        operation,
                        format!("move publication version unavailable: {error}"),
                        None,
                    ))
                }
            };
            created = Some(file);
            if let Some(error) = movement_error {
                return Ok(publication_error(operation, error, publication));
            }
        }
        OperationKind::Copy => {
            let source =
                source.ok_or_else(|| failure(FsFailureKind::Protocol, "missing copy source"))?;
            let destination = destination
                .ok_or_else(|| failure(FsFailureKind::Protocol, "missing copy destination"))?;
            let source_parent = if source.value.is_some() {
                Some(parent_of(source)?)
            } else {
                None
            };
            return copy::copy(
                operation,
                source,
                destination,
                source_parent,
                parent_of(destination)?,
                contents,
                token,
            );
        }
        OperationKind::Remove => {
            let source =
                source.ok_or_else(|| failure(FsFailureKind::Protocol, "missing removal source"))?;
            let parent = parent_of(source)?;
            let source_name = name(source)?;
            let flags = if let Some(approved) = source
                .value
                .as_ref()
                .filter(|source| source.kind == EntryKind::Directory)
            {
                let (directory, count) =
                    guard::prune_directory_locks(parent, &source_name, approved, token)?;
                removed_directory = Some(directory);
                removed_locks = count;
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            };
            if token.is_cancelled() {
                return Err(failure(FsFailureKind::Cancelled, format!("removal cancelled before publication; {removed_locks} protocol lock removals confirmed")));
            }
            if let Err(error) = rustix::fs::unlinkat(&parent.file, &source_name, flags) {
                let mut failure = if error == rustix::io::Errno::NOTEMPTY {
                    failure(
                        FsFailureKind::Unsupported,
                        "non-empty directory removal is not authorized",
                    )
                } else {
                    io_failure(error.into())
                };
                if removed_locks > 0 {
                    failure.detail.push_str(&format!(
                        "; {removed_locks} captured protocol locks retired"
                    ));
                }
                return Ok(publication_error(operation, failure, None));
            }
        }
        OperationKind::Trash => return crate::trash::execute(operation, token),
    }
    for parent in &parents {
        if let Err(error) = parent
            .file
            .sync_all()
            .map_err(io_failure)
            .and_then(|_| parent.revalidate())
        {
            return Ok(uncertain(operation, error.to_string(), publication));
        }
    }
    let mut warnings: Vec<_> = restore_info
        .and_then(|info| info.remove().err())
        .map(|error| format!("restored data; Trash metadata cleanup failed: {error}"))
        .into_iter()
        .collect();
    if removed_locks > 0 {
        warnings.push(format!(
            "retired {removed_locks} quiescent protocol locks before empty-directory removal"
        ));
    }
    let result = committed(operation, warnings, publication);
    drop(created);
    drop(removed_directory);
    result
}
