//! The worker's serving loop (0058 WK04): one client session over one
//! bounded transport. Frames arrive from `reader` (stdin in
//! `strop --worker-stdio`, a socketpair in tests), admitted requests are
//! dispatched to the strop-fs kernel, the exec supervisor and the notify
//! backend, and outcomes leave on `writer` (stdout in production —
//! protocol bytes only). Diagnostics are private and bounded; they are
//! never protocol authority.
//!
//! ## Threading
//!
//! ```text
//! main thread      blocking frame read → admission → dispatch
//! request threads  one per admitted request (bounded by max_pending_requests)
//! notify pump      25 ms poll tick, events through the shared writer
//! exec pumps       per-exec stdout/stderr/stdin/stream supervisors
//! ```
//!
//! Every outbound frame crosses one writer mutex, so chunks of different
//! streams interleave freely but never tear. Cancellation arrives as the
//! protocol `cancel` (a per-request [`CancelToken`] pair) or as lease
//! teardown (EOF/shutdown revokes every admitted exec before exit).
//!
//! Stream ids are session-scoped and partitioned by allocator: the client
//! mints odd ids (upload content streams), the worker mints even ids
//! (read payloads, exec pipes). Neither side ever guesses the other's.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use strop_core::worker::{CancelHandle, CancelReason, CancelToken};
use strop_fs::Environment;
use strop_fs::ExecutionContext;
use strop_worker_protocol::codec::{self, Incoming, StreamChunk};
use strop_worker_protocol::frame::{self, FrameDecoder};
use strop_worker_protocol::{
    Authority, Capabilities, ClientMessage, EndpointInfo, Limits, NamespaceIdentity,
    NotifyCoverage, ProtocolError, Refusal, Request, RequestId, ResultOutcome, Session,
    ShutdownReason, StreamId, WorkerMessage, PROTOCOL_VERSION,
};
use strop_workspace::operation::{FsFailure, FsFailureKind};
use strop_workspace::ResourceLocation;

mod fs;
mod process;

pub(crate) fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}

pub(crate) fn io_failure(error: io::Error) -> FsFailure {
    let kind = match error.kind() {
        io::ErrorKind::PermissionDenied => FsFailureKind::Permission,
        io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound => FsFailureKind::Conflict,
        io::ErrorKind::InvalidInput => FsFailureKind::InvalidPath,
        _ => FsFailureKind::Io,
    };
    failure(kind, error.to_string())
}

/// The serving loop's own fatal failures. Everything else — admission,
/// domain faults, protocol violations — is reported in-band and typed.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("worker transport failed: {0}")]
    Io(#[from] io::Error),
    #[error("session randomness unavailable: {0}")]
    Random(String),
}

/// Diagnostics are private and bounded: at most this many lines, each
/// truncated. They never carry protocol authority (docs/worker-protocol.md
/// §7), so a chatty peer or fault cannot flood the parent's stderr.
const DIAG_LINE_LIMIT: usize = 256;
const DIAG_TEXT_LIMIT: usize = 1024;

/// Grace for admitted work to drain after an authorized shutdown.
const DRAIN_WAIT: Duration = Duration::from_secs(2);
/// Notify pump cadence: subscription commands and hint batches are both
/// advisory, so a short poll tick bounds their latency without a
/// self-pipe. Requests never wait on it (they ride the blocking reader).
const NOTIFY_TICK: Duration = Duration::from_millis(25);

/// One uploaded content stream still accumulating (client → worker), or a
/// relayed exec stdin sink.
enum Inbound {
    Content {
        bytes: Vec<u8>,
        next_sequence: u64,
        closed: bool,
    },
    ExecStdin {
        sender: std::sync::mpsc::SyncSender<StreamChunk>,
        next_sequence: u64,
    },
}

/// Bounded private diagnostics sink: line-counted and truncated, always
/// to the process's own stderr.
struct Diag {
    lines: usize,
}

