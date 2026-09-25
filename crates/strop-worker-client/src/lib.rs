//! The editor-side client of the unified worker (0058 WK04).
//!
//! A [`Worker`] is a lease on one worker incarnation serving one admitted
//! namespace. Semantics:
//!
//! - **Spawn the matching executable.** [`Worker::local`] launches
//!   `current_exe --worker-stdio` — never a PATH-found `strop`, never a
//!   project checkout binary. An explicit artifact path
//!   ([`Worker::spawn_program`]) is the administrator-provisioned/test
//!   route and is validated by the same handshake.
//! - **Readiness is the handshake.** A successful spawn means nothing;
//!   the lease answers requests only after `welcome` lands, and a
//!   version/build/target mismatch is a typed failure, never a downgrade
//!   or a fallback to in-process filesystem code.
//! - **Shared across compatible contexts, bounded, retired.** Clones of
//!   one `Worker` share the single connection to the one worker process
//!   of their compatible context; when the last clone drops, the worker
//!   is shut down and reaped. There is no second worker per context and
//!   no cross-session sharing.
//! - **Cancellation propagates.** A cancelled [`CancelToken`] sends the
//!   protocol `cancel` for the in-flight request; a mid-stream cancel of
//!   a read payload ends the stream without its announced bytes, which
//!   the payload surfaces as an I/O error — never silent truncation.
//! - **Worker death is typed.** A killed worker fails every in-flight
//!   request with [`ClientError::WorkerLost`]. The next request spawns a
//!   fresh incarnation (fresh session authority); stale prepared
//!   authority fails closed at the new worker. There is NEVER a fallback
//!   to a second filesystem implementation.
//!
//! Stream ids are partitioned by allocator: the client mints odd ids
//! (upload content), the worker mints even ids (read/exec payloads).

mod connection;
mod error;
mod payload;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use parking_lot::Mutex;
use strop_core::worker::CancelToken;
use strop_worker_protocol::message::NotifyCoverage;
use strop_worker_protocol::{
    Capabilities, ClientMessage, Event, ExecId, ExitStatus, ProtocolError, Request, ResultOutcome,
    StreamChunk, StreamRef, Subscription,
};
use strop_workspace::operation::{
    LocatedObservation, OperationIntent, OperationRefusal, PreparedOperation, StepReceipt,
    VerifiedOutcome,
};
use strop_workspace::{DirectorySnapshot, ResourceLocation};

pub use connection::{StderrCapture, Transport};
pub use error::ClientError;
pub use payload::ReadPayload;
pub use strop_worker_protocol::request::EnvironmentOverride;
pub use strop_worker_protocol::Refusal;

use connection::{Attachments, Conn, Reply};

/// How a connection is (re)created. Process transports carry a child to
/// reap; factory transports (the in-process test seam and deployed
/// remote/container workers) cross the same codec over caller-provided
/// pipes. Each connector also pins the target triple the worker's
/// handshake must report: this build's own, or the admitted endpoint's
/// for a deployed worker (WK07/WK08).
enum Connector {
    /// `current_exe --worker-stdio`: the matching installed executable.
    Installed,
    /// An explicit artifact path (administrator-provisioned or test);
    /// the handshake validates its identity exactly as for `Installed`.
    Program(PathBuf),
    /// Caller-provided duplex plus the endpoint target the handshake
    /// must report (this build's triple for the in-process test seam).
    Deployed {
        expected_target: String,
        factory: Box<dyn Fn() -> std::io::Result<Transport> + Send + Sync>,
    },
}

impl Connector {
    fn expected_target(&self) -> String {
        match self {
            Self::Installed | Self::Program(_) => strop_worker_protocol::TARGET_TRIPLE.to_string(),
            Self::Deployed {
                expected_target, ..
            } => expected_target.clone(),
        }
    }

    fn connect(&self) -> Result<Transport, ClientError> {
        match self {
            Self::Installed => {
                let program = std::env::current_exe()
                    .map_err(|error| ClientError::Spawn(format!("locate current exe: {error}")))?;
                connection::spawn_worker(&program)
            }
            Self::Program(path) => connection::spawn_worker(path),
            Self::Deployed { factory, .. } => factory()
                .map_err(|error| ClientError::Spawn(format!("in-process transport: {error}"))),
        }
    }
}

struct Shared {
    connector: Connector,
    slot: Mutex<Option<Arc<Conn>>>,
    /// Serializes (re)connection so one death never spawns two workers.
    connecting: Mutex<()>,
    events: Mutex<Option<Sender<Event>>>,
}

