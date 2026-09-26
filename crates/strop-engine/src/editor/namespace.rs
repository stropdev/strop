//! Client-side namespace dispatch above the worker protocol (0058
//! WK03/WK04/WK07/WK09). Local operations ride the worker client, and SSH
//! workspaces whose host admits a worker ride that endpoint's deployed
//! worker lease ([`super::remote::workers::RemoteWorkers`]) — the same
//! handlers, the same protocol, no in-process filesystem twin anywhere
//! in the engine. Hosts that do not admit a worker keep the read-only
//! SFTP path byte-identically; mutations there are the deploy state
//! machine's typed refusal, never an alternate write path. Browsing
//! never deploys: read arms use only an already-admitted live lease,
//! and [`RemoteWorkers::admit`] runs exclusively from mutation families.
//! Containers stay read-only by policy. Document saves (`:w`) are the
//! [`StoreDispatch`] path: the Store intent's protected save through the
//! same kernel the worker serves. Transport selection lives here — never
//! inside the kernel, which sees namespace identity and capabilities only
//! as data.
//!
//! The in-process strop-fs kernel remains directly usable by lower-level
//! tests (strop-fs's own suite); engine tests cross the real codec
//! through an in-process transport ([`local_worker`]).

use super::containers::BoundWorker;
use strop_core::worker::CancelToken;
use strop_fs::batch::PreparedBatch;
use strop_fs::Environment;
use strop_remote::worker_transport::RemoteWorker;
use strop_worker_client::{ClientError, Worker};
use strop_workspace::operation::{
    FsFailure, FsFailureKind, OperationIntent, PreparedOperation, StepOutcome, StepReceipt,
    VerifiedOutcome,
};
use strop_workspace::{EntryKind, Filesystem, ResourceLocation};

use super::remote::workers::RemoteWorkers;

fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}

/// This editor session's local worker lease (0058 WK04): one worker per
/// compatible context, shared by every local filesystem owner in the
/// session, retired when the session drops it.
///
/// Production spawns the matching installed executable
/// (`current_exe --worker-stdio`, never PATH). Engine tests cross the
/// same codec over pipes to the real in-process serve loop — readiness
/// is still the handshake, and no filesystem policy runs in the editor
/// process either way.
pub(crate) fn local_worker() -> Worker {
    #[cfg(not(any(test, feature = "test-support")))]
    {
        Worker::local()
    }
    // Engine tests and dependents' test-support builds cross the same
    // codec against the real in-process serve loop.
    #[cfg(any(test, feature = "test-support"))]
    {
        Worker::connect_with(|| {
            let (client_read, worker_write) = std::io::pipe()?;
            let (worker_read, client_write) = std::io::pipe()?;
            std::thread::spawn(move || {
                if let Err(error) = strop_worker::serve::run(worker_read, worker_write) {
                    eprintln!("strop-worker test serve failed: {error}");
                }
            });
            Ok(strop_worker_client::Transport {
                reader: Box::new(client_read),
                writer: Box::new(client_write),
                child: None,
                stderr: None,
            })
        })
    }
}

/// Client failures become domain failures at this boundary: cancellation
/// and the worker's own typed `FsFailure` pass through unchanged;
/// transport/admission failures map onto the closest honest kind. The
/// message keeps the exact cause — failures are visible in the status
/// line, never silent.
pub(crate) fn map_client(error: ClientError) -> FsFailure {
    let kind = match &error {
        ClientError::Cancelled => FsFailureKind::Cancelled,
        ClientError::Domain(failure) => return failure.clone(),
        ClientError::Refused(refusal) => match refusal {
            strop_worker_client::Refusal::Capability { .. } => FsFailureKind::Unsupported,
            strop_worker_client::Refusal::Limit { .. } => FsFailureKind::Incomplete,
            strop_worker_client::Refusal::Busy { .. } => FsFailureKind::Busy,
            strop_worker_client::Refusal::WrongIncarnation { .. }
            | strop_worker_client::Refusal::WrongLease
            | strop_worker_client::Refusal::UnknownHandle { .. }
            | strop_worker_client::Refusal::StaleSubscription { .. }
            | strop_worker_client::Refusal::NamespaceChanged { .. } => FsFailureKind::Conflict,
            strop_worker_client::Refusal::Retiring | strop_worker_client::Refusal::Closed => {
                FsFailureKind::Io
            }
        },
        ClientError::Protocol(_) => FsFailureKind::Protocol,
        ClientError::Spawn(_)
        | ClientError::Handshake(_)
        | ClientError::Mismatch { .. }
        | ClientError::WorkerLost(_)
        | ClientError::Closed => FsFailureKind::Io,
    };
    failure(kind, error.to_string())
}

