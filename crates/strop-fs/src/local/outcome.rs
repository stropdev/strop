use super::*;

pub(super) fn uncertain(
    operation: &PreparedOperation,
    mut detail: String,
    publication: Option<PublicationWitness>,
) -> StepOutcome {
    let observed_destination = match operation
        .destination
        .as_ref()
        .map(|destination| observation::stat(&destination.location.path))
        .transpose()
    {
        Ok(value) => value.flatten(),
        Err(error) => {
            detail.push_str(&format!("; post-publication observation: {error}"));
            None
        }
    };
    StepOutcome::Unconfirmed {
        detail,
        observed_destination,
        recovery: None,
        publication,
    }
}
pub(super) fn publication_error(
    operation: &PreparedOperation,
    error: FsFailure,
    publication: Option<PublicationWitness>,
) -> StepOutcome {
    if error.kind == FsFailureKind::Io {
        uncertain(operation, error.to_string(), publication)
    } else {
        StepOutcome::Refused(error)
    }
}
pub(super) fn committed(
    operation: &PreparedOperation,
    warnings: Vec<String>,
    publication: Option<PublicationWitness>,
) -> Result<StepOutcome, FsFailure> {
    let source_after = operation
        .source
        .as_ref()
        .map(|source| observation::stat(&source.location.path))
        .transpose();
    let destination_after = operation
        .destination
        .as_ref()
        .map(|destination| observation::stat(&destination.location.path))
        .transpose();
    match (source_after, destination_after) {
        (Ok(source), Ok(destination)) => {
            let source = source.flatten();
            let destination = destination.flatten();
            let valid = match operation.intent.kind {
                OperationKind::Rename | OperationKind::Restore => {
                    source.is_none()
                        && operation
                            .source
                            .as_ref()
                            .and_then(|source| source.value.as_ref())
                            .zip(destination.as_ref())
                            .is_some_and(|(before, after)| before.same_object(after))
                        && publication
                            .zip(destination.as_ref())
                            .is_some_and(|(owned, after)| owned.matches_metadata(after))
                }
                OperationKind::CreateFile
                | OperationKind::CreateDirectory
                | OperationKind::Copy => {
                    publication
                        .zip(destination.as_ref())
                        .is_some_and(|(owned, after)| {
                            owned.matches_metadata(after)
                                && after.kind
                                    == if operation.intent.kind == OperationKind::CreateDirectory {
                                        EntryKind::Directory
                                    } else {
                                        EntryKind::File
                                    }
                        })
                }
                OperationKind::Remove => source.is_none(),
                OperationKind::Trash => false,
            };
            if !valid {
                return Ok(uncertain(
                    operation,
                    "post-publication names do not identify the intended outcome".into(),
                    publication,
                ));
            }
            Ok(StepOutcome::Committed {
                source_after: source,
                destination_after: destination,
                recovery: None,
                warnings,
                publication,
            })
        }
        (Err(error), _) | (_, Err(error)) => {
            Ok(uncertain(operation, error.to_string(), publication))
        }
    }
}