/// A lease on one worker incarnation. Cheap to clone; clones share the
/// connection. See the crate docs for the lifecycle contract.
pub struct Worker {
    shared: Arc<Shared>,
}

impl Clone for Worker {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Worker {
    /// The lease for this process's own native namespace, served by the
    /// matching installed executable. Lazy: nothing spawns until the
    /// first request; a spawn failure is typed at that request.
    pub fn local() -> Self {
        Self::new(Connector::Installed)
    }

    /// A lease served by an explicit artifact path (the
    /// administrator-provisioned and test route). The handshake still
    /// validates version/target identity — an invalid override never
    /// silently chooses another file.
    pub fn spawn_program(program: impl Into<PathBuf>) -> Self {
        Self::new(Connector::Program(program.into()))
    }

    /// A lease over a caller-provided duplex whose worker runs this
    /// build's own target. Engine tests pass a socketpair whose far end
    /// runs the real `strop_worker::serve` loop in-process: same
    /// handlers, same codec, no child process.
    pub fn connect_with(
        factory: impl Fn() -> std::io::Result<Transport> + Send + Sync + 'static,
    ) -> Self {
        Self::connect_deployed(strop_worker_protocol::TARGET_TRIPLE, factory)
    }

    /// A lease over a caller-provided duplex to a *deployed* worker
    /// (0058 WK07/WK08): the far end was provisioned and verified by the
    /// deployment flow for its endpoint, so the handshake binds the
    /// worker's reported target to the admitted endpoint's triple —
    /// which may differ from this client's platform (a macOS editor
    /// driving a Linux container worker), never to a guessed local one.
    /// Version and protocol still bind to this exact release.
    pub fn connect_deployed(
        expected_target: impl Into<String>,
        factory: impl Fn() -> std::io::Result<Transport> + Send + Sync + 'static,
    ) -> Self {
        Self::new(Connector::Deployed {
            expected_target: expected_target.into(),
            factory: Box::new(factory),
        })
    }

    fn new(connector: Connector) -> Self {
        Self {
            shared: Arc::new(Shared {
                connector,
                slot: Mutex::new(None),
                connecting: Mutex::new(()),
                events: Mutex::new(None),
            }),
        }
    }

    /// The live connection, connecting or respawning as needed. A dead
    /// incarnation is replaced by a fresh one with fresh session
    /// authority; handles minted by the dead incarnation fail closed.
    fn connection(&self) -> Result<Arc<Conn>, ClientError> {
        if let Some(conn) = self.shared.slot.lock().as_ref() {
            if conn.alive() {
                return Ok(Arc::clone(conn));
            }
        }
        let _serialize = self.shared.connecting.lock();
        if let Some(conn) = self.shared.slot.lock().as_ref() {
            if conn.alive() {
                return Ok(Arc::clone(conn));
            }
        }
        let conn = Conn::connect(
            self.shared.connector.connect()?,
            &self.shared.connector.expected_target(),
        )?;
        if let Some(events) = self.shared.events.lock().as_ref() {
            conn.set_events(Some(events.clone()));
        }
        *self.shared.slot.lock() = Some(Arc::clone(&conn));
        Ok(conn)
    }

    /// The admitted capabilities, connecting if needed.
    pub fn capabilities(&self) -> Result<Capabilities, ClientError> {
        Ok(self.connection()?.capabilities())
    }

    /// The negotiated hard bounds (WK11 registers budgets against them).
    pub fn limits(&self) -> Result<strop_worker_protocol::Limits, ClientError> {
        Ok(self.connection()?.limits())
    }

    /// The worker's host/namespace identity as observed at handshake.
    pub fn namespace(&self) -> Result<strop_worker_protocol::NamespaceIdentity, ClientError> {
        Ok(self.connection()?.namespace())
    }

    /// The current session identity, when connected (tests/diagnostics).
    pub fn session(&self) -> Option<strop_worker_protocol::Session> {
        self.shared
            .slot
            .lock()
            .as_ref()
            .filter(|conn| conn.alive())
            .map(|conn| conn.session())
    }

    /// The spawned worker's PID, when the transport is a process
    /// (diagnostics and the kill smoke test).
    pub fn worker_pid(&self) -> Option<u32> {
        self.shared
            .slot
            .lock()
            .as_ref()
            .and_then(|conn| conn.child_pid())
    }

