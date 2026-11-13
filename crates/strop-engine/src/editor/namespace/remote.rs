//! Reviewed file management on an SSH host: the batch orchestration of
//! `strop_fs::batch` (missing parents as visible steps, dependency order,
//! cancellation, the unconfirmed-outcome cascade) over SFTP primitives on
//! the endpoint's connection. Every step is checked again just before it
//! runs: the source must still be what the review observed and the
//! destination must still be free — nothing is ever replaced. There is
//! no Trash on a host (removal is permanent), no lock against other
//! programs, and no atomicity across steps.
use strop_core::worker::CancelToken;
use strop_fs::batch::StepKernel;
use strop_fs::Environment;
use strop_remote::{Mutation, RemoteAttrs, RemoteClient, RemoteError, RemoteFailureKind};
use strop_workspace::operation::{
    CopyVersion, FsFailure, FsFailureKind, LocatedObservation, OperationCapability,
    OperationIntent, OperationKind, PreparedOperation, StepOutcome, StepReceipt, VerifiedChange,
    VerifiedOutcome,
};
use strop_workspace::{
    EntryKind, FileTime, Filesystem, Observation, Permissions, RemoteEndpoint, RemoteFile,
    ResourceLocation,
};

/// The largest file a copy reads into memory.
const COPY_LIMIT: u64 = strop_remote::MAX_TEXT;

/// One host's step executor for one batch.
pub(crate) struct RemoteKernel<'a> {
    pub client: &'a RemoteClient,
    pub endpoint: RemoteEndpoint,
}

fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}

fn observation(attrs: &RemoteAttrs) -> Observation {
    let mut observed = Observation::unknown(super::entry_kind(attrs.kind));
    observed.size = attrs.size;
    observed.modified = attrs.mtime.map(|seconds| FileTime {
        seconds: i64::from(seconds),
        nanos: 0,
    });
    observed.permissions = attrs
        .permissions
        .map(|mode| Permissions::from_mode(mode & 0o7777));
    observed
}

/// Kind, size and modification time: what SFTP v3 can say about "the
/// same file as reviewed".
fn same(before: &Observation, now: &Observation) -> bool {
    before.kind == now.kind && before.size == now.size && before.modified == now.modified
}

/// A failure that never reached the host or was refused cleanly leaves
/// nothing changed; anything else may have happened on the host.
fn settled(error: &RemoteError) -> Option<FsFailureKind> {
    match error.kind() {
        RemoteFailureKind::Permission => Some(FsFailureKind::Permission),
        RemoteFailureKind::NotFound
        | RemoteFailureKind::Changed
        | RemoteFailureKind::Exists
        | RemoteFailureKind::Failure => Some(FsFailureKind::Conflict),
        RemoteFailureKind::NotDirectory
        | RemoteFailureKind::NotRegularFile
        | RemoteFailureKind::TooLarge
        | RemoteFailureKind::UnknownLength
        | RemoteFailureKind::InvalidUtf8
        | RemoteFailureKind::HomeUnsupported
        | RemoteFailureKind::TooManyEntries => Some(FsFailureKind::Unsupported),
        RemoteFailureKind::Spawn
        | RemoteFailureKind::Auth
        | RemoteFailureKind::Trust
        | RemoteFailureKind::Network
        | RemoteFailureKind::Subsystem
        | RemoteFailureKind::Connect
        | RemoteFailureKind::NotConnected
        | RemoteFailureKind::QueueFull => Some(FsFailureKind::Io),
        RemoteFailureKind::Protocol
        | RemoteFailureKind::Io
        | RemoteFailureKind::ShortRead
        | RemoteFailureKind::Cancelled
        | RemoteFailureKind::Stopped
        | RemoteFailureKind::Deadline => None,
    }
}

impl RemoteKernel<'_> {
    fn file(&self, location: &ResourceLocation) -> Result<RemoteFile, FsFailure> {
        if location.filesystem != Filesystem::Remote(self.endpoint.clone()) {
            return Err(failure(
                FsFailureKind::Unsupported,
                "one review covers one host",
            ));
        }
        RemoteFile::from_path(self.endpoint.clone(), location.path.clone())
            .map_err(|error| failure(FsFailureKind::InvalidPath, error.to_string()))
    }

    fn observe(
        &self,
        location: &ResourceLocation,
        token: &CancelToken,
    ) -> Result<LocatedObservation, FsFailure> {
        let file = self.file(location)?;
        let value = self
            .client
            .observe(&file, token)
            .map_err(|error| {
                failure(
                    settled(&error).unwrap_or(FsFailureKind::Io),
                    error.to_string(),
                )
            })?
            .as_ref()
            .map(observation);
        Ok(LocatedObservation {
            location: location.clone(),
            value,
        })
    }

    fn capability(&self) -> OperationCapability {
        OperationCapability {
            principal: None,
            incarnation: self.endpoint.to_string(),
            no_replace: true,
            trash: false,
            trash_root: None,
            metadata_policy: format!(
                "{}: no Trash — delete is permanent; create 0644/0755 subject to the host's \
                 umask; copy keeps permission bits, not owner or times",
                self.endpoint
            ),
            concurrency_policy: "each step is checked again just before it runs; no lock \
                                 against other programs on the host and no atomicity across steps"
                .into(),
        }
    }
}

