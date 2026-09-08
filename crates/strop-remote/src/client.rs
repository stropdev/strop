//! Worker-only requests to the per-endpoint SFTP actor. Cloning a client or
//! lease never connects; every accepted request retains its own lease.
use crate::address::{RemoteEndpoint, RemoteFile, RemoteLocation};
use crate::pool::{EndpointHandle, Job, Registry, Reply, Request, SessionState};
use crate::{ReadFailureKind, ReadSelection, ReadStage, RemoteReadError, RemoteWindow};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use strop_core::worker::CancelToken;

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
    pub window: RemoteWindow,
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
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RemoteEntryKind {
    File,
    Directory,
    Other,
}

#[derive(Clone, Default)]
pub struct RemoteClient {
    registry: Arc<Registry>,
}
impl RemoteClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(
        &self,
        location: &RemoteLocation,
        selection: ReadSelection,
        token: &CancelToken,
    ) -> Result<RemoteSnapshot, RemoteReadError> {
        match self.request(
            location.endpoint(),
            Request::Read {
                location: location.clone(),
                selection,
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
        selection: ReadSelection,
        token: &CancelToken,
    ) -> Result<RemoteResource, RemoteReadError> {
        match self.request(
            location.endpoint(),
            Request::Open {
                location: location.clone(),
                selection,
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
    ) -> Result<RemoteDirectorySnapshot, RemoteReadError> {
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
    ) -> Result<Vec<RemoteEntry>, RemoteReadError> {
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
    ) -> Result<ConnectionLease, RemoteReadError> {
        match self.request(endpoint, Request::Connect, token, false)? {
            Reply::Lease(lease) => Ok(lease),
            _ => Err(wrong_reply()),
        }
    }
    /// Worker-only: revoke admission, then wait for the actor's cleanup result.
    pub fn disconnect(&self, endpoint: &RemoteEndpoint) -> Result<(), RemoteReadError> {
        if let Some(handle) = self.registry.remove(endpoint) {
            handle.wait_stopped()?;
        }
        Ok(())
    }
    pub fn disconnect_all(&self) -> Result<(), RemoteReadError> {
        let handles = self.registry.clear();
        let mut failure = None;
        for handle in handles {
            if let Err(error) = handle.wait_stopped() {
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
    ) -> Result<Reply, RemoteReadError> {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let inner = if connected_only {
            self.registry.connected_handle(endpoint).ok_or_else(|| {
                RemoteReadError::bare(
                    ReadStage::Session,
                    ReadFailureKind::NotConnected,
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
            mpsc::TrySendError::Full(_) => RemoteReadError::bare(
                ReadStage::Session,
                ReadFailureKind::QueueFull,
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
fn cancelled() -> RemoteReadError {
    RemoteReadError::bare(
        ReadStage::Session,
        ReadFailureKind::Cancelled,
        "remote request cancelled",
    )
}
fn stopped() -> RemoteReadError {
    RemoteReadError::bare(
        ReadStage::Session,
        ReadFailureKind::Stopped,
        "remote session stopped before delivering a result",
    )
}
fn wrong_reply() -> RemoteReadError {
    RemoteReadError::bare(
        ReadStage::Session,
        ReadFailureKind::Protocol,
        "session reply does not match the accepted request",
    )
}
