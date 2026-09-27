//! The worker's serving loop (0058 WK04; WK11 scheduling): one client
//! session over one bounded transport. Frames arrive from `reader`
//! (stdin in `strop --worker-stdio`, a socketpair in tests), admitted
//! requests are dispatched to the strop-fs kernel, the exec supervisor
//! and the notify backend, and outcomes leave through the session
//! scheduler's writer thread (stdout in production — protocol bytes
//! only). Diagnostics are private and bounded; they are never protocol
//! authority.
//!
//! ## Threading and scheduling (WK11)
//!
//! ```text
//! main thread      blocking frame read → admission → dispatch
//! writer thread    drains the control lane before the data lane
//! request threads  one per admitted request (bounded by the budget registry)
//! notify pump      25 ms poll tick, events through the control lane
//! exec pumps       per-exec stdout/stderr/stdin supervisors
//! ```
//!
//! The budget registry and the two-lane outbound scheduler live in
//! [`schedule`]: control frames (outcomes, errors, events, `bye`) never
//! queue behind bulk stream data, cancellation wakes lane-blocked
//! producers, and every admitted request settles exactly once — an
//! outcome, a typed refusal, or session death. Stream ids are
//! session-scoped and partitioned by allocator: the client mints odd
//! ids (upload content streams), the worker mints even ids (read
//! payloads, exec pipes). Neither side ever guesses the other's.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use strop_core::worker::{CancelReason, CancelToken};
use strop_fs::Environment;
use strop_fs::ExecutionContext;
use strop_worker_protocol::codec::{self, Incoming, StreamChunk};
use strop_worker_protocol::frame::{self, FrameDecoder};
use strop_worker_protocol::{
    Authority, ClientMessage, Limits, NamespaceIdentity, ProtocolError, Refusal, Request,
    RequestId, ResultOutcome, Session, ShutdownReason, StreamId, WorkerMessage,
};
use strop_workspace::operation::{FsFailure, FsFailureKind};

#[cfg(unix)]
mod cache_gc;
#[cfg(unix)]
mod cache_lease;
mod fs;
mod handshake;
mod notify;
mod process;
mod schedule;
mod stream_window;

#[cfg(unix)]
type CacheGuard = Option<cache_lease::CacheLeaseGuard>;
#[cfg(not(unix))]
type CacheGuard = ();
use handshake::{handshake, limits};
pub(crate) use schedule::{Budgets, Outbound, PushChunk, Scheduler};

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