impl Diag {
    fn note(&mut self, message: impl std::fmt::Display) {
        if self.lines >= DIAG_LINE_LIMIT {
            return;
        }
        self.lines += 1;
        let mut text = message.to_string();
        text.truncate(DIAG_TEXT_LIMIT);
        eprintln!("strop-worker: {text}");
    }
}

/// One client session's shared state. The `Authority` itself is mutated
/// only under its mutex (quiesce from a request thread, close at
/// teardown); every other table keys on the admitted session, so a
/// rejected frame never reaches a handle.
pub(crate) struct SessionState<W> {
    authority: Mutex<Authority>,
    writer: Mutex<W>,
    diagnostics: Mutex<Diag>,
    context: ExecutionContext,
    environment: Environment,
    limits: Limits,
    requests: Mutex<HashMap<RequestId, CancelHandle>>,
    inbound: Mutex<HashMap<StreamId, Inbound>>,
    execs: Mutex<HashMap<strop_worker_protocol::ExecId, process::ExecEntry>>,
    /// Even stream ids; the client owns odd ones.
    next_stream: AtomicU64,
    next_exec: AtomicU64,
    stop: AtomicBool,
    #[cfg(target_os = "linux")]
    notify: Arc<Mutex<crate::notify::NotifyManager>>,
}

impl<W: Write> SessionState<W> {
    pub(crate) fn send(&self, message: &WorkerMessage) {
        if let Err(error) = codec::write_envelope(&mut *self.writer.lock(), message) {
            self.stop.store(true, Ordering::Release);
            self.diagnostics
                .lock()
                .note(format_args!("write failed: {error}"));
        }
    }

    pub(crate) fn send_chunk(&self, chunk: &StreamChunk) {
        if let Err(error) = codec::write_chunk(&mut *self.writer.lock(), chunk) {
            self.stop.store(true, Ordering::Release);
            self.diagnostics
                .lock()
                .note(format_args!("write failed: {error}"));
        }
    }

    fn send_error(&self, id: Option<RequestId>, error: ProtocolError) {
        self.send(&WorkerMessage::Error { id, error });
    }

    fn note(&self, message: impl std::fmt::Display) {
        self.diagnostics.lock().note(message);
    }

    /// The next worker-minted stream id (even; the client owns odd ids).
    pub(crate) fn mint_stream(&self) -> StreamId {
        StreamId(self.next_stream.fetch_add(2, Ordering::Relaxed))
    }