/// A listed directory plus the connection lease that produced it, when the
/// listing rode an owned remote connection. The lease is client state; the
/// kernel never holds transport handles.
pub(crate) struct Listed {
    pub directory: strop_fs::ListedDirectory,
    pub connection: Option<strop_remote::ConnectionLease>,
}

/// List one directory in whichever namespace owns it.
pub(crate) fn list(
    worker: &Worker,
    remote: &RemoteWorkers,
    location: &ResourceLocation,
    client: &strop_remote::RemoteClient,
    container: Option<&strop_containers::ContainerIdentity>,
    container_worker: Option<&BoundWorker>,
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
            let snapshot = worker.list(token, location.clone()).map_err(map_client)?;
            Ok(Listed {
                directory: strop_fs::ListedDirectory { snapshot },
                connection: None,
            })
        }
        Filesystem::Remote(endpoint) => {
            // WK07: an admitted endpoint with a live lease lists through
            // its worker. A dead lease or an unadmitted host keeps the
            // read-only SFTP path — listing never spawns a connection
            // (completion's Tab-never-authenticates rule) and never
            // deploys.
            if let Some(worker) = remote
                .get(endpoint)
                .filter(|worker| worker.worker().session().is_some())
            {
                let snapshot = worker.list(token, location.clone()).map_err(map_client)?;
                return Ok(Listed {
                    directory: strop_fs::ListedDirectory { snapshot },
                    connection: None,
                });
            }
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
        Filesystem::Container(id) => {
            let identity = container
                .filter(|identity| identity.id == id.as_str())
                .ok_or_else(|| {
                    failure(
                        FsFailureKind::Unsupported,
                        "attach the container before browsing its filesystem",
                    )
                })?;
            if let Some(worker) = container_worker {
                if !worker.matches(identity) {
                    return Err(failure(
                        FsFailureKind::Conflict,
                        "container worker lease belongs to a stale incarnation",
                    ));
                }
                let snapshot = worker.list(token, location).map_err(map_client)?;
                return Ok(Listed {
                    directory: strop_fs::ListedDirectory { snapshot },
                    connection: None,
                });
            }
            let path = location.path.to_str().ok_or_else(|| {
                failure(
                    FsFailureKind::Unsupported,
                    "container backend requires a UTF-8 path",
                )
            })?;
            let engine = strop_containers::engine(token)
                .map_err(|error| failure(FsFailureKind::Io, error.to_string()))?;
            let reference = strop_containers::ContainerRef::of(identity)
                .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?;
            let entries = strop_containers::list_dir(&engine, &reference, path, token)
                .map_err(|error| failure(FsFailureKind::Io, error.to_string()))?;
            from_container(location.clone(), entries)
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
            size: entry.size.map(|size| size.get()),
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

/// Decode one container engine listing into kernel-validated data.
pub(crate) fn from_container(
    location: ResourceLocation,
    source: Vec<strop_containers::DirEntry>,
) -> Result<Listed, FsFailure> {
    if !matches!(location.filesystem, Filesystem::Container(_)) {
        return Err(failure(
            FsFailureKind::Protocol,
            "container listing has a different filesystem namespace",
        ));
    }
    let entries = source
        .into_iter()
        .map(|entry| strop_fs::ObservedEntry {
            name: entry.name.into(),
            kind: match entry.kind {
                strop_containers::DirEntryKind::File => EntryKind::File,
                strop_containers::DirEntryKind::Dir => EntryKind::Directory,
                strop_containers::DirEntryKind::Symlink => EntryKind::SymbolicLink,
                strop_containers::DirEntryKind::Other => EntryKind::Unknown,
            },
            size: entry.size,
            permissions: None,
        })
        .collect();
    Ok(Listed {
        directory: strop_fs::assemble(location, entries)?,
        connection: None,
    })
}

/// The admitted executor for one namespace: the local worker lease, or
/// the endpoint's deployed worker lease (WK07). Remote mutation families
/// admit the worker here — consent-gated on first use — and a refusal
/// is the deploy state machine's honest outcome, never a retry through
/// the legacy helper, SFTP, Python or a local path.
enum Dispatch {
    Local(Worker),
    Remote(RemoteWorker),
}

fn kernel(
    namespace: &Filesystem,
    worker: &Worker,
    remote: &RemoteWorkers,
    token: &CancelToken,
) -> Result<Dispatch, FsFailure> {
    match namespace {
        Filesystem::Local => Ok(Dispatch::Local(worker.clone())),
        Filesystem::Remote(endpoint) => {
            let worker = remote.admit(endpoint, "remote filesystem mutation", token)?;
            Ok(Dispatch::Remote(worker))
        }
        Filesystem::Container(_) => Err(failure(
            FsFailureKind::Unsupported,
            "container filesystem operations are read-only by policy",
        )),
    }
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

/// Prepare one reviewed batch in its owning namespace.
pub(crate) fn prepare(
    worker: &Worker,
    remote: &RemoteWorkers,
    intents: &[OperationIntent],
    environment: &Environment,
    token: &CancelToken,
) -> Result<PreparedBatch, FsFailure> {
    let namespace = batch_namespace(intents.iter().filter_map(|intent| {
        intent
            .location()
            .map(|location| location.filesystem.clone())
    }))?;
    match kernel(&namespace, worker, remote, token)? {
        Dispatch::Local(worker) => {
            // The whole batch — dependency ordering, parent synthesis,
            // per-intent refusals — runs worker-side through the same
            // strop-fs orchestration; the engine keeps no local twin. The
            // session's environment (trash roots) crosses with it.
            let environment = strop_worker_client::EnvironmentOverride {
                home: environment.home.clone(),
                data_home: environment.data_home.clone(),
            };
            let (steps, refused) = worker
                .prepare(token, intents.to_vec(), Some(environment))
                .map_err(map_client)?;
            Ok(PreparedBatch { steps, refused })
        }
        Dispatch::Remote(worker) => {
            // The remote worker captures its own environment; local
            // trash roots never cross namespaces (WK07).
            let (steps, refused) = worker
                .prepare(token, intents.to_vec())
                .map_err(map_client)?;
            Ok(PreparedBatch { steps, refused })
        }
    }
}

/// Execute one approved batch in its owning namespace.
pub(crate) fn execute(
    worker: &Worker,
    remote: &RemoteWorkers,
    plan: &PreparedBatch,
    contents: &std::collections::HashMap<usize, ropey::Rope>,
    token: &CancelToken,
) -> Vec<StepReceipt> {
    let namespace = batch_namespace(plan.steps.iter().filter_map(|step| {
        step.intent
            .location()
            .map(|location| location.filesystem.clone())
    }));
    let dispatch = namespace.and_then(|namespace| kernel(&namespace, worker, remote, token));
    let dispatch = match dispatch {
        Ok(dispatch) => dispatch,
        Err(error) => {
            return plan
                .steps
                .iter()
                .enumerate()
                .map(|(step, operation)| StepReceipt {
                    step,
                    operation: operation.clone(),
                    outcome: StepOutcome::Refused(error.clone()),
                })
                .collect();
        }
    };
    // The wire carries one frozen content stream per apply; the
    // common case (one buffer pasted to any number of
    // destinations) shares those bytes. Distinct contents in one
    // batch are refused typed, never silently truncated.
    let mut distinct: Vec<Vec<u8>> = Vec::new();
    for rope in contents.values() {
        let bytes: Vec<u8> = rope
            .chunks()
            .flat_map(|chunk| chunk.as_bytes())
            .copied()
            .collect();
        if !distinct.contains(&bytes) {
            distinct.push(bytes);
        }
    }
    if distinct.len() > 1 {
        return plan
            .steps
            .iter()
            .enumerate()
            .map(|(step, operation)| StepReceipt {
                step,
                operation: operation.clone(),
                outcome: StepOutcome::Refused(failure(
                    FsFailureKind::Unsupported,
                    "one frozen content stream per apply; split the review",
                )),
            })
            .collect();
    }
    let applied = match &dispatch {
        Dispatch::Local(worker) => worker.apply(
            token,
            plan.steps.clone(),
            distinct.first().map(Vec::as_slice),
        ),
        Dispatch::Remote(worker) => worker.apply(
            token,
            plan.steps.clone(),
            distinct.first().map(Vec::as_slice),
        ),
    };
    match applied {
        Ok(receipts) => receipts,
        Err(error) => {
            let failure = map_client(error);
            plan.steps
                .iter()
                .enumerate()
                .map(|(step, operation)| StepReceipt {
                    step,
                    operation: operation.clone(),
                    outcome: StepOutcome::Refused(failure.clone()),
                })
                .collect()
        }
    }
}

/// Verify one uncertain step in its owning namespace.
pub(crate) fn verify(
    worker: &Worker,
    remote: &RemoteWorkers,
    receipt: &StepReceipt,
    token: &CancelToken,
) -> Result<VerifiedOutcome, FsFailure> {
    let namespace = receipt
        .operation
        .intent
        .location()
        .map(|location| location.filesystem.clone())
        .ok_or_else(|| failure(FsFailureKind::InvalidPath, "operation has no resource"))?;
    match kernel(&namespace, worker, remote, token)? {
        Dispatch::Local(worker) => worker.verify(token, receipt.clone()).map_err(map_client),
        Dispatch::Remote(worker) => worker.verify(token, receipt.clone()).map_err(map_client),
    }
}

/// One admitted Store dispatch target (0058 WK09): the local worker lease
/// or the endpoint's deployed worker lease. Same protocol, same kernel —
/// transport selection lives here, never in the kernel.
pub(crate) enum StoreDispatch {
    Local(Worker),
    Remote(RemoteWorker),
}

/// The outcome of one protected document store: either admission refused
/// before any effect was possible, or exactly one step receipt —
/// committed, refused at effect time, cancelled, or unconfirmed with its
/// frozen evidence.
pub(crate) enum StoreOutcome {
    Refused(FsFailure),
    Receipt(Box<StepReceipt>),
}

impl StoreDispatch {
    /// Prepare one Store intent: the admitted step (its destination
    /// observation is the editor's evidence) or the typed refusal.
    pub(crate) fn prepare_store(
        &self,
        intent: OperationIntent,
        token: &CancelToken,
    ) -> Result<PreparedOperation, FsFailure> {
        let prepared = match self {
            // No environment override crosses for a store: trash roots are
            // not save policy, and a remote worker captures its own.
            Self::Local(worker) => worker.prepare(token, vec![intent], None),
            Self::Remote(worker) => worker.prepare(token, vec![intent]),
        };
        let (mut steps, refused) = prepared.map_err(map_client)?;
        if let Some(refusal) = refused.into_iter().next() {
            return Err(refusal.failure);
        }
        if steps.len() != 1 {
            return Err(failure(
                FsFailureKind::Protocol,
                "a document store prepares exactly one step",
            ));
        }
        Ok(steps.remove(0))
    }

    /// Apply one prepared Store step with its frozen content stream. A
    /// transport loss after launch is honest uncertainty with the frozen
    /// evidence retained — never a silent retry or a guessed outcome.
    pub(crate) fn apply_store(
        &self,
        operation: PreparedOperation,
        content: &[u8],
        token: &CancelToken,
    ) -> StoreOutcome {
        let applied = match self {
            Self::Local(worker) => worker.apply(token, vec![operation.clone()], Some(content)),
            Self::Remote(worker) => worker.apply(token, vec![operation.clone()], Some(content)),
        };
        match applied {
            Ok(receipts) if receipts.len() == 1 => {
                StoreOutcome::Receipt(Box::new(receipts.into_iter().next().unwrap()))
            }
            Ok(_) => StoreOutcome::Receipt(Box::new(StepReceipt {
                step: 0,
                operation,
                outcome: StepOutcome::Unconfirmed {
                    detail: "the worker answered a store with the wrong receipt count".into(),
                    observed_destination: None,
                    recovery: None,
                    publication: None,
                },
            })),
            // Admission refusal and the worker's apply-domain error are
            // pre-effect. Client-side cancellation can race after the
            // request crossed the publication boundary and belongs to
            // the unconfirmed path below.
            Err(error @ (ClientError::Domain(_) | ClientError::Refused(_))) => {
                StoreOutcome::Refused(map_client(error))
            }
            Err(error) => StoreOutcome::Receipt(Box::new(StepReceipt {
                step: 0,
                operation,
                outcome: StepOutcome::Unconfirmed {
                    detail: format!("store outcome unconfirmed: {}", map_client(error)),
                    observed_destination: None,
                    recovery: None,
                    publication: None,
                },
            })),
        }
    }

    /// Verify a frozen old-session Store after reconnect without
    /// reviving its prepared mutation capability. The worker proves
    /// the exact captured native namespace before observing/syncing.
    pub(crate) fn verify_store_recovered(
        &self,
        receipt: StepReceipt,
        namespace: strop_worker_protocol::NamespaceIdentity,
        token: &CancelToken,
    ) -> Result<VerifiedOutcome, FsFailure> {
        if namespace.identity == "unattested" {
            // macOS has no admitted boot+mount witness yet. The same
            // live worker can still verify its own capability, but a
            // restarted one fails the ordinary incarnation guard.
            return match self {
                Self::Local(worker) => worker.verify(token, receipt).map_err(map_client),
                Self::Remote(worker) => worker.verify(token, receipt).map_err(map_client),
            };
        }
        match self {
            Self::Local(worker) => worker
                .verify_recovered(token, receipt, namespace)
                .map_err(map_client),
            Self::Remote(worker) => worker
                .verify_recovered(token, receipt, namespace)
                .map_err(map_client),
        }
    }
}
