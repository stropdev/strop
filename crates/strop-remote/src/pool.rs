//! The connection pool: one actor per endpoint, weak manager retention and
//! strong `ConnectionLease` handles. No child exists until a worker
//! acquires a session; the last owner signals stop and the actor itself
//! reaps. The actor owns its codec, runtime and process exclusively — no
//! concurrent writes to one connection are possible — and serializes every
//! job. Cancellation domains are separate: a queued job's cancellation only
//! skips that job, while an active one retires the physical connection and
//! its epoch, fails that request honestly, and never replays it silently.

mod lifecycle;
use crate::client::{ConnectionLease, RemoteDirectorySnapshot, RemoteEntry, RemoteSnapshot};
use crate::exec::ExecHost;
use crate::transport::{
    Expect, Fault, Mutation, RemoteAttrs, RemoteError, RemoteFailureKind, RemoteStage, Stamp,
};
pub(crate) use lifecycle::{SessionState, StatusCell, StopSignal};
use parking_lot::Mutex;
use parking_lot::RwLock;
use std::cell::Cell;
use std::collections::HashMap;
use std::future::Future;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Weak};
use std::time::Duration;
use strop_core::worker::CancelToken;
use strop_workspace::{RemoteEndpoint, RemoteFile, RemoteLocation};

#[cfg(unix)]
use crate::transport::{mutate, process::Physical, save, session};

/// Jobs one endpoint actor will hold before admission blocks a worker.
pub(crate) const QUEUE_CAPACITY: usize = 32;
/// The largest text file an open reads into a buffer.
pub const MAX_TEXT: u64 = 256 * 1024 * 1024;
/// How often an idle actor re-checks its stop signal.
const IDLE_POLL: Duration = Duration::from_millis(100);

/// What one admitted job asks the actor to do.
pub(crate) enum Request {
    /// A whole text file of at most `max` bytes.
    Read { location: RemoteLocation, max: u64 },
    /// Explicit open: routes by the target's attributes and returns the
    /// typed file or directory resource.
    Open { location: RemoteLocation },
    /// Explicit browsing action: may resolve a home query and connect.
    List { location: RemoteLocation },
    /// Completion path: must find a live authenticated session; it never
    /// connects or authenticates on its own.
    ListConnected { directory: RemoteFile },
    /// Explicit user connection: establish and hold a session.
    Connect,
    /// LSTAT one name.
    Observe { file: RemoteFile },
    /// One file-management operation; it never replaces a name.
    Mutate {
        endpoint: RemoteEndpoint,
        mutation: Mutation,
    },
    /// Save a whole file under `expect`.
    Save {
        file: RemoteFile,
        bytes: Vec<u8>,
        expect: Expect,
    },
}

impl Request {
    /// The remote identity diagnostics name for this job.
    fn remote(&self) -> String {
        match self {
            Self::Read { location, .. } | Self::Open { location, .. } | Self::List { location } => {
                location.to_string()
            }
            Self::ListConnected { directory } => directory.to_string(),
            Self::Save { file, .. } | Self::Observe { file } => file.to_string(),
            Self::Mutate { endpoint, .. } => endpoint.to_string(),
            Self::Connect => String::new(),
        }
    }
}

pub(crate) enum Reply {
    Snapshot(Box<RemoteSnapshot>),
    Directory(RemoteDirectorySnapshot),
    Entries(Vec<RemoteEntry>),
    Lease(ConnectionLease),
    Saved(Stamp),
    Observed(Option<RemoteAttrs>),
}

/// One admitted job. It owns a lease from admission until it fails or
/// transfers that lease into its reply, so shutdown can never sneak
/// between acquiring the endpoint and delivering the result.
pub(crate) struct Job {
    pub(crate) lease: ConnectionLease,
    pub(crate) request: Request,
    pub(crate) cancel: CancelToken,
    pub(crate) reply: std::sync::mpsc::Sender<Result<Reply, RemoteError>>,
}