impl StepKernel for RemoteKernel<'_> {
    fn namespace(&self) -> Filesystem {
        Filesystem::Remote(self.endpoint.clone())
    }

    fn prepare_one(
        &self,
        intent: &OperationIntent,
        allow_occupied: bool,
        _environment: &Environment,
        token: &CancelToken,
    ) -> Result<PreparedOperation, FsFailure> {
        match intent.kind {
            OperationKind::Trash => {
                return Err(failure(
                    FsFailureKind::Unsupported,
                    "an SSH host has no Trash; a delete there is permanent",
                ))
            }
            OperationKind::Restore => {
                return Err(failure(
                    FsFailureKind::Unsupported,
                    "an SSH host has no Trash to restore from",
                ))
            }
            OperationKind::Store => {
                return Err(failure(
                    FsFailureKind::Unsupported,
                    "remote files are saved with :w",
                ))
            }
            _ => {}
        }
        for location in intent.source.iter().chain(intent.destination.iter()) {
            if !location.path.is_absolute() {
                return Err(failure(
                    FsFailureKind::InvalidPath,
                    "remote paths must be absolute",
                ));
            }
        }
        let buffer_copy =
            intent.kind == OperationKind::Copy && intent.copy_version == CopyVersion::Buffer;
        let source = intent
            .source
            .as_ref()
            .map(|location| self.observe(location, token))
            .transpose()?;
        let destination = intent
            .destination
            .as_ref()
            .map(|location| self.observe(location, token))
            .transpose()?;
        let mut parents = Vec::new();
        if let Some(parent) = intent
            .destination
            .as_ref()
            .and_then(|location| location.path.parent().map(|path| (location, path)))
            .map(|(location, path)| ResourceLocation {
                filesystem: location.filesystem.clone(),
                path: path.to_path_buf(),
            })
        {
            let parent = self.observe(&parent, token)?;
            if parent
                .value
                .as_ref()
                .is_some_and(|value| value.kind != EntryKind::Directory)
            {
                return Err(failure(
                    FsFailureKind::InvalidPath,
                    "the destination's parent is not a directory",
                ));
            }
            parents.push(parent);
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
                match &before.value {
                    None if !buffer_copy => {
                        return Err(failure(FsFailureKind::Conflict, "source no longer exists"))
                    }
                    None => {}
                    Some(observed) => {
                        if !matches!(observed.kind, EntryKind::File | EntryKind::Directory) {
                            return Err(failure(
                                FsFailureKind::Unsupported,
                                "link and special-file operations are unsupported on a host",
                            ));
                        }
                        if intent.kind == OperationKind::Copy && observed.kind != EntryKind::File {
                            return Err(failure(
                                FsFailureKind::Unsupported,
                                "copy supports regular files; directories are not copied \
                                 recursively over SSH",
                            ));
                        }
                        if observed.kind == EntryKind::Directory
                            && destination.as_ref().is_some_and(|after| {
                                after.location.path.starts_with(&before.location.path)
                            })
                        {
                            return Err(failure(
                                FsFailureKind::InvalidPath,
                                "a directory cannot move into itself",
                            ));
                        }
                    }
                }
            }
        }
        if matches!(intent.kind, OperationKind::Rename | OperationKind::Copy)
            && destination.is_none()
        {
            return Err(failure(
                FsFailureKind::InvalidPath,
                "operation requires a destination",
            ));
        }
        if intent.kind == OperationKind::Remove && destination.is_some() {
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
        Ok(PreparedOperation {
            intent: intent.clone(),
            source,
            destination,
            parents,
            dependencies: Vec::new(),
            capability: self.capability(),
        })
    }

    fn execute_one(
        &self,
        operation: &PreparedOperation,
        contents: Option<&ropey::Rope>,
        _receipts: &[StepReceipt],
        token: &CancelToken,
    ) -> StepOutcome {
        self.execute_checked(operation, contents, token)
            .unwrap_or_else(StepOutcome::Refused)
    }

    fn verify_one(
        &self,
        receipt: &StepReceipt,
        token: &CancelToken,
    ) -> Result<VerifiedOutcome, FsFailure> {
        let operation = &receipt.operation;
        let now = |located: &Option<LocatedObservation>| {
            located
                .as_ref()
                .map(|located| self.observe(&located.location, token))
                .transpose()
                .map(|located| located.and_then(|located| located.value))
        };
        let source_after = now(&operation.source)?;
        let destination_after = now(&operation.destination)?;
        let committed = |source_after, destination_after| {
            VerifiedOutcome::Committed(Box::new(VerifiedChange {
                source_after,
                destination_after,
            }))
        };
        Ok(match operation.intent.kind {
            OperationKind::CreateFile | OperationKind::CreateDirectory | OperationKind::Copy => {
                match destination_after {
                    Some(after) => committed(source_after, Some(after)),
                    None => VerifiedOutcome::Unchanged,
                }
            }
            OperationKind::Rename => match (source_after, destination_after) {
                (None, Some(after)) => committed(None, Some(after)),
                (Some(_), None) => VerifiedOutcome::Unchanged,
                _ => VerifiedOutcome::Unknown {
                    detail: "both or neither of the rename's names exist on the host".into(),
                },
            },
            OperationKind::Remove => match source_after {
                None => committed(None, None),
                Some(_) => VerifiedOutcome::Unchanged,
            },
            _ => VerifiedOutcome::Unknown {
                detail: "this operation does not run on a host".into(),
            },
        })
    }
}