    pub(crate) fn mint_exec(&self) -> strop_worker_protocol::ExecId {
        strop_worker_protocol::ExecId(self.next_exec.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn refuse(id: RequestId, refusal: Refusal) -> WorkerMessage {
        WorkerMessage::Result {
            id,
            outcome: ResultOutcome::Refused { refusal },
        }
    }
}

fn mint(error_stage: &'static str) -> Result<u64, ServeError> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes)
        .map_err(|error| ServeError::Random(format!("{error_stage}: {error}")))?;
    Ok(u64::from_le_bytes(bytes))
}

/// The native boot observation for the namespace identity: Linux's boot
/// id, honestly "unattested" elsewhere rather than a display hostname.
fn namespace_identity() -> NamespaceIdentity {
    #[cfg(target_os = "linux")]
    let identity = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|id| id.trim().to_owned())
        .unwrap_or_else(|_| "unattested".into());
    #[cfg(not(target_os = "linux"))]
    let identity = "unattested".to_owned();
    #[cfg(unix)]
    let principal = Some(unsafe { libc::geteuid() });
    #[cfg(not(unix))]
    let principal = None;
    NamespaceIdentity {
        identity,
        principal,
    }
}

fn capabilities() -> Capabilities {
    let native_fs = cfg!(any(target_os = "linux", target_os = "macos"));
    Capabilities {
        observe: true,
        list: true,
        read: true,
        write: native_fs,
        trash: native_fs,
        notify: if cfg!(target_os = "linux") {
            NotifyCoverage::Native
        } else {
            NotifyCoverage::Unsupported
        },
        exec_finite: cfg!(unix),
        exec_service: cfg!(unix),
        // Local PTY ownership is WK12's terminal integration; until then a
        // pty request is a typed capability refusal, never a silent pipe.
        pty: false,
    }
}

fn limits() -> Limits {
    Limits {
        max_frame_bytes: strop_worker_protocol::MAX_BODY_BYTES,
        max_chunk_bytes: strop_worker_protocol::MAX_CHUNK_BYTES,
        max_pending_requests: 64,
        max_batch_steps: strop_fs::batch::STEP_LIMIT,
        max_listing_entries: 100_000,
        max_subscriptions: 64,
        max_streams: 64,
        max_exec_processes: 32,
    }
}

/// Serve one client session until authorized shutdown or disconnect. The
/// first frame must complete the handshake; the writer carries frames
/// only.
pub fn run(reader: impl Read, mut writer: impl Write + Send + 'static) -> Result<(), ServeError> {
    let mut reader = reader;
    let mut decoder = FrameDecoder::default();
    let Some(session) = handshake(&mut reader, &mut decoder, &mut writer)? else {
        return Ok(());
    };
    let shared = Arc::new(SessionState {
        authority: Mutex::new(Authority::new(session)),
        writer: Mutex::new(writer),
        diagnostics: Mutex::new(Diag { lines: 0 }),
        context: ExecutionContext::native(),
        environment: Environment::capture(),
        limits: limits(),
        requests: Mutex::new(HashMap::new()),
        inbound: Mutex::new(HashMap::new()),
        execs: Mutex::new(HashMap::new()),
        next_stream: AtomicU64::new(0),
        next_exec: AtomicU64::new(0),
        stop: AtomicBool::new(false),
        #[cfg(target_os = "linux")]
        notify: Arc::new(Mutex::new(crate::notify::NotifyManager::new(
            crate::notify::NotifyConfig::default(),
        ))),
    });
    #[cfg(target_os = "linux")]
    let pump = start_notify_pump(&shared);
    let reason = read_loop(&shared, &mut reader, &mut decoder);
    shared.stop.store(true, Ordering::Release);
    #[cfg(target_os = "linux")]
    let _ = pump.join();
    teardown(&shared, reason);
    Ok(())
}

/// The handshake: exactly one `hello`, answered by `welcome` with the
/// fresh session authority. Anything else — or a version this build does
/// not speak — is a typed in-band failure and a clean close, never a
/// guessed downgrade.
fn handshake(
    reader: &mut impl Read,
    decoder: &mut FrameDecoder,
    writer: &mut impl Write,
) -> Result<Option<Session>, ServeError> {
    let first = frame::read_frame(reader, decoder)?;
    let Some(body) = first else {
        return Ok(None);
    };
    let hello = match codec::decode_body::<ClientMessage>(&body) {
        Ok(Incoming::Envelope(ClientMessage::Hello { protocol, .. })) => protocol,
        Ok(_) => {
            codec::write_envelope(
                &mut *writer,
                &WorkerMessage::Error {
                    id: None,
                    error: ProtocolError::Unexpected {
                        message: "the first message must be hello".into(),
                    },
                },
            )?;
            return Ok(None);
        }
        Err(error) => {
            codec::write_envelope(
                &mut *writer,
                &WorkerMessage::Error {
                    id: None,
                    error: ProtocolError::Decode {
                        message: error.to_string(),
                    },
                },
            )?;
            return Ok(None);
        }
    };
    if hello != PROTOCOL_VERSION {
        codec::write_envelope(
            &mut *writer,
            &WorkerMessage::Error {
                id: None,
                error: ProtocolError::Version {
                    supported: PROTOCOL_VERSION,
                    offered: hello,
                },
            },
        )?;
        codec::write_envelope(
            &mut *writer,
            &WorkerMessage::Bye {
                reason: ShutdownReason::ProtocolViolation,
            },
        )?;
        return Ok(None);
    }
    let session = Session {
        incarnation: mint("incarnation")?,
        lease: strop_worker_protocol::LeaseId(mint("lease")?),
    };
    codec::write_envelope(
        writer,
        &WorkerMessage::Welcome {
            protocol: PROTOCOL_VERSION,
            worker: EndpointInfo {
                name: "strop".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                build: None,
                target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            },
            session,
            namespace: namespace_identity(),
            limits: limits(),
            capabilities: capabilities(),
        },
    )?;
    Ok(Some(session))
}

/// The post-handshake frame loop. Returns why the session ended; exec
/// revocation and the drain happen in [`teardown`].
fn read_loop<W: Write + Send + 'static>(
    shared: &Arc<SessionState<W>>,
    reader: &mut impl Read,
    decoder: &mut FrameDecoder,
) -> ShutdownReason {
    loop {
        let body = match frame::read_frame(reader, decoder) {
            Ok(Some(body)) => body,
            Ok(None) => return ShutdownReason::Disconnect,
            Err(error) => {
                shared.note(format_args!("frame read failed: {error}"));
                return ShutdownReason::Disconnect;
            }
        };
        let incoming = match codec::decode_body::<ClientMessage>(&body) {
            Ok(incoming) => incoming,
            Err(error) => {
                shared.send_error(
                    None,
                    ProtocolError::Decode {
                        message: error.to_string(),
                    },
                );
                return ShutdownReason::ProtocolViolation;
            }
        };
        match incoming {
            Incoming::Chunk(chunk) => {
                if let Err(violation) = route_chunk(shared, chunk) {
                    shared.send_error(None, ProtocolError::Stream { message: violation });
                    return ShutdownReason::ProtocolViolation;
                }
            }
            Incoming::Envelope(message) => match message {
                ClientMessage::Hello { .. } => {
                    shared.send_error(
                        None,
                        ProtocolError::Unexpected {
                            message: "a second hello is not admitted".into(),
                        },
                    );
                    return ShutdownReason::ProtocolViolation;
                }
                ClientMessage::Request { session, id, body } => {
                    admit_request(shared, session, id, *body);
                }
                ClientMessage::Cancel { session, id } => {
                    if shared.authority.lock().admit(&session).is_ok() {
                        if let Some(handle) = shared.requests.lock().remove(&id) {
                            handle.cancel(CancelReason::Dismissed);
                        }
                    }
                }
                ClientMessage::Shutdown { session } => {
                    if shared.authority.lock().admit(&session).is_err() {
                        return ShutdownReason::Disconnect;
                    }
                    return ShutdownReason::Requested;
                }
            },
        }
    }
}

