//! Requests to the per-endpoint actor, made from jobs (never from input or
//! render). Cloning a client or lease never connects; every accepted
//! request retains its own lease.
use crate::exec::ExecHost;
use crate::pool::{EndpointHandle, Job, Registry, Reply, Request, SessionState};
use crate::{Expect, RemoteError, RemoteFailureKind, RemoteStage, Stamp};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use strop_core::worker::CancelToken;
use strop_workspace::{RemoteEndpoint, RemoteFile, RemoteLocation};

const CANCEL_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct ConnectionLease {
    pub(crate) inner: Arc<EndpointHandle>,
}
impl ConnectionLease {
    pub fn endpoint(&self) -> &RemoteEndpoint {
        self.inner.endpoint()
    }
    pub fn is_connected(&self) -> bool {
        !self.inner.stop_signal().signalled() && self.inner.status() == SessionState::Connected
    }
}

pub struct RemoteSnapshot {
    pub file: RemoteFile,
    pub buffer: strop_core::Buffer,
    /// What the read observed; a save compares against it.
    pub stamp: Stamp,
    pub connection: ConnectionLease,
}
pub struct RemoteDirectorySnapshot {
    pub directory: RemoteFile,
    pub entries: Vec<RemoteEntry>,
    pub connection: ConnectionLease,
}
pub enum RemoteResource {
    File(Box<RemoteSnapshot>),
    Directory(RemoteDirectorySnapshot),
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteEntry {
    pub file: RemoteFile,
    pub kind: RemoteEntryKind,
    pub permissions: Option<RemotePermissions>,
    /// Server-reported bytes, never a recursively computed directory total.
    pub size: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RemoteEntryKind {
    File,
    Directory,
    SymbolicLink,
    Fifo,
    Socket,
    BlockDevice,
    CharacterDevice,
    Unknown,
}

impl RemoteEntryKind {
    pub const fn marker(self) -> char {
        match self {
            Self::File => '-',
            Self::Directory => 'd',
            Self::SymbolicLink => 'l',
            Self::Fifo => 'p',
            Self::Socket => 's',
            Self::BlockDevice => 'b',
            Self::CharacterDevice => 'c',
            Self::Unknown => '?',
        }
    }
}

/// Only the POSIX access/special bits; file kind is a separate typed value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct RemotePermissions(u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("permission bits exceed POSIX access and special bits")]
pub struct PermissionBitsError;

impl RemotePermissions {
    pub const fn new(bits: u16) -> Result<Self, PermissionBitsError> {
        if bits & !0o7777 != 0 {
            Err(PermissionBitsError)
        } else {
            Ok(Self(bits))
        }
    }
    pub(crate) const fn from_mode(mode: u32) -> Self {
        Self((mode & 0o7777) as u16)
    }
    pub const fn bits(self) -> u16 {
        self.0
    }
}
impl TryFrom<u16> for RemotePermissions {
    type Error = PermissionBitsError;
    fn try_from(bits: u16) -> Result<Self, Self::Error> {
        Self::new(bits)
    }
}
impl From<RemotePermissions> for u16 {
    fn from(value: RemotePermissions) -> Self {
        value.bits()
    }
}
impl std::fmt::Display for RemotePermissions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (read, write, execute, special, lower, upper) in [
            (0o400, 0o200, 0o100, 0o4000, 's', 'S'),
            (0o040, 0o020, 0o010, 0o2000, 's', 'S'),
            (0o004, 0o002, 0o001, 0o1000, 't', 'T'),
        ] {
            let r = if self.0 & read != 0 { 'r' } else { '-' };
            let w = if self.0 & write != 0 { 'w' } else { '-' };
            let x = match (self.0 & execute != 0, self.0 & special != 0) {
                (true, true) => lower,
                (false, true) => upper,
                (true, false) => 'x',
                (false, false) => '-',
            };
            write!(formatter, "{r}{w}{x}")?;
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct RemoteClient {
    registry: Arc<Registry>,
}
impl Default for RemoteClient {
    fn default() -> Self {
        Self::new()
    }
}
impl RemoteClient {
    /// Control sockets live in `$XDG_RUNTIME_DIR/strop-ssh` when a socket
    /// path there fits `sun_path`, else in a per-user directory under the
    /// system temporary directory (`/tmp` when that is long too); the
    /// directory is created private and refused when it is not.
    pub fn new() -> Self {
        let user = format!("strop-ssh-{}", rustix::process::geteuid().as_raw());
        let control = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|dir| !dir.is_empty())
            .map(|dir| std::path::PathBuf::from(dir).join("strop-ssh"))
            .into_iter()
            .chain([
                std::env::temp_dir().join(&user),
                std::path::Path::new("/tmp").join(&user),
            ])
            .find(|dir| crate::exec::fits_socket_path(dir))
            .unwrap_or_else(|| std::path::Path::new("/tmp").join(user));
        Self::with_control_dir(control)
    }

    pub fn with_control_dir(control: std::path::PathBuf) -> Self {
        Self {
            registry: Arc::new(Registry::new(control)),
        }
    }

    /// A whole text file of at most `max` bytes.
    pub fn read(
        &self,
        location: &RemoteLocation,
        max: u64,
        token: &CancelToken,
    ) -> Result<RemoteSnapshot, RemoteError> {
        match self.request(
            location.endpoint(),
            Request::Read {
                location: location.clone(),
                max,
            },
            token,
            false,
        )? {
            Reply::Snapshot(snapshot) => Ok(*snapshot),
            _ => Err(wrong_reply()),
        }
    }
    pub fn open(
        &self,
        location: &RemoteLocation,
        token: &CancelToken,
    ) -> Result<RemoteResource, RemoteError> {
        match self.request(
            location.endpoint(),
            Request::Open {
                location: location.clone(),
            },
            token,
            false,
        )? {
            Reply::Snapshot(snapshot) => Ok(RemoteResource::File(snapshot)),
            Reply::Directory(snapshot) => Ok(RemoteResource::Directory(snapshot)),
            _ => Err(wrong_reply()),
        }
    }
    pub fn list(
        &self,
        location: &RemoteLocation,
        token: &CancelToken,
    ) -> Result<RemoteDirectorySnapshot, RemoteError> {
        match self.request(
            location.endpoint(),
            Request::List {
                location: location.clone(),
            },
            token,
            false,
        )? {
            Reply::Directory(snapshot) => Ok(snapshot),
            _ => Err(wrong_reply()),
        }
    }
    /// Completion never creates an actor or authenticates. The actor checks the
    /// same policy again when dequeuing, since the observed connection may die.
    pub fn list_connected(
        &self,
        directory: &RemoteFile,
        token: &CancelToken,
    ) -> Result<Vec<RemoteEntry>, RemoteError> {
        match self.request(
            directory.endpoint(),
            Request::ListConnected {
                directory: directory.clone(),
            },
            token,
            true,
        )? {
            Reply::Entries(entries) => Ok(entries),
            _ => Err(wrong_reply()),
        }
    }
    pub fn connect(
        &self,
        endpoint: &RemoteEndpoint,
        token: &CancelToken,
    ) -> Result<ConnectionLease, RemoteError> {
        match self.request(endpoint, Request::Connect, token, false)? {
            Reply::Lease(lease) => Ok(lease),
            _ => Err(wrong_reply()),
        }
    }
    /// Save `bytes` to `file` under `expect`; the new stamp is what the
    /// next save compares against.
    pub fn save(
        &self,
        file: &RemoteFile,
        bytes: Vec<u8>,
        expect: Expect,
        token: &CancelToken,
    ) -> Result<Stamp, RemoteError> {
        match self.request(
            file.endpoint(),
            Request::Save {
                file: file.clone(),
                bytes,
                expect,
            },
            token,
            false,
        )? {
            Reply::Saved(stamp) => Ok(stamp),
            _ => Err(wrong_reply()),
        }
    }
    /// The endpoint's authenticated master for exec, connecting first
    /// when there is none: `rg`, `git`, language servers and terminals
    /// run as channels on it.
    pub fn host(
        &self,
        endpoint: &RemoteEndpoint,
        token: &CancelToken,
    ) -> Result<ExecHost, RemoteError> {
        let lease = self.connect(endpoint, token)?;
        lease.inner.host().ok_or_else(|| {
            RemoteError::bare(
                RemoteStage::Session,
                RemoteFailureKind::NotConnected,
                "the session has no ssh master to run commands on",
            )
            .remote(&endpoint.to_string())
        })
    }
    /// The endpoint's master when its session is connected right now: a
    /// registry lookup, no I/O, so it may run on the input path.
    pub fn connected_host(&self, endpoint: &RemoteEndpoint) -> Option<ExecHost> {
        self.registry
            .connected_handle(endpoint)
            .and_then(|handle| handle.host())
    }
    /// Stop the endpoint's session and its master, waiting for both.
    pub fn disconnect(&self, endpoint: &RemoteEndpoint) -> Result<(), RemoteError> {
        match self.registry.remove(endpoint) {
            Some(handle) => retire(&handle),
            None => Ok(()),
        }
    }
    pub fn disconnect_all(&self) -> Result<(), RemoteError> {
        let handles = self.registry.clear();
        let mut failure = None;
        for handle in handles {
            if let Err(error) = retire(&handle) {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
    /// Observational metadata, not a promise that the next request will succeed.
    pub fn connections(&self) -> Vec<RemoteEndpoint> {
        self.registry.connected()
    }

    fn request(
        &self,
        endpoint: &RemoteEndpoint,
        request: Request,
        cancel: &CancelToken,
        connected_only: bool,
    ) -> Result<Reply, RemoteError> {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let inner = if connected_only {
            self.registry.connected_handle(endpoint).ok_or_else(|| {
                RemoteError::bare(
                    RemoteStage::Session,
                    RemoteFailureKind::NotConnected,
                    "completion requires an existing authenticated connection",
                )
            })?
        } else {
            self.registry.acquire(endpoint)?
        };
        let (reply, receiver) = mpsc::channel();
        let job = Job {
            lease: ConnectionLease {
                inner: inner.clone(),
            },
            request,
            cancel: cancel.clone(),
            reply,
        };
        inner.jobs().try_send(job).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => RemoteError::bare(
                RemoteStage::Session,
                RemoteFailureKind::QueueFull,
                "remote session queue is full",
            ),
            mpsc::TrySendError::Disconnected(_) => stopped(),
        })?;
        loop {
            if cancel.is_cancelled() {
                return Err(cancelled());
            }
            if inner.stop_signal().signalled() {
                return Err(stopped());
            }
            match receiver.recv_timeout(CANCEL_POLL) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(stopped()),
            }
        }
    }
}
/// Wait for the actor's cleanup, then stop the master: channels still
/// on it (a terminal, a language server) end with it.
fn retire(handle: &EndpointHandle) -> Result<(), RemoteError> {
    let stopped = handle.wait_stopped();
    if let Some(host) = handle.host() {
        if host.is_connected() {
            host.disconnect().map_err(|error| {
                RemoteError::bare(
                    RemoteStage::Teardown,
                    RemoteFailureKind::Io,
                    error.to_string(),
                )
            })?;
        }
    }
    stopped
}
fn cancelled() -> RemoteError {
    RemoteError::bare(
        RemoteStage::Session,
        RemoteFailureKind::Cancelled,
        "remote request cancelled",
    )
}
fn stopped() -> RemoteError {
    RemoteError::bare(
        RemoteStage::Session,
        RemoteFailureKind::Stopped,
        "remote session stopped before delivering a result",
    )
}
fn wrong_reply() -> RemoteError {
    RemoteError::bare(
        RemoteStage::Session,
        RemoteFailureKind::Protocol,
        "session reply does not match the accepted request",
    )
}