impl RemoteKernel<'_> {
    /// One step: `Err` is a refusal decided before the mutation.
    fn execute_checked(
        &self,
        operation: &PreparedOperation,
        contents: Option<&ropey::Rope>,
        token: &CancelToken,
    ) -> Result<StepOutcome, FsFailure> {
        let refuse = |kind, detail: &str| failure(kind, detail);
        let observe = |located: &LocatedObservation| self.observe(&located.location, token);
        if let Some(source) = &operation.source {
            let now = observe(source)?;
            match (&source.value, &now.value) {
                (Some(before), Some(now)) if same(before, now) => {}
                (None, _) => {}
                _ => {
                    return Err(refuse(
                        FsFailureKind::Conflict,
                        "the source changed on the host since the review",
                    ))
                }
            }
        }
        if let Some(destination) = &operation.destination {
            if observe(destination)?.value.is_some() {
                return Err(refuse(
                    FsFailureKind::Conflict,
                    "the destination is occupied; nothing is replaced",
                ));
            }
        }
        let path = |located: &Option<LocatedObservation>| {
            located
                .as_ref()
                .map(|located| located.location.path.clone())
                .ok_or_else(|| refuse(FsFailureKind::InvalidPath, "step names no resource"))
        };
        let intent = &operation.intent;
        let mutation = match intent.kind {
            OperationKind::CreateFile => Mutation::CreateFile {
                path: path(&operation.destination)?,
                bytes: Vec::new(),
            },
            OperationKind::CreateDirectory => Mutation::CreateDirectory {
                path: path(&operation.destination)?,
            },
            OperationKind::Rename => Mutation::Rename {
                from: path(&operation.source)?,
                to: path(&operation.destination)?,
            },
            OperationKind::Copy if intent.copy_version == CopyVersion::Buffer => {
                let text = contents.ok_or_else(|| {
                    refuse(
                        FsFailureKind::Protocol,
                        "a buffer copy carries no pinned contents",
                    )
                })?;
                let mut bytes = Vec::with_capacity(text.len_bytes());
                for chunk in text.chunks() {
                    bytes.extend_from_slice(chunk.as_bytes());
                }
                Mutation::CreateFile {
                    path: path(&operation.destination)?,
                    bytes,
                }
            }
            OperationKind::Copy => Mutation::Copy {
                from: path(&operation.source)?,
                to: path(&operation.destination)?,
                max: COPY_LIMIT,
            },
            OperationKind::Remove => Mutation::Remove {
                path: path(&operation.source)?,
                directory: operation
                    .source
                    .as_ref()
                    .and_then(|source| source.value.as_ref())
                    .is_some_and(|value| value.kind == EntryKind::Directory),
            },
            _ => {
                return Err(refuse(
                    FsFailureKind::Unsupported,
                    "this operation does not run on a host",
                ))
            }
        };
        if token.is_cancelled() {
            return Ok(StepOutcome::Cancelled {
                detail: "cancelled before this step started".into(),
            });
        }
        let vacates = matches!(intent.kind, OperationKind::Rename | OperationKind::Remove);
        // SSH_FX_FAILURE carries no reason; name the likely one.
        let reason = match &mutation {
            Mutation::Remove {
                directory: true, ..
            } => "the host refused: a directory must be empty to be removed",
            Mutation::Remove { .. } => "the host refused the removal",
            _ => "the host refused: the destination may already exist",
        };
        match self.client.mutate(&self.endpoint, mutation, token) {
            Ok(after) => Ok(StepOutcome::Committed {
                source_after: if vacates {
                    None
                } else {
                    operation
                        .source
                        .as_ref()
                        .and_then(|source| source.value.clone())
                },
                destination_after: after.as_ref().map(observation),
                recovery: None,
                warnings: Vec::new(),
                publication: None,
            }),
            Err(error) => Ok(match settled(&error) {
                Some(kind) if error.kind() == RemoteFailureKind::Failure => {
                    StepOutcome::Refused(failure(kind, format!("{reason} ({error})")))
                }
                Some(kind) => StepOutcome::Refused(failure(kind, error.to_string())),
                None => StepOutcome::Unconfirmed {
                    detail: error.to_string(),
                    observed_destination: None,
                    recovery: None,
                    publication: None,
                },
            }),
        }
    }
}