/// Admission happens on the read loop before a thread is spawned: session
/// freshness, mutation retirement and the pending bound are decided here,
/// so a refused request never reaches a handle table.
fn admit_request<W: Write + Send + 'static>(
    shared: &Arc<SessionState<W>>,
    stamped: Session,
    id: RequestId,
    body: Request,
) {
    let admitted = {
        let authority = shared.authority.lock();
        let mutation = matches!(
            body,
            Request::Prepare { .. } | Request::Apply { .. } | Request::Exec { .. }
        );
        if mutation {
            authority.admit_mutation(&stamped)
        } else {
            authority.admit(&stamped)
        }
    };
    if let Err(refusal) = admitted {
        shared.send(&SessionState::<W>::refuse(id, refusal));
        return;
    }
    if shared.requests.lock().len() >= shared.limits.max_pending_requests {
        shared.send(&SessionState::<W>::refuse(
            id,
            Refusal::Busy {
                message: "admitted request bound reached".into(),
            },
        ));
        return;
    }
    let (token, handle) = CancelToken::standalone();
    shared.requests.lock().insert(id, handle);
    let worker = Arc::clone(shared);
    thread::spawn(move || {
        run_request(&worker, id, &token, body);
        token.clear_cancel_resource();
        worker.requests.lock().remove(&id);
    });
}

