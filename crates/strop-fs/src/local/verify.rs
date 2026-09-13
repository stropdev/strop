use super::*;
/// Reconcile observed names against before-state and owned publication evidence.
/// Occupancy, matching bytes or an untrusted post-error stat alone grant no ownership.
pub fn verify(receipt: &StepReceipt, token: &CancelToken) -> Result<VerifiedOutcome, FsFailure> {
    let operation = &receipt.operation;
    let current = capability()?;
    if operation.capability.principal != current.principal
        || operation.capability.incarnation != current.incarnation
    {
        return Err(failure(
            FsFailureKind::Conflict,
            "verification principal/process capability changed",
        ));
    }
    let source = operation
        .source
        .as_ref()
        .map(|source| {
            observation::observe(
                guard::local_path(&source.location)?,
                source
                    .value
                    .as_ref()
                    .is_some_and(|value| value.digest.is_some()),
                token,
            )
        })
        .transpose()?
        .flatten();
    let (recovery, publication) = match &receipt.outcome {
        StepOutcome::Committed {
            recovery,
            publication,
            ..
        }
        | StepOutcome::Unconfirmed {
            recovery,
            publication,
            ..
        } => (recovery.as_ref(), *publication),
        _ => (None, None),
    };
    let target = recovery.or_else(|| {
        operation
            .destination
            .as_ref()
            .map(|destination| &destination.location)
    });
    let mut destination = target
        .map(|target| observation::observe(guard::local_path(target)?, false, token))
        .transpose()?
        .flatten();
    let before_source = operation
        .source
        .as_ref()
        .and_then(|source| source.value.as_ref());
    let before_destination = operation
        .destination
        .as_ref()
        .and_then(|destination| destination.value.as_ref());
    if source.as_ref() == before_source && destination.as_ref() == before_destination {
        return Ok(VerifiedOutcome::Unchanged);
    }
    let intended = match operation.intent.kind {
        OperationKind::Remove => source.is_none(),
        kind => {
            let kind_matches = match kind {
                OperationKind::Rename | OperationKind::Restore | OperationKind::Trash => {
                    source.is_none()
                        && before_source
                            .zip(destination.as_ref())
                            .is_some_and(|(before, after)| before.same_object(after))
                }
                OperationKind::CreateDirectory => destination
                    .as_ref()
                    .is_some_and(|after| after.kind == EntryKind::Directory),
                _ => destination
                    .as_ref()
                    .is_some_and(|after| after.kind == EntryKind::File),
            };
            if let Some(owned) = publication {
                let metadata_matches = kind_matches
                    && destination
                        .as_ref()
                        .is_some_and(|after| owned.matches_metadata(after));
                if metadata_matches && owned.content.is_some() {
                    destination = target
                        .map(|target| observation::observe(guard::local_path(target)?, true, token))
                        .transpose()?
                        .flatten();
                }
                metadata_matches
                    && destination.as_ref().is_some_and(|after| {
                        owned.matches_metadata(after)
                            && owned
                                .content
                                .is_none_or(|content| after.digest == Some(content))
                    })
            } else {
                false
            }
        }
    };
    if intended {
        synchronize(operation, target, destination.as_ref(), token)?;
    }
    Ok(if intended {
        VerifiedOutcome::Committed(Box::new(strop_workspace::operation::VerifiedChange {
            source_after: source,
            destination_after: destination,
        }))
    } else {
        VerifiedOutcome::Unknown {
            detail: "current names do not prove the intended object/version; no automatic retry"
                .into(),
        }
    })
}

fn synchronize(
    operation: &PreparedOperation,
    target: Option<&ResourceLocation>,
    observed: Option<&Observation>,
    token: &CancelToken,
) -> Result<(), FsFailure> {
    use rustix::fs::{Mode, OFlags};
    if token.is_cancelled() {
        return Err(failure(
            FsFailureKind::Cancelled,
            "verification cancelled before synchronization",
        ));
    }
    if let (Some(target), Some(observed)) = (target, observed) {
        let path = guard::local_path(target)?;
        let parent = Parent::open(path.parent().ok_or_else(|| {
            failure(
                FsFailureKind::InvalidPath,
                "verification target has no parent",
            )
        })?)?;
        let name = path.file_name().ok_or_else(|| {
            failure(
                FsFailureKind::InvalidPath,
                "verification target has no basename",
            )
        })?;
        let file = rustix::fs::openat(
            &parent.file,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|error| io_failure(error.into()))?;
        if !observation::metadata(&file.metadata().map_err(io_failure)?).same_metadata(observed) {
            return Err(failure(
                FsFailureKind::Conflict,
                "verification target changed before synchronization",
            ));
        }
        file.sync_all().map_err(io_failure)?;
        parent.file.sync_all().map_err(io_failure)?;
        parent.revalidate()?;
        if observation::stat(path)?
            .as_ref()
            .is_none_or(|current| !current.same_metadata(observed))
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "verification target changed during synchronization",
            ));
        }
    }
    for expected in &operation.parents {
        let parent = Parent::open(guard::local_path(&expected.location)?)?;
        if expected
            .value
            .as_ref()
            .is_some_and(|value| value.identity != Some(parent.identity))
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "verification parent changed identity",
            ));
        }
        parent.file.sync_all().map_err(io_failure)?;
        parent.revalidate()?;
    }
    Ok(())
}
