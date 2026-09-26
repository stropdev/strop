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
mod exec;
mod lifecycle;
mod payload;
mod session;

use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

use strop_core::worker::CancelToken;
use strop_worker_protocol::message::NotifyCoverage;
use strop_worker_protocol::{
    Capabilities, ClientMessage, Event, ProtocolError, Request, ResultOutcome, StreamChunk,
    StreamId, StreamRef, Subscription,
};
use strop_workspace::operation::{
    FsFailure, LocatedObservation, OperationIntent, OperationRefusal, PreparedOperation,
    StepReceipt, VerifiedOutcome,
};
use strop_workspace::{DirectorySnapshot, ResourceLocation};

pub use connection::{ExecEvent, StderrCapture, StreamEvent, Transport};
pub use error::ClientError;
pub use payload::ReadPayload;
pub use session::{ExecControl, ExecExit, ExecHandle, ExecStdin, PtySession};
pub use strop_worker_protocol::request::EnvironmentOverride;
pub use strop_worker_protocol::Refusal;

use connection::{Attachments, Conn, Reply};
use lifecycle::{Connector, Shared};
use payload::CancelGuard;
/// One per-lease nudge on an arriving stream chunk, shared across reconnects.
pub(crate) type StreamNotifier = Arc<dyn Fn(StreamId) + Send + Sync>;

/// Deletions that did occur, plus an optional typed partial-pass
/// failure. A lost reply is not a successful cache retirement claim.
pub struct CacheGcOutcome {
    pub report: strop_core::worker::cache_record::CacheGcReport,
    pub failure: Option<FsFailure>,
}

/// Outcome and its precise connection owner: streaming payload credits
/// must go to the incarnation that minted the stream, not a reconnect.
struct CallResult {
    outcome: ResultOutcome,
    attachments: Attachments,
    conn: Arc<Conn>,
    id: strop_worker_protocol::RequestId,
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
            shared: Arc::new(Shared::new(connector)),
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
        if let Some(notifier) = self.shared.stream_notifier.lock().as_ref() {
            conn.set_stream_notifier(Arc::clone(notifier));
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

    /// Install the stream-arrival hook (0058 WK12): the reader thread
    /// fires it after routing a chunk, so fd-polling consumers (the
    /// terminal service) wake on output rather than their poll timeout.
    /// One hook per lease — the terminal service owns it; a second
    /// install replaces the first. Re-installed on (re)connection.
    pub fn set_stream_notifier(&self, notifier: impl Fn(StreamId) + Send + Sync + 'static) {
        let notifier: StreamNotifier = Arc::new(notifier);
        *self.shared.stream_notifier.lock() = Some(Arc::clone(&notifier));
        if let Some(conn) = self.shared.slot.lock().as_ref() {
            if conn.alive() {
                conn.set_stream_notifier(notifier);
            }
        }
    }

    /// One admitted request/response round trip with cancellation
    /// propagation: a cancelled token sends the protocol `cancel` for
    /// exactly this request id. Streaming families register their payload
    /// sinks on the reader thread before the reply is delivered.
    fn call(&self, token: &CancelToken, body: Request) -> Result<ResultOutcome, ClientError> {
        Ok(self.call_inner(token, body, false)?.outcome)
    }

    fn call_streaming(
        &self,
        token: &CancelToken,
        body: Request,
    ) -> Result<CallResult, ClientError> {
        self.call_inner(token, body, true)
    }

    fn call_inner(
        &self,
        token: &CancelToken,
        body: Request,
        streaming: bool,
    ) -> Result<CallResult, ClientError> {
        if token.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        let conn = self.connection()?;
        self.call_inner_on(token, body, streaming, conn)
    }

    fn call_inner_on(
        &self,
        token: &CancelToken,
        body: Request,
        streaming: bool,
        conn: Arc<Conn>,
    ) -> Result<CallResult, ClientError> {
        if token.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        let id = conn.alloc_request();
        let (tx, rx) = channel();
        // The client's inbound budget mirrors the worker's admission
        // (WK11): overload fails typed here instead of queueing behind
        // bulk traffic on the wire.
        conn.admit(id, connection::Pending::new(tx, streaming, body.class()))?;
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
                Ok(CallResult {
                    outcome,
                    attachments,
                    conn,
                    id,
                })
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
        let CallResult {
            outcome,
            attachments,
            conn,
            id,
        } = self.call_streaming(
            token,
            Request::Read {
                location,
                offset,
                length,
            },
        )?;
        let cancel = CancelGuard(token.clone());
        match outcome {
            ResultOutcome::ReadOpened { stream, size } => {
                let Some((_, receiver)) = attachments
                    .streams
                    .into_iter()
                    .find(|(candidate, _)| *candidate == stream)
                else {
                    return Err(missing_stream("read payload"));
                };
                Ok(ReadPayload::streaming(
                    receiver, size, cancel, conn, stream, id,
                ))
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

    /// Read-only reconciliation of an old worker's uncertain effect.
    /// The worker compares the captured native namespace before calling
    /// the recovery verifier; old prepared write authority never revives.
    pub fn verify_recovered(
        &self,
        token: &CancelToken,
        attempt: StepReceipt,
        namespace: strop_worker_protocol::NamespaceIdentity,
    ) -> Result<VerifiedOutcome, ClientError> {
        match self.call(
            token,
            Request::VerifyRecovered {
                attempt: Box::new(attempt),
                namespace,
                binding: None,
            },
        )? {
            ResultOutcome::Verified { verified, .. } => Ok(verified),
            other => Err(unexpected(other)),
        }
    }

    /// Retire only this selected endpoint's unleased cache artifacts.
    /// Runs on the caller's job thread; a failed pass is a typed
    /// maintenance outcome, never authority to use SFTP for edits.
    pub fn collect_cache(
        &self,
        token: &CancelToken,
        context: &str,
    ) -> Result<CacheGcOutcome, ClientError> {
        match self.call(
            token,
            Request::CollectCache {
                context: context.to_owned(),
            },
        )? {
            ResultOutcome::CacheCollected { report, failure } => {
                Ok(CacheGcOutcome { report, failure })
            }
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