/// The manager-facing handle: everything a worker needs to talk to one
/// endpoint's actor. The registry holds only weak references; leases hold
/// the strong ones.
pub(crate) struct EndpointHandle {
    endpoint: RemoteEndpoint,
    jobs: SyncSender<Job>,
    stop: Arc<StopSignal>,
    status: Arc<StatusCell>,
    /// The authenticated master, once the actor has one.
    host: Arc<Mutex<Option<ExecHost>>>,
}

impl EndpointHandle {
    pub(crate) fn endpoint(&self) -> &RemoteEndpoint {
        &self.endpoint
    }

    pub(crate) fn jobs(&self) -> &SyncSender<Job> {
        &self.jobs
    }

    pub(crate) fn stop_signal(&self) -> &Arc<StopSignal> {
        &self.stop
    }

    pub(crate) fn status(&self) -> SessionState {
        self.status.get()
    }

    pub(crate) fn host(&self) -> Option<ExecHost> {
        self.host.lock().clone()
    }

    pub(crate) fn wait_stopped(&self) -> Result<(), RemoteError> {
        self.stop.wait()
    }

    /// Signal the actor to stop. Fire and forget: the actor owns teardown.
    pub(crate) fn signal_stop(&self) {
        self.stop.signal();
    }
}

impl std::fmt::Debug for EndpointHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EndpointHandle")
            .field("endpoint", &self.endpoint)
            .field("status", &self.status.get())
            .finish()
    }
}

impl Drop for EndpointHandle {
    /// Last lease released: signal stop only. No lock, no I/O, no wait —
    /// the actor reaps its own child asynchronously.
    fn drop(&mut self) {
        self.stop.signal();
    }
}

/// Weak-retention registry of endpoint actors. Looked up and inserted on
/// workers; never on input or render.
pub(crate) struct Registry {
    slots: RwLock<HashMap<RemoteEndpoint, Weak<EndpointHandle>>>,
    /// The private directory holding every master's control socket.
    control: std::path::PathBuf,
    /// Every master this registry authenticated: they are stopped when
    /// the registry goes, so a clean exit leaves no background `ssh`.
    masters: Arc<Mutex<Vec<ExecHost>>>,
}

impl Drop for Registry {
    fn drop(&mut self) {
        for host in self.masters.lock().drain(..) {
            if host.is_connected() {
                let _ = host.disconnect();
            }
        }
    }
}

impl Registry {
    pub(crate) fn new(control: std::path::PathBuf) -> Self {
        Self {
            slots: RwLock::default(),
            control,
            masters: Arc::default(),
        }
    }

    fn upgrade(&self, endpoint: &RemoteEndpoint) -> Option<Arc<EndpointHandle>> {
        self.slots
            .read()
            .get(endpoint)
            .and_then(Weak::upgrade)
            .filter(|handle| !handle.stop.signalled())
    }

    pub(crate) fn connected_handle(
        &self,
        endpoint: &RemoteEndpoint,
    ) -> Option<Arc<EndpointHandle>> {
        self.upgrade(endpoint)
            .filter(|handle| handle.status() == SessionState::Connected)
    }

    /// Look up — or create exactly once — the actor for one endpoint.
    /// The writer mutex covers construction and publication only; no
    /// asynchronous admission or I/O happens under it.
    pub(crate) fn acquire(
        &self,
        endpoint: &RemoteEndpoint,
    ) -> Result<Arc<EndpointHandle>, RemoteError> {
        #[cfg(not(unix))]
        {
            let _ = endpoint;
            return Err(RemoteError::bare(
                RemoteStage::Session,
                RemoteFailureKind::Unsupported,
                "remote sessions require Unix process supervision",
            ));
        }
        #[cfg(unix)]
        {
            if let Some(handle) = self.upgrade(endpoint) {
                return Ok(handle);
            }
            let mut slots = self.slots.write();
            if let Some(handle) = slots
                .get(endpoint)
                .and_then(Weak::upgrade)
                .filter(|handle| !handle.stop.signalled())
            {
                return Ok(handle);
            }
            let handle = spawn_actor(
                endpoint.clone(),
                connector(endpoint, &self.control, Arc::clone(&self.masters)),
            )
            .map_err(|error| {
                RemoteError::bare(
                    RemoteStage::Session,
                    RemoteFailureKind::Io,
                    format!("starting the session actor: {error}"),
                )
            })?;
            // Prune dead weak entries while the writer lock is held.
            slots.retain(|_, weak| weak.strong_count() > 0);
            slots.insert(endpoint.clone(), Arc::downgrade(&handle));
            Ok(handle)
        }
    }