/// One admitted request on its own thread. `read` writes its own reply
/// (the `ReadOpened` envelope must precede its chunks on the wire); every
/// other family resolves to one outcome envelope.
fn run_request<W: Write + Send + 'static>(
    shared: &Arc<SessionState<W>>,
    id: RequestId,
    token: &CancelToken,
    body: Request,
) {
    let outcome = match body {
        Request::Observe { locations } => fs::observe(shared, token, &locations),
        Request::List { location, cursor } => fs::list(shared, token, location, cursor),
        Request::Read {
            location,
            offset,
            length,
        } => {
            fs::read_streaming(shared, id, token, location, offset, length);
            return;
        }
        Request::Prepare {
            intents,
            environment,
            ..
        } => fs::prepare(shared, token, &intents, environment),
        Request::Apply {
            steps,
            content,
            binding,
        } => fs::apply(shared, token, steps, content, binding),
        Request::Verify { attempt, binding } => fs::verify(shared, token, &attempt, binding),
        Request::Exec { spec } => {
            // The `ExecStarted` reply must precede any pump chunk on the
            // wire, so exec owns its reply like read does.
            process::exec(shared, id, token, spec);
            return;
        }
        Request::ExecHalfClose { exec } => process::half_close(shared, exec),
        Request::ExecCancel { exec } => process::cancel(shared, exec),
        Request::Subscribe { scope, recursive } => subscribe(shared, &scope, recursive),
        Request::Unsubscribe { subscription } => unsubscribe(shared, subscription),
        Request::Health => ResultOutcome::Healthy,
        Request::Quiesce => {
            shared.authority.lock().quiesce();
            ResultOutcome::Quiesced
        }
    };
    shared.send(&WorkerMessage::Result { id, outcome });
}

#[cfg(target_os = "linux")]
fn subscribe<W: Write>(
    shared: &Arc<SessionState<W>>,
    scope: &ResourceLocation,
    recursive: bool,
) -> ResultOutcome {
    match shared.notify.lock().subscribe(scope, recursive) {
        Ok(subscribed) => ResultOutcome::Subscribed {
            subscription: subscribed.subscription,
            coverage: subscribed.coverage,
        },
        Err(error) => ResultOutcome::Refused {
            refusal: error.refusal(),
        },
    }
}

#[cfg(not(target_os = "linux"))]
fn subscribe<W: Write>(
    _shared: &Arc<SessionState<W>>,
    _scope: &ResourceLocation,
    _recursive: bool,
) -> ResultOutcome {
    ResultOutcome::Refused {
        refusal: Refusal::Capability {
            capability: strop_worker_protocol::Capability::Notify,
        },
    }
}

#[cfg(target_os = "linux")]
fn unsubscribe<W: Write>(
    shared: &Arc<SessionState<W>>,
    subscription: strop_worker_protocol::Subscription,
) -> ResultOutcome {
    match shared.notify.lock().unsubscribe(subscription) {
        Ok(()) => ResultOutcome::Done,
        Err(error) => ResultOutcome::Refused {
            refusal: error.refusal(),
        },
    }
}

#[cfg(not(target_os = "linux"))]
fn unsubscribe<W: Write>(
    _shared: &Arc<SessionState<W>>,
    _subscription: strop_worker_protocol::Subscription,
) -> ResultOutcome {
    ResultOutcome::Refused {
        refusal: Refusal::Capability {
            capability: strop_worker_protocol::Capability::Notify,
        },
    }
}