    /// Subscribe one event sink for notify/exec events on this lease;
    /// installed on every (re)connection too.
    pub fn set_event_sink(&self, sender: Sender<Event>) {
        *self.shared.events.lock() = Some(sender.clone());
        if let Some(conn) = self.shared.slot.lock().as_ref() {
            if conn.alive() {
                conn.set_events(Some(sender));
            }
        }
    }

    /// One admitted request/response round trip with cancellation
    /// propagation: a cancelled token sends the protocol `cancel` for
    /// exactly this request id. Streaming families register their payload
    /// sinks on the reader thread before the reply is delivered.
    fn call(&self, token: &CancelToken, body: Request) -> Result<ResultOutcome, ClientError> {
        Ok(self.call_inner(token, body, false)?.0)
    }

    fn call_streaming(
        &self,
        token: &CancelToken,
        body: Request,
    ) -> Result<(ResultOutcome, Attachments), ClientError> {
        self.call_inner(token, body, true)
    }

    fn call_inner(
        &self,
        token: &CancelToken,
        body: Request,
        streaming: bool,
    ) -> Result<(ResultOutcome, Attachments), ClientError> {
        if token.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        let conn = self.connection()?;
        let id = conn.alloc_request();
        let (tx, rx) = channel();
        if streaming {
            conn.register_streaming(id, tx);
        } else {
            conn.register_plain(id, tx);
        }
        let registration = token.register_cancel_resource({
            let conn = Arc::clone(&conn);
            move || {
                conn.write_quiet(&ClientMessage::Cancel {
                    session: conn.session(),
                    id,
                });
                Ok(())
            }
        });
        if let Err(failure) = registration {
            conn.retract(id);
            return Err(ClientError::Protocol(ProtocolError::Unexpected {
                message: failure.message,
            }));
        }
        let sent = conn.write(&ClientMessage::Request {
            session: conn.session(),
            id,
            body: Box::new(body),
        });
        if let Err(error) = sent {
            conn.retract(id);
            token.clear_cancel_resource();
            return Err(error);
        }
        let reply = rx.recv();
        match reply {
            Ok(Reply::Outcome {
                outcome,
                attachments,
            }) => {
                if !streaming {
                    token.clear_cancel_resource();
                }
                Ok((outcome, attachments))
            }
            Ok(Reply::Protocol(error)) => {
                token.clear_cancel_resource();
                Err(ClientError::Protocol(error))
            }
            Ok(Reply::Lost(reason)) => {
                token.clear_cancel_resource();
                Err(ClientError::WorkerLost(reason))
            }
            Err(_) => {
                token.clear_cancel_resource();
                Err(ClientError::WorkerLost(conn.death_detail()))
            }
        }
    }

    /// Bounded stat-class observation of exact resources.
    pub fn observe(
        &self,
        token: &CancelToken,
        locations: Vec<ResourceLocation>,
    ) -> Result<Vec<LocatedObservation>, ClientError> {
        match self.call(token, Request::Observe { locations })? {
            ResultOutcome::Observations { observations } => Ok(observations),
            other => Err(unexpected(other)),
        }
    }

    /// One bounded listing. The kernel's listings are complete snapshots;
    /// no cursor ever comes back.
    pub fn list(
        &self,
        token: &CancelToken,
        location: ResourceLocation,
    ) -> Result<DirectorySnapshot, ClientError> {
        match self.call(
            token,
            Request::List {
                location,
                cursor: None,
            },
        )? {
            ResultOutcome::Listing { snapshot, .. } => Ok(snapshot),
            other => Err(unexpected(other)),
        }
    }

    /// A complete or ranged read. The payload is a pull reader over the
    /// stream; a worker that ends it short of the announced size is an
    /// I/O error there, never silent truncation.
    pub fn read(
        &self,
        token: &CancelToken,
        location: ResourceLocation,
        offset: u64,
        length: Option<u64>,
    ) -> Result<ReadPayload, ClientError> {
        let (outcome, attachments) = self.call_streaming(
            token,
            Request::Read {
                location,
                offset,
                length,
            },
        )?;
        match outcome {
            ResultOutcome::ReadOpened { size, .. } => {
                let Some((_, receiver)) = attachments.streams.into_iter().next() else {
                    return Err(missing_stream("read payload"));
                };
                // The cancel hook stays registered while the payload
                // streams; the payload clears it at completion or drop,
                // so a mid-stream cancel still propagates.
                Ok(ReadPayload::streaming(receiver, size, token.clone()))
            }
            other => Err(unexpected(other)),
        }
    }