    /// Remove one endpoint's slot and signal its actor to stop. Later work
    /// re-creates a fresh actor (a new incarnation).
    pub(crate) fn remove(&self, endpoint: &RemoteEndpoint) -> Option<Arc<EndpointHandle>> {
        let handle = self
            .slots
            .write()
            .remove(endpoint)
            .and_then(|weak| weak.upgrade());
        if let Some(handle) = &handle {
            handle.signal_stop();
        }
        handle
    }

    /// Remove every slot and stop every actor.
    pub(crate) fn clear(&self) -> Vec<Arc<EndpointHandle>> {
        let handles: Vec<Arc<EndpointHandle>> = self
            .slots
            .write()
            .drain()
            .filter_map(|(_, weak)| weak.upgrade())
            .collect();
        for handle in &handles {
            handle.signal_stop();
        }
        handles
    }

    /// Endpoints with a live, connected actor. Observational only: the
    /// result can become stale immediately and is not a liveness
    /// guarantee. Returns owned metadata, never leases.
    pub(crate) fn connected(&self) -> Vec<RemoteEndpoint> {
        self.slots
            .read()
            .values()
            .filter_map(|weak| weak.upgrade())
            .filter(|handle| handle.status() == SessionState::Connected)
            .map(|handle| handle.endpoint().clone())
            .collect()
    }
}

/// How an actor reaches its endpoint: authenticate (or reuse) the
/// master, then the SFTP channel command on it. Tests substitute a local
/// `sftp-server` with no master.
pub(crate) type Connector = Box<
    dyn Fn(&CancelToken) -> Result<(Option<ExecHost>, std::process::Command), RemoteError> + Send,
>;

/// The production connector: the endpoint's master in `control`.
#[cfg(unix)]
fn connector(
    endpoint: &RemoteEndpoint,
    control: &std::path::Path,
    masters: Arc<Mutex<Vec<ExecHost>>>,
) -> Connector {
    let endpoint = endpoint.clone();
    let control = control.to_path_buf();
    Box::new(move |token| {
        let host = ExecHost::connect(&endpoint, &control, token).map_err(|error| {
            let kind = match &error {
                crate::exec::ExecError::Cancelled { .. } => RemoteFailureKind::Cancelled,
                crate::exec::ExecError::Spawn(_) => RemoteFailureKind::Spawn,
                _ => RemoteFailureKind::Connect,
            };
            let mut failure = RemoteError::bare(RemoteStage::Connect, kind, error.to_string())
                .remote(&endpoint.to_string());
            if kind == RemoteFailureKind::Connect {
                failure.refine_connect();
            }
            failure
        })?;
        let command = host.sftp_command();
        let mut known = masters.lock();
        if !known.contains(&host) {
            known.push(host.clone());
        }
        Ok((Some(host), command))
    })
}

/// Everything the actor thread owns. It never holds a strong handle to
/// itself, or it would keep its own session alive forever.
struct Actor {
    receiver: Receiver<Job>,
    stop: Arc<StopSignal>,
    status: Arc<StatusCell>,
    host: Arc<Mutex<Option<ExecHost>>>,
    connector: Connector,
}