#[cfg(target_os = "linux")]
fn start_notify_pump<W: Write + Send + 'static>(
    shared: &Arc<SessionState<W>>,
) -> thread::JoinHandle<()> {
    let worker = Arc::clone(shared);
    let manager = Arc::clone(&shared.notify);
    thread::spawn(move || {
        while !worker.stop.load(Ordering::Acquire) {
            let drained = {
                let mut manager = manager.lock();
                match manager.poll(Some(NOTIFY_TICK)) {
                    Ok(_) => manager.drain(),
                    Err(error) => {
                        worker.note(format_args!("notify poll: {error}"));
                        Ok(Vec::new())
                    }
                }
            };
            match drained {
                Ok(events) => {
                    for event in events {
                        worker.send(&WorkerMessage::Event { event });
                    }
                }
                Err(error) => worker.note(format_args!("notify drain: {error}")),
            }
        }
    })
}

/// Client → worker chunk routing: upload streams accumulate in order,
/// exec stdin relays to its pump. Ordering violations, unknown streams,
/// post-close chunks and size violations poison the session — they are
/// protocol, never data.
fn route_chunk<W: Write>(shared: &Arc<SessionState<W>>, chunk: StreamChunk) -> Result<(), String> {
    let stream = chunk.stream;
    let mut inbound = shared.inbound.lock();
    if !inbound.contains_key(&stream) {
        if stream.0.is_multiple_of(2) {
            // Worker-minted (even) streams are registered before their
            // first chunk can arrive; an unknown even id names nothing.
            return Err(format!("unknown stream {}", stream.0));
        }
        if inbound.len() >= shared.limits.max_streams {
            return Err("inbound stream bound reached".into());
        }
        if chunk.sequence != 0 {
            return Err(format!(
                "stream {} opened at sequence {}",
                stream.0, chunk.sequence
            ));
        }
        inbound.insert(
            stream,
            Inbound::Content {
                bytes: Vec::new(),
                next_sequence: 0,
                closed: false,
            },
        );
    }
    let entry = inbound.get_mut(&stream).unwrap();
    match entry {
        Inbound::Content {
            bytes,
            next_sequence,
            closed,
        } => {
            if *closed {
                return Err(format!("chunk on closed stream {}", stream.0));
            }
            if chunk.sequence != *next_sequence {
                return Err(format!(
                    "stream {} sequence {} out of order",
                    stream.0, chunk.sequence
                ));
            }
            if bytes.len() + chunk.bytes.len() > fs::MAX_CONTENT_BYTES {
                return Err("content stream exceeds the 256 MiB bound".into());
            }
            *next_sequence += 1;
            bytes.extend_from_slice(&chunk.bytes);
            *closed = chunk.last;
            Ok(())
        }
        Inbound::ExecStdin {
            sender,
            next_sequence,
        } => {
            if chunk.sequence != *next_sequence {
                return Err(format!(
                    "stream {} sequence {} out of order",
                    stream.0, chunk.sequence
                ));
            }
            *next_sequence += 1;
            let last = chunk.last;
            let failed = sender.try_send(chunk).is_err();
            if failed {
                // The child is not draining fast enough; its lease is
                // revoked rather than letting protocol control wait behind
                // a full data queue (WK11 owns fair scheduling). The
                // session itself is not poisoned — this is flow pressure,
                // not corruption.
                if let Some(exec) = process::exec_for_stream(shared, stream) {
                    process::revoke(shared, exec);
                }
            }
            if failed || last {
                inbound.remove(&stream);
            }
            Ok(())
        }
    }
}

/// Authorized retirement or disconnect: revoke every admitted exec, drain
/// in-flight requests, and publish `bye` only when the client is still
/// there to read it.
fn teardown<W: Write>(shared: &Arc<SessionState<W>>, reason: ShutdownReason) {
    shared.authority.lock().quiesce();
    {
        let mut execs = shared.execs.lock();
        for (_, entry) in execs.drain() {
            entry.cancel.cancel(CancelReason::Shutdown);
        }
    }
    let deadline = std::time::Instant::now() + DRAIN_WAIT;
    loop {
        if shared.requests.lock().is_empty() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            shared.note("shutdown drain expired with requests in flight");
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if reason == ShutdownReason::Requested {
        shared.send(&WorkerMessage::Bye { reason });
    }
    shared.authority.lock().close();
}