/// One uploaded content stream still accumulating (client → worker), a
/// relayed exec stdin sink, or a PTY's ordered input queue.
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
    /// PTY input (WK12): chunks ride the exec's ordered control queue.
    PtyStdin {
        sender: std::sync::mpsc::SyncSender<process::PtyControl>,
        next_sequence: u64,
        closed: bool,
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
/// rejected frame never reaches a handle. Outbound frames never touch
/// the transport here: they cross the [`Scheduler`], whose writer thread
/// alone owns the writer.
pub(crate) struct SessionState {
    authority: Mutex<Authority>,
    /// Held by every admitted request's shared session until its last
    /// effect settles, even if the bounded shutdown drain expires.
    cache_lease: CacheGuard,
    /// Admission budgets and the two-lane outbound scheduler (WK11).
    pub(crate) scheduler: Scheduler,
    diagnostics: Mutex<Diag>,
    context: ExecutionContext,
    namespace: NamespaceIdentity,
    environment: Environment,
    limits: Limits,
    inbound: Mutex<HashMap<StreamId, Inbound>>,
    /// File-read and exec-output credits, bounded by the admitted
    /// read count plus two output streams per live exec.
    stream_windows: Mutex<HashMap<StreamId, Arc<stream_window::StreamWindow>>>,
    execs: Mutex<HashMap<strop_worker_protocol::ExecId, process::ExecEntry>>,
    /// Even stream ids; the client owns odd ones.
    next_stream: AtomicU64,
    next_exec: AtomicU64,
    stop: AtomicBool,
    #[cfg(target_os = "linux")]
    notify: Arc<Mutex<crate::notify::NotifyManager>>,
}

impl SessionState {
    /// One control frame (outcome, error, event, `bye`). Dropped only
    /// once the session has halted — session death is then the terminal
    /// disposition for everything in flight.
    pub(crate) fn send(&self, message: WorkerMessage) {
        self.scheduler.push_control(message);
    }

    /// One data-lane chunk, blocking (cancel-aware) while the lane is
    /// full; see [`Scheduler::push_chunk`].
    pub(crate) fn push_chunk(&self, chunk: StreamChunk, token: Option<&CancelToken>) -> PushChunk {
        self.scheduler.push_chunk(chunk, token)
    }

    /// A stream's terminal marker: the empty `last` chunk. Bypasses a
    /// full data lane — EOF never queues behind data.
    pub(crate) fn end_stream(&self, stream: StreamId, sequence: u64) {
        self.scheduler.push_chunk(
            StreamChunk {
                stream,
                sequence,
                last: true,
                bytes: Vec::new(),
            },
            None,
        );
    }

    fn send_error(&self, id: Option<RequestId>, error: ProtocolError) {
        self.send(WorkerMessage::Error { id, error });
    }

    fn note(&self, message: impl std::fmt::Display) {
        self.diagnostics.lock().note(message);
    }

    /// Stop the session: flag producers/pumps and halt the scheduler so
    /// every lane-blocked waiter wakes and settles.
    fn halt(&self) {
        self.stop.store(true, Ordering::Release);
        self.scheduler.halt();
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

/// Serve one client session until authorized shutdown or disconnect. The
/// first frame must complete the handshake; the writer carries frames
/// only.
pub fn run(reader: impl Read, writer: impl Write + Send + 'static) -> Result<(), ServeError> {
    run_with_limits(reader, writer, limits())
}

/// As [`run`], with explicit negotiated bounds: the binary serves the
/// built-in defaults; tests tighten them to reach the budget edges
/// without filling the production registries.
pub fn run_with_limits(
    reader: impl Read,
    mut writer: impl Write + Send + 'static,
    limits: Limits,
) -> Result<(), ServeError> {
    let mut reader = reader;
    let mut decoder = FrameDecoder::default();
    let Some((session, namespace, cache_lease)) =
        handshake(&mut reader, &mut decoder, &mut writer, &limits)?
    else {
        return Ok(());
    };
    let shared = Arc::new(SessionState {
        authority: Mutex::new(Authority::new(session)),
        cache_lease,
        scheduler: Scheduler::new(Budgets::from_limits(&limits)),
        diagnostics: Mutex::new(Diag { lines: 0 }),
        context: ExecutionContext::native(),
        namespace,
        environment: Environment::capture(),
        limits,
        inbound: Mutex::new(HashMap::new()),
        stream_windows: Mutex::new(HashMap::new()),
        execs: Mutex::new(HashMap::new()),
        next_stream: AtomicU64::new(0),
        next_exec: AtomicU64::new(0),
        stop: AtomicBool::new(false),
        #[cfg(target_os = "linux")]
        notify: Arc::new(Mutex::new(crate::notify::NotifyManager::new(
            crate::notify::NotifyConfig::default(),
        ))),
    });
    let writing = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || write_loop(&shared, writer))
    };
    #[cfg(target_os = "linux")]
    let pump = notify::start_notify_pump(&shared);
    let reason = read_loop(&shared, &mut reader, &mut decoder);
    shared.stop.store(true, Ordering::Release);
    // Wake lane-blocked producers so the drain observes their tokens
    // promptly instead of waiting out the lane.
    shared.scheduler.wake_producers();
    #[cfg(target_os = "linux")]
    let _ = pump.join();
    teardown(&shared, reason);
    // Drain, then halt: the writer publishes the queued `bye` (and any
    // settled outcomes) before observing the halt and exiting.
    shared.scheduler.halt();
    let _ = writing.join();
    Ok(())
}

/// The one writer thread: drains the control lane before the data lane
/// (WK11 priority), so a health/quiesce outcome or a `cancel` effect
/// never queues behind bulk stream bytes. A write failure halts the
/// session — lane-blocked producers wake and settle by session death.
fn write_loop<W: Write>(shared: &Arc<SessionState>, mut writer: W) {
    while let Some(frame) = shared.scheduler.pop() {
        let written = match &frame {
            Outbound::Control(message) => codec::write_envelope(&mut writer, message),
            Outbound::Chunk(chunk) => {
                let result = codec::write_chunk(&mut writer, chunk);
                if result.is_ok() {
                    shared.scheduler.chunk_written();
                }
                result
            }
        };
        if let Err(error) = written {
            shared.note(format_args!("write failed: {error}"));
            shared.halt();
        }
    }
}

/// The post-handshake frame loop. Returns why the session ended; exec
/// revocation and the drain happen in [`teardown`]. Cancellation is
/// handled inline here — a `cancel` frame never queues behind requests
/// or stream chunks beyond its own transport position.
fn read_loop(
    shared: &Arc<SessionState>,
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
                        shared.scheduler.cancel(id);
                    }
                }
                ClientMessage::StreamCredit {
                    session,
                    stream,
                    chunks,
                } => {
                    if shared.authority.lock().admit(&session).is_err() {
                        continue;
                    }
                    if chunks == 0
                        || usize::from(chunks) > strop_worker_protocol::STREAM_WINDOW_CHUNKS
                    {
                        shared.send_error(
                            None,
                            ProtocolError::Stream {
                                message: "stream credit outside the negotiated window".into(),
                            },
                        );
                        return ShutdownReason::ProtocolViolation;
                    }
                    // A consumed chunk may race the worker's final
                    // marker/removal; a late credit is a no-op, never
                    // authority for a different stream.
                    if let Some(window) = shared.stream_windows.lock().get(&stream).cloned() {
                        window.grant(usize::from(chunks));
                    }
                }
                ClientMessage::StreamAbandon { session, stream } => {
                    if shared.authority.lock().admit(&session).is_err() {
                        continue;
                    }
                    if let Some(window) = shared.stream_windows.lock().get(&stream).cloned() {
                        window.abandon();
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

/// Admission happens on the read loop before a thread is spawned:
/// session freshness, mutation retirement and the per-class budgets are
/// decided here, so a refused request never reaches a handle table.
/// Settlement is exactly once: the spawned thread publishes the outcome
/// (or its stream's terminal chunk) and settles the registry slot.
fn admit_request(shared: &Arc<SessionState>, stamped: Session, id: RequestId, body: Request) {
    let admitted = {
        let authority = shared.authority.lock();
        let mutation = matches!(
            body,
            Request::Prepare { .. }
                | Request::Apply { .. }
                | Request::Exec { .. }
                | Request::CollectCache { .. }
        );
        if mutation {
            authority.admit_mutation(&stamped)
        } else {
            authority.admit(&stamped)
        }
    };
    if let Err(refusal) = admitted {
        shared.send(SessionState::refuse(id, refusal));
        return;
    }
    let (token, handle) = CancelToken::standalone();
    if let Err(refusal) = shared.scheduler.admit(id, body.class(), handle) {
        shared.send(SessionState::refuse(id, refusal));
        return;
    }
    let worker = Arc::clone(shared);
    thread::spawn(move || {
        run_request(&worker, id, &token, body);
        token.clear_cancel_resource();
        worker.scheduler.settle(id);
    });
}

/// One admitted request on its own thread. `read` writes its own reply
/// (the `ReadOpened` envelope must precede its chunks on the wire); every
/// other family resolves to one outcome envelope.
fn run_request(shared: &Arc<SessionState>, id: RequestId, token: &CancelToken, body: Request) {
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
        Request::VerifyRecovered {
            attempt,
            namespace,
            binding,
        } => fs::verify_recovered(shared, token, &attempt, &namespace, binding),
        Request::Verify { attempt, binding } => fs::verify(shared, token, &attempt, binding),
        Request::Exec { spec } => {
            // The `ExecStarted` reply must precede any pump chunk on the
            // wire, so exec owns its reply like read does.
            process::exec(shared, id, token, spec);
            return;
        }
        Request::ExecHalfClose { exec } => process::half_close(shared, exec),
        Request::ExecCancel { exec } => process::cancel(shared, exec),
        Request::ExecResize { exec, geometry } => {
            // The resize reply is the ordered geometry boundary, so the
            // PTY input thread owns it; nothing more is sent here.
            process::enqueue_resize(shared, id, exec, geometry);
            return;
        }
        Request::Subscribe { scope, recursive } => notify::subscribe(shared, &scope, recursive),
        Request::Unsubscribe { subscription } => notify::unsubscribe(shared, subscription),
        Request::CollectCache { context } => {
            #[cfg(unix)]
            {
                cache_gc::collect(shared, token, &context)
            }
            #[cfg(not(unix))]
            {
                ResultOutcome::Failed {
                    failure: failure(FsFailureKind::Unsupported, "no native worker cache here"),
                }
            }
        }
        Request::Health => ResultOutcome::Healthy,
        Request::Quiesce => {
            shared.authority.lock().quiesce();
            ResultOutcome::Quiesced
        }
    };
    shared.send(WorkerMessage::Result { id, outcome });
}

/// Client → worker chunk routing: upload streams accumulate in order,
/// exec stdin relays to its pump. Ordering violations, unknown streams,
/// post-close chunks and size violations poison the session — they are
/// protocol, never data.
fn route_chunk(shared: &Arc<SessionState>, chunk: StreamChunk) -> Result<(), String> {
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
                // a full data queue (WK11 fair scheduling). The session
                // itself is not poisoned — this is flow pressure, not
                // corruption.
                if let Some(exec) = process::exec_for_stream(shared, stream) {
                    process::revoke(shared, exec);
                }
            }
            if failed || last {
                inbound.remove(&stream);
            }
            Ok(())
        }
        Inbound::PtyStdin {
            sender,
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
            *next_sequence += 1;
            let last = chunk.last;
            // Same flow-pressure rule as piped stdin: a terminal that is
            // not draining loses its lease rather than stalling control.
            let failed = sender.try_send(process::PtyControl::Input(chunk)).is_err();
            if failed {
                if let Some(exec) = process::exec_for_stream(shared, stream) {
                    process::revoke(shared, exec);
                }
            }
            *closed = last;
            if failed || last {
                inbound.remove(&stream);
            }
            Ok(())
        }
    }
}

/// Authorized retirement or disconnect: revoke every admitted exec, drain
/// in-flight requests (each settles its own outcome or terminal chunk),
/// and publish `bye` only when the client is still there to read it.
/// The scheduler halt that ends the writer happens in [`run_with_limits`]
/// after this drain.
fn teardown(shared: &Arc<SessionState>, reason: ShutdownReason) {
    shared.authority.lock().quiesce();
    {
        let mut execs = shared.execs.lock();
        for (_, entry) in execs.drain() {
            entry.cancel.cancel(CancelReason::Shutdown);
            shared.scheduler.release_exec();
        }
    }
    let deadline = std::time::Instant::now() + DRAIN_WAIT;
    loop {
        if shared.scheduler.outstanding() == 0 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            shared.note("shutdown drain expired with requests in flight");
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if reason == ShutdownReason::Requested {
        shared.send(WorkerMessage::Bye { reason });
    }
    shared.authority.lock().close();
}