    /// Prepare one batch in the worker's namespace: exact observations
    /// and the admitted capability, no effects yet.
    pub fn prepare(
        &self,
        token: &CancelToken,
        intents: Vec<OperationIntent>,
        environment: Option<strop_worker_protocol::request::EnvironmentOverride>,
    ) -> Result<(Vec<PreparedOperation>, Vec<OperationRefusal>), ClientError> {
        match self.call(
            token,
            Request::Prepare {
                intents,
                binding: None,
                environment,
            },
        )? {
            ResultOutcome::Prepared { steps, refused } => Ok((steps, refused)),
            other => Err(unexpected(other)),
        }
    }

    /// Apply prepared steps with at most one frozen content stream. The
    /// digest/length are verified against exactly the bytes uploaded
    /// before the request.
    pub fn apply(
        &self,
        token: &CancelToken,
        steps: Vec<PreparedOperation>,
        content: Option<&[u8]>,
    ) -> Result<Vec<StepReceipt>, ClientError> {
        let reference = match content {
            Some(bytes) => {
                let conn = self.connection()?;
                Some(upload(&conn, bytes)?)
            }
            None => None,
        };
        match self.call(
            token,
            Request::Apply {
                steps,
                content: reference,
                binding: None,
            },
        )? {
            ResultOutcome::Applied { receipts, .. } => Ok(receipts),
            other => Err(unexpected(other)),
        }
    }

    /// Verify one uncertain completed attempt against fresh evidence.
    pub fn verify(
        &self,
        token: &CancelToken,
        attempt: StepReceipt,
    ) -> Result<VerifiedOutcome, ClientError> {
        match self.call(
            token,
            Request::Verify {
                attempt: Box::new(attempt),
                binding: None,
            },
        )? {
            ResultOutcome::Verified { verified, .. } => Ok(verified),
            other => Err(unexpected(other)),
        }
    }