/// Spawn the dedicated actor thread for one endpoint incarnation. The
/// thread is detached: its lifetime is governed by the stop signal and the
/// job channel, never by a joiner.
#[cfg(unix)]
fn spawn_actor(
    endpoint: RemoteEndpoint,
    connector: Connector,
) -> Result<Arc<EndpointHandle>, String> {
    let (jobs, receiver) = std::sync::mpsc::sync_channel(QUEUE_CAPACITY);
    let stop = Arc::new(StopSignal::new());
    let status = Arc::new(StatusCell::default());
    let host = Arc::new(Mutex::new(None));
    let handle = Arc::new(EndpointHandle {
        endpoint,
        jobs,
        stop: stop.clone(),
        status: status.clone(),
        host: host.clone(),
    });
    let actor = Actor {
        receiver,
        stop,
        status,
        host,
        connector,
    };
    std::thread::Builder::new()
        .name("strop-remote-session".to_owned())
        .spawn(move || actor_loop(actor))
        .map(drop)
        .map_err(|error| error.to_string())?;
    Ok(handle)
}

/// The actor state machine:
///
/// ```text
/// Disconnected --dequeue live job--> Connecting(epoch) --> Working
/// Working --success--> ConnectedIdle --stop--> teardown --> exit
/// Working --transport/protocol error or cancel--> teardown --> Disconnected
/// ```
#[cfg(unix)]
fn actor_loop(actor: Actor) {
    let Actor {
        receiver,
        stop,
        status,
        host,
        connector,
    } = actor;
    let _exit = lifecycle::ActorExit(stop.clone());
    let mut next_epoch: u64 = 0;
    let mut physical: Option<Physical> = None;
    loop {
        if stop.signalled() {
            break;
        }
        match receiver.recv_timeout(IDLE_POLL) {
            Ok(job) => {
                // Cancellation observed before dispatch skips the job; the
                // physical connection is never touched.
                if job.cancel.is_cancelled() {
                    refuse(job, cancelled_before_dispatch());
                    continue;
                }
                dispatch(
                    &stop,
                    &status,
                    &host,
                    &connector,
                    &mut next_epoch,
                    &mut physical,
                    job,
                );
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let outcome = physical.take().map_or(Ok(()), Physical::retire);
    status.set(SessionState::Disconnected);
    stop.complete(outcome);
}

#[cfg(unix)]
fn cancelled_before_dispatch() -> RemoteError {
    RemoteError::bare(
        RemoteStage::Session,
        RemoteFailureKind::Cancelled,
        "the job was cancelled before the session picked it up",
    )
}

#[cfg(unix)]
fn not_connected() -> RemoteError {
    RemoteError::bare(
        RemoteStage::Session,
        RemoteFailureKind::NotConnected,
        "no authenticated session exists for this endpoint, and this \
         operation never starts one",
    )
}

fn refuse(job: Job, error: RemoteError) {
    // A gone receiver simply drops the job (and its lease).
    let _ = job.reply.send(Err(error));
}

/// Process one job to completion. Connection policy: clean SFTP refusals
/// fail only this job; transport/protocol failures, cancellation, stop and
/// failed cleanup retire the physical connection. Nothing is retried
/// silently — a later job may establish a new epoch on its own behalf.
#[cfg(unix)]
fn dispatch(
    stop: &Arc<StopSignal>,
    status: &Arc<StatusCell>,
    host: &Mutex<Option<ExecHost>>,
    connector: &Connector,
    next_epoch: &mut u64,
    physical: &mut Option<Physical>,
    job: Job,
) {
    let remote = job.request.remote();
    // Completion-path jobs must find a live session: never connect here.
    if matches!(job.request, Request::ListConnected { .. }) && physical.is_none() {
        refuse(job, not_connected());
        return;
    }
    let mut spawned_here = false;
    if physical.is_none() {
        let Some(epoch) = next_epoch.checked_add(1) else {
            refuse(
                job,
                RemoteError::bare(
                    RemoteStage::Session,
                    RemoteFailureKind::Protocol,
                    "connection epoch exhausted",
                ),
            );
            return;
        };
        *next_epoch = epoch;
        status.set(SessionState::Connecting);
        let (master, mut command) = match connector(&job.cancel) {
            Ok(connected) => connected,
            Err(error) => {
                status.set(SessionState::Disconnected);
                refuse(job, error);
                return;
            }
        };
        *host.lock() = master;
        match Physical::connect(&mut command, &job.cancel, stop, *next_epoch, &remote) {
            Ok(connection) => {
                physical.replace(connection);
                spawned_here = true;
                status.set(SessionState::Connected);
            }
            Err(error) => {
                status.set(SessionState::Disconnected);
                // `Physical::connect` already consumed, terminated and
                // reaped the child; the spawning job's hook is spent.
                job.cancel.clear_cancel_resource();
                refuse(job, error);
                return;
            }
        }
    }
    let Some(connection) = physical.as_mut() else {
        refuse(
            job,
            RemoteError::bare(
                RemoteStage::Session,
                RemoteFailureKind::Protocol,
                "connected actor has no physical session",
            ),
        );
        return;
    };
    debug_assert_eq!(connection.epoch, *next_epoch);
    let outcome = execute(connection, &job, stop);
    if spawned_here {
        // The connection outlives this job: detach its kill hook so a
        // later cancellation of this job's owner cannot kill a connection
        // now serving others.
        job.cancel.clear_cancel_resource();
    }
    match outcome {
        Ok(reply) => {
            // Cancellation has priority over an uncommitted success; a
            // clean success leaves the connection sound.
            if job.cancel.is_cancelled() || stop.signalled() {
                let fault = if stop.signalled() && !job.cancel.is_cancelled() {
                    Fault::stopped(RemoteStage::Teardown)
                } else {
                    Fault::cancelled(RemoteStage::Teardown)
                };
                let fault = if let Some(connection) = physical.take() {
                    match connection.retire() {
                        Ok(()) => fault,
                        Err(error) => fault.with_cleanup(Fault::new(
                            RemoteStage::Teardown,
                            error.kind(),
                            error.to_string(),
                        )),
                    }
                } else {
                    fault
                };
                status.set(SessionState::Disconnected);
                refuse(job, RemoteError::fault(&remote, fault));
            } else {
                let _ = job.reply.send(Ok(reply));
            }
        }
        Err(mut fault) => {
            let diagnostics = physical.as_ref().and_then(Physical::diagnostics);
            let (kind, poison) = fault.disposition();
            if poison || !connection_survives(kind) {
                if let Some(connection) = physical.take() {
                    if let Err(error) = connection.retire() {
                        fault = fault.with_cleanup(Fault::new(
                            RemoteStage::Teardown,
                            error.kind(),
                            error.to_string(),
                        ));
                    }
                }
                status.set(SessionState::Disconnected);
            }
            refuse(job, RemoteError::fault(&remote, fault).stderr(diagnostics));
        }
    }
}

/// Clean SFTP refusals that leave the framing sound: the connection stays.
#[cfg(unix)]
fn connection_survives(kind: RemoteFailureKind) -> bool {
    matches!(
        kind,
        RemoteFailureKind::NotFound
            | RemoteFailureKind::Permission
            | RemoteFailureKind::NotRegularFile
            | RemoteFailureKind::NotDirectory
            | RemoteFailureKind::UnknownLength
            | RemoteFailureKind::TooLarge
            | RemoteFailureKind::InvalidUtf8
            | RemoteFailureKind::HomeUnsupported
            | RemoteFailureKind::TooManyEntries
            | RemoteFailureKind::Changed
            | RemoteFailureKind::Exists
            | RemoteFailureKind::Failure
    )
}

/// Run one job's protocol script on the actor's connection.
#[cfg(unix)]
fn execute(physical: &mut Physical, job: &Job, stop: &Arc<StopSignal>) -> Result<Reply, Fault> {
    use crate::client::RemoteEntryKind;

    let stage = Cell::new(RemoteStage::Connect);
    let Physical { runtime, codec, .. } = physical;
    let cancel = &job.cancel;
    let lease = job.lease.clone();
    let snapshot = |file, text: String, stamp| {
        Reply::Snapshot(Box::new(RemoteSnapshot {
            file,
            buffer: strop_core::Buffer::from_text(&text),
            stamp,
            connection: lease.clone(),
        }))
    };
    let directory = |directory, entries| {
        Reply::Directory(RemoteDirectorySnapshot {
            directory,
            entries,
            connection: lease.clone(),
        })
    };
    match &job.request {
        Request::Read { location, max } => {
            let file = block_on_guarded(
                runtime,
                session::resolve_location(codec, location),
                cancel,
                stop,
                &stage,
            )?;
            let (text, stamp) = block_on_guarded(
                runtime,
                session::read_text(codec, &file, *max, &stage),
                cancel,
                stop,
                &stage,
            )?;
            Ok(snapshot(file, text, stamp))
        }
        Request::Open { location } => {
            let file = block_on_guarded(
                runtime,
                session::resolve_location(codec, location),
                cancel,
                stop,
                &stage,
            )?;
            let kind = block_on_guarded(
                runtime,
                session::path_kind(codec, &file, &stage),
                cancel,
                stop,
                &stage,
            )?;
            match kind {
                RemoteEntryKind::Directory => {
                    let entries = block_on_guarded(
                        runtime,
                        session::list_entries(codec, &file, &stage),
                        cancel,
                        stop,
                        &stage,
                    )?;
                    Ok(directory(file, entries))
                }
                RemoteEntryKind::File => {
                    let (text, stamp) = block_on_guarded(
                        runtime,
                        session::read_text(codec, &file, MAX_TEXT, &stage),
                        cancel,
                        stop,
                        &stage,
                    )?;
                    Ok(snapshot(file, text, stamp))
                }
                _ => Err(Fault::new(
                    RemoteStage::Inspect,
                    RemoteFailureKind::NotRegularFile,
                    "the target is neither a regular file nor a directory",
                )),
            }
        }
        Request::List { location } => {
            let resolved = block_on_guarded(
                runtime,
                session::resolve_location(codec, location),
                cancel,
                stop,
                &stage,
            )?;
            let kind = block_on_guarded(
                runtime,
                session::path_kind(codec, &resolved, &stage),
                cancel,
                stop,
                &stage,
            )?;
            if kind != RemoteEntryKind::Directory {
                return Err(Fault::new(
                    RemoteStage::Open,
                    RemoteFailureKind::NotDirectory,
                    "listing requires a directory",
                ));
            }
            let entries = block_on_guarded(
                runtime,
                session::list_entries(codec, &resolved, &stage),
                cancel,
                stop,
                &stage,
            )?;
            Ok(directory(resolved, entries))
        }
        Request::ListConnected { directory } => {
            let entries = block_on_guarded(
                runtime,
                session::list_entries(codec, directory, &stage),
                cancel,
                stop,
                &stage,
            )?;
            Ok(Reply::Entries(entries))
        }
        Request::Save {
            file,
            bytes,
            expect,
        } => {
            stage.set(RemoteStage::Transfer);
            let stamp = block_on_guarded(
                runtime,
                save::save(codec, file.path(), bytes, *expect),
                cancel,
                stop,
                &stage,
            )?;
            Ok(Reply::Saved(stamp))
        }
        Request::Observe { file } => {
            stage.set(RemoteStage::Inspect);
            let attrs = block_on_guarded(
                runtime,
                mutate::observe(codec, file.path()),
                cancel,
                stop,
                &stage,
            )?;
            Ok(Reply::Observed(attrs))
        }
        Request::Mutate { mutation, .. } => {
            stage.set(RemoteStage::Transfer);
            let attrs = block_on_guarded(
                runtime,
                mutate::mutate(codec, mutation),
                cancel,
                stop,
                &stage,
            )?;
            Ok(Reply::Observed(attrs))
        }
        Request::Connect => Ok(Reply::Lease(lease)),
    }
}

/// Drive one script under the cancellation/stop/deadline race.
#[cfg(unix)]
fn block_on_guarded<T, F>(
    runtime: &tokio::runtime::Runtime,
    work: F,
    cancel: &CancelToken,
    stop: &Arc<StopSignal>,
    stage: &Cell<RemoteStage>,
) -> Result<T, Fault>
where
    F: Future<Output = Result<T, Fault>>,
{
    runtime.block_on(session::guarded(work, cancel, stop, stage))
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
