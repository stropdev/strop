//! Namespace dispatch: the local filesystem runs in process
//! ([`LocalFs`]); SSH workspaces list and read over the endpoint's SFTP
//! session. Checked filesystem operations (rename, delete, copy) are
//! local only — an SSH host is edited file by file, and anything else
//! runs in its terminal.

use strop_core::worker::CancelToken;
use strop_fs::batch::PreparedBatch;
use strop_fs::Environment;
use strop_workspace::operation::{
    FsFailure, FsFailureKind, OperationIntent, StepOutcome, StepReceipt, VerifiedOutcome,
};
use strop_workspace::{EntryKind, Filesystem, ResourceLocation};

mod local;
pub(crate) use local::LocalFs;

fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}

const REMOTE_OPERATIONS: &str =
    "file operations on an SSH host are not supported; run them in its :terminal";

/// A listed directory plus the connection lease that produced it, when the
/// listing rode an owned remote connection. The lease is client state; the
/// kernel never holds transport handles.
pub(crate) struct Listed {
    pub directory: strop_fs::ListedDirectory,
    pub connection: Option<strop_remote::ConnectionLease>,
}

/// List one directory in whichever namespace owns it.
pub(crate) fn list(
    local: &LocalFs,
    location: &ResourceLocation,
    client: &strop_remote::RemoteClient,
    token: &CancelToken,
) -> Result<Listed, FsFailure> {
    if !location.path.is_absolute() {
        return Err(failure(
            FsFailureKind::InvalidPath,
            "directory scope must be absolute",
        ));
    }
    match &location.filesystem {
        Filesystem::Local => {
            let snapshot = local.list(token, location)?;
            Ok(Listed {
                directory: strop_fs::ListedDirectory { snapshot },
                connection: None,
            })
        }
        Filesystem::Remote(endpoint) => {
            let file =
                strop_workspace::RemoteFile::from_path(endpoint.clone(), location.path.clone())
                    .map_err(|error| failure(FsFailureKind::InvalidPath, error.to_string()))?;
            let listed = client.list(&file.into(), token).map_err(|error| {
                failure(
                    if error.is_cancellation() {
                        FsFailureKind::Cancelled
                    } else {
                        FsFailureKind::Io
                    },
                    error.to_string(),
                )
            })?;
            from_remote(listed)
        }
    }
}

/// Decode one owned remote listing into kernel-validated data. Children that
/// escaped their captured parent are a protocol failure, not a listing.
pub(crate) fn from_remote(
    listed: strop_remote::RemoteDirectorySnapshot,
) -> Result<Listed, FsFailure> {
    let location = ResourceLocation::remote(
        listed.directory.endpoint().clone(),
        listed.directory.path().to_path_buf(),
    );
    let mut entries = Vec::with_capacity(listed.entries.len());
    for entry in &listed.entries {
        if entry.file.endpoint() != listed.directory.endpoint()
            || entry.file.path().parent() != Some(listed.directory.path())
        {
            return Err(failure(
                FsFailureKind::Protocol,
                "remote directory child escaped its captured parent",
            ));
        }
        let name =
            entry.file.path().file_name().ok_or_else(|| {
                failure(FsFailureKind::Protocol, "remote child has no native name")
            })?;
        let kind = match entry.kind {
            strop_remote::RemoteEntryKind::File => EntryKind::File,
            strop_remote::RemoteEntryKind::Directory => EntryKind::Directory,
            strop_remote::RemoteEntryKind::SymbolicLink => EntryKind::SymbolicLink,
            strop_remote::RemoteEntryKind::Fifo => EntryKind::Fifo,
            strop_remote::RemoteEntryKind::Socket => EntryKind::Socket,
            strop_remote::RemoteEntryKind::BlockDevice => EntryKind::BlockDevice,
            strop_remote::RemoteEntryKind::CharacterDevice => EntryKind::CharacterDevice,
            strop_remote::RemoteEntryKind::Unknown => EntryKind::Unknown,
        };
        entries.push(strop_fs::ObservedEntry {
            name: name.to_owned(),
            kind,
            size: entry.size,
            permissions: entry.permissions.map(|permissions| {
                strop_workspace::Permissions::from_mode(u32::from(permissions.bits()))
            }),
        });
    }
    Ok(Listed {
        directory: strop_fs::assemble(location, entries)?,
        connection: Some(listed.connection),
    })
}

/// The one namespace a batch covers, or a refusal: one review never mixes
/// namespaces (cross-namespace moves are a separate transfer contract).
fn batch_namespace(locations: impl Iterator<Item = Filesystem>) -> Result<Filesystem, FsFailure> {
    let mut namespaces = locations;
    let Some(first) = namespaces.next() else {
        return Ok(Filesystem::Local);
    };
    if namespaces.any(|namespace| namespace != first) {
        return Err(failure(
            FsFailureKind::Unsupported,
            "one review covers one namespace; split cross-namespace operations",
        ));
    }
    Ok(first)
}

/// Prepare one reviewed batch. Only the local namespace has checked
/// operations.
pub(crate) fn prepare(
    local: &LocalFs,
    intents: &[OperationIntent],
    environment: &Environment,
    token: &CancelToken,
) -> Result<PreparedBatch, FsFailure> {
    let namespace = batch_namespace(intents.iter().filter_map(|intent| {
        intent
            .location()
            .map(|location| location.filesystem.clone())
    }))?;
    match namespace {
        Filesystem::Local => local.prepare(token, intents, environment),
        Filesystem::Remote(_) => Err(failure(FsFailureKind::Unsupported, REMOTE_OPERATIONS)),
    }
}

/// Execute one approved batch.
pub(crate) fn execute(
    local: &LocalFs,
    plan: &PreparedBatch,
    contents: &std::collections::HashMap<usize, ropey::Rope>,
    token: &CancelToken,
) -> Vec<StepReceipt> {
    let namespace = batch_namespace(plan.steps.iter().filter_map(|step| {
        step.intent
            .location()
            .map(|location| location.filesystem.clone())
    }));
    match namespace {
        Ok(Filesystem::Local) => local.execute(token, plan, contents),
        Ok(Filesystem::Remote(_)) => {
            refuse_all(plan, failure(FsFailureKind::Unsupported, REMOTE_OPERATIONS))
        }
        Err(error) => refuse_all(plan, error),
    }
}

fn refuse_all(plan: &PreparedBatch, error: FsFailure) -> Vec<StepReceipt> {
    plan.steps
        .iter()
        .enumerate()
        .map(|(step, operation)| StepReceipt {
            step,
            operation: operation.clone(),
            outcome: StepOutcome::Refused(error.clone()),
        })
        .collect()
}

/// Verify one uncertain step.
pub(crate) fn verify(
    local: &LocalFs,
    receipt: &StepReceipt,
    token: &CancelToken,
) -> Result<VerifiedOutcome, FsFailure> {
    match receipt
        .operation
        .intent
        .location()
        .map(|location| &location.filesystem)
    {
        Some(Filesystem::Local) => local.verify(token, receipt),
        Some(Filesystem::Remote(_)) => Err(failure(FsFailureKind::Unsupported, REMOTE_OPERATIONS)),
        None => Err(failure(
            FsFailureKind::InvalidPath,
            "operation has no resource",
        )),
    }
}