    /// Lease health: liveness without side effects.
    pub fn health(&self, token: &CancelToken) -> Result<(), ClientError> {
        match self.call(token, Request::Health)? {
            ResultOutcome::Healthy => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Install one notify subscription over a scope. Events arrive on the
    /// sink registered with [`Worker::set_event_sink`].
    pub fn subscribe(
        &self,
        token: &CancelToken,
        scope: ResourceLocation,
        recursive: bool,
    ) -> Result<(Subscription, NotifyCoverage), ClientError> {
        match self.call(token, Request::Subscribe { scope, recursive })? {
            ResultOutcome::Subscribed {
                subscription,
                coverage,
            } => Ok((subscription, coverage)),
            other => Err(unexpected(other)),
        }
    }

    /// Retire one subscription by its full identity.
    pub fn unsubscribe(
        &self,
        token: &CancelToken,
        subscription: Subscription,
    ) -> Result<(), ClientError> {
        match self.call(token, Request::Unsubscribe { subscription })? {
            ResultOutcome::Done => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Spawn one admitted finite command or leased service. The exit
    /// arrives on the handle's receiver even if the process settles
    /// before this call returns — the reader thread registers the waiter
    /// before delivering `ExecStarted`.
    pub fn exec(
        &self,
        token: &CancelToken,
        spec: strop_worker_protocol::ExecSpec,
    ) -> Result<ExecHandle, ClientError> {
        let conn = self.connection()?;
        let (outcome, attachments) = self.call_streaming(token, Request::Exec { spec })?;
        let (exec, stdin, stdout, stderr) = match outcome {
            ResultOutcome::ExecStarted {
                exec,
                stdin,
                stdout,
                stderr,
            } => (exec, stdin, stdout, stderr),
            other => return Err(unexpected(other)),
        };
        let mut streams: HashMap<_, _> = attachments.streams.into_iter().collect();
        let stdout = streams
            .remove(&stdout)
            .ok_or_else(|| missing_stream("exec stdout"))?;
        let stderr = streams
            .remove(&stderr)
            .ok_or_else(|| missing_stream("exec stderr"))?;
        let Some((_, exit)) = attachments.exits.into_iter().next() else {
            return Err(ClientError::Protocol(ProtocolError::Stream {
                message: "exec exit waiter was not attached".into(),
            }));
        };
        Ok(ExecHandle {
            id: exec,
            conn,
            stdin,
            stdin_sequence: 0,
            stdout: ReadPayload::new(stdout, None),
            stderr: ReadPayload::new(stderr, None),
            exit,
        })
    }

    /// Revoke one exec's lease (TERM/grace/KILL, then the exit event).
    pub fn exec_cancel(&self, token: &CancelToken, exec: ExecId) -> Result<(), ClientError> {
        match self.call(token, Request::ExecCancel { exec })? {
            ResultOutcome::Done => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Half-close one exec's stdin: EOF, not revocation.
    pub fn exec_half_close(&self, token: &CancelToken, exec: ExecId) -> Result<(), ClientError> {
        match self.call(token, Request::ExecHalfClose { exec })? {
            ResultOutcome::Done => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Authorized orderly shutdown: quiesce, drain, `bye`. The lease is
    /// dead afterwards; the next request spawns a fresh incarnation.
    pub fn shutdown(&self) -> Result<(), ClientError> {
        if let Some(conn) = self.shared.slot.lock().as_ref() {
            if conn.alive() {
                conn.retire();
            }
        }
        *self.shared.slot.lock() = None;
        Ok(())
    }
}

impl Drop for Shared {
    /// The last owner closed: the worker is retired (shutdown handshake,
    /// bounded wait, reap). Never a daemon left behind.
    fn drop(&mut self) {
        if let Some(conn) = self.slot.lock().take() {
            conn.retire();
        }
    }
}

/// Push one upload stream (client-minted odd id), digest-declared. The
/// chunks land on the wire before the `Apply` envelope that names them.
fn upload(conn: &Arc<Conn>, bytes: &[u8]) -> Result<StreamRef, ClientError> {
    use sha2::Digest;
    let stream = conn.alloc_stream();
    let mut sequence = 0_u64;
    let mut chunks = bytes
        .chunks(strop_worker_protocol::MAX_CHUNK_BYTES)
        .peekable();
    while let Some(slice) = chunks.next() {
        conn.write_chunk(&StreamChunk {
            stream,
            sequence,
            last: chunks.peek().is_none(),
            bytes: slice.to_vec(),
        })?;
        sequence += 1;
    }
    if bytes.is_empty() {
        conn.write_chunk(&StreamChunk {
            stream,
            sequence: 0,
            last: true,
            bytes: Vec::new(),
        })?;
    }
    Ok(StreamRef {
        stream,
        bytes: bytes.len() as u64,
        digest: sha2::Sha256::digest(bytes).into(),
    })
}

fn unexpected(outcome: ResultOutcome) -> ClientError {
    match outcome {
        ResultOutcome::Refused { refusal } => ClientError::Refused(refusal),
        ResultOutcome::Failed { failure } => ClientError::Domain(failure),
        other => ClientError::Protocol(ProtocolError::Unexpected {
            message: format!("unexpected outcome for the request family: {other:?}"),
        }),
    }
}

fn missing_stream(name: &'static str) -> ClientError {
    ClientError::Protocol(ProtocolError::Stream {
        message: format!("{name} stream was not attached"),
    })
}

/// One admitted exec's client-side handle. Dropping the handle never
/// kills the process; cancellation is explicit through
/// [`Worker::exec_cancel`], and the exit event arrives regardless.
pub struct ExecHandle {
    id: ExecId,
    conn: Arc<Conn>,
    stdin: Option<strop_worker_protocol::StreamId>,
    stdin_sequence: u64,
    stdout: ReadPayload,
    stderr: ReadPayload,
    exit: Receiver<connection::ExitEvent>,
}

impl ExecHandle {
    pub fn id(&self) -> ExecId {
        self.id
    }

    /// The child's stdout payload (ordered chunks, `last`-terminated).
    pub fn stdout(&mut self) -> &mut ReadPayload {
        &mut self.stdout
    }

    /// The child's stderr payload.
    pub fn stderr(&mut self) -> &mut ReadPayload {
        &mut self.stderr
    }

    /// Write to a relayed stdin stream; `last` delivers EOF (the same
    /// half-close semantics as [`Worker::exec_half_close`]).
    pub fn write_stdin(&mut self, bytes: &[u8], last: bool) -> Result<(), ClientError> {
        let Some(stream) = self.stdin else {
            return Err(ClientError::Refused(
                strop_worker_protocol::Refusal::UnknownHandle {
                    message: "exec has no relayed stdin".into(),
                },
            ));
        };
        let sequence = self.stdin_sequence;
        self.stdin_sequence += 1;
        self.conn.write_chunk(&StreamChunk {
            stream,
            sequence,
            last,
            bytes: bytes.to_vec(),
        })
    }

    /// Wait for the terminal status. `Lost` means the supervisor could
    /// not attest the exit — it is never silently mapped to a code.
    pub fn wait_exit(self) -> ExitStatus {
        match self.exit.recv() {
            Ok(event) => event.0,
            Err(_) => ExitStatus::Lost,
        }
    }
}
