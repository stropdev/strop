//! Connection machinery: one framed transport to one worker incarnation.
//!
//! A [`Conn`] owns the writer (shared, frame-atomic), the reader thread
//! (routing results, events and stream chunks), the child process handle
//! when the transport is a spawned process, and the worker's bounded
//! stderr capture for diagnostics. When the reader sees EOF, corruption
//! or `bye`, the connection terminates: every pending request and open
//! stream fails typed, and the next caller respawns through the connector
//! (a fresh incarnation — stale prepared authority fails closed there).

mod reader;

use reader::read_loop;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use strop_worker_protocol::codec::{self, StreamChunk};
use strop_worker_protocol::{
    Capabilities, ClientMessage, EndpointInfo, Event, ExecId, LeaseId, Limits, NamespaceIdentity,
    ProtocolError, Refusal, RequestClass, RequestId, Session, ShutdownReason, StreamId,
    WorkerMessage, PROTOCOL_VERSION,
};

use crate::error::ClientError;

/// Handshake readiness bound: spawn plus `hello`/`welcome` must resolve
/// inside this window or the attempt is a typed failure (never a silent
/// fallback, never an unbounded wait on a half-alive child).
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// The worker's private stderr is captured for diagnostics, bounded; it
/// is never protocol authority.
const STDERR_CAPTURE_LIMIT: usize = 64 * 1024;

/// One end of a worker transport, ready to handshake.
pub struct Transport {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Option<Child>,
    pub stderr: Option<StderrCapture>,
}

/// Bounded capture of a spawned worker's stderr for typed diagnostics.
#[derive(Clone)]
pub struct StderrCapture {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl StderrCapture {
    /// Drain a worker's stderr into the bounded capture, off the
    /// caller's thread. Public for deployed transports (WK07/WK08):
    /// their child pipes are taken by the caller, and the worker's
    /// private stderr still lands in the same bounded diagnostics.
    pub fn spawn(mut source: impl Read + Send + 'static) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buffer);
        thread::spawn(move || {
            let mut chunk = [0_u8; 4096];
            loop {
                match source.read(&mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(count) => {
                        let mut buffer = sink.lock();
                        if buffer.len() < STDERR_CAPTURE_LIMIT {
                            let room = STDERR_CAPTURE_LIMIT - buffer.len();
                            buffer.extend_from_slice(&chunk[..count.min(room)]);
                        }
                    }
                }
            }
        });
        Self { buffer }
    }

    /// The captured diagnostics as text, marked when truncated.
    pub fn text(&self) -> String {
        let buffer = self.buffer.lock();
        let mut text = String::from_utf8_lossy(&buffer).into_owned();
        if text.len() >= STDERR_CAPTURE_LIMIT {
            text.push_str("…[truncated]");
        }
        text.trim().to_owned()
    }
}

/// Spawn the given executable in worker mode. The caller chooses the
/// program — production passes `current_exe`, never a PATH lookup.
pub fn spawn_worker(program: &std::path::Path) -> Result<Transport, ClientError> {
    let mut child = Command::new(program)
        .arg("--worker-stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ClientError::Spawn(format!("{}: {error}", program.display())))?;
    let writer = child
        .stdin
        .take()
        .map(|stdin| Box::new(stdin) as Box<dyn Write + Send>);
    let reader = child
        .stdout
        .take()
        .map(|stdout| Box::new(stdout) as Box<dyn Read + Send>);
    let stderr = child.stderr.take().map(StderrCapture::spawn);
    let (Some(writer), Some(reader)) = (writer, reader) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ClientError::Spawn("worker stdio pipes unavailable".into()));
    };
    Ok(Transport {
        reader,
        writer,
        child: Some(child),
        stderr,
    })
}

/// What the reader thread attaches to a streaming family's reply:
/// payload sinks and exec-exit waiters, registered BEFORE the reply is
/// delivered so a chunk or exit event can never race its routing entry.
#[derive(Default)]
pub(crate) struct Attachments {
    pub streams: Vec<(StreamId, Receiver<StreamEvent>)>,
    pub exits: Vec<(ExecId, Receiver<ExecEvent>)>,
}

/// The reply routed to one waiting request.
pub(crate) enum Reply {
    Outcome {
        outcome: strop_worker_protocol::ResultOutcome,
        attachments: Attachments,
    },
    Protocol(ProtocolError),
    Lost(String),
}
/// One inbound stream's events (worker → client payloads). `Resized`
/// is the PTY ordered geometry boundary (0058 WK12): it rides the
/// exec's output stream in exact wire order, so a terminal parser
/// flips geometry exactly between the pre- and post-resize bytes.
pub enum StreamEvent {
    Chunk(StreamChunk),
    Resized,
    Failed(String),
}

/// One exec's routed lifecycle (0058 WK12): delivered-input
/// acknowledgments and the terminal status, on one channel in wire
/// order.
pub enum ExecEvent {
    Input { sequence: u64 },
    Exit(strop_worker_protocol::ExitStatus),
}

/// One waiting request's routing entry. `streaming` replies attach
/// payload sinks; `class` is the WK11 scheduling class the pending
/// budget counts against. `pty_resize` names the exec whose output
/// stream receives the ordered [`StreamEvent::Resized`] marker when
/// the reply is `done` (WK12).
pub(crate) struct Pending {
    sender: Sender<Reply>,
    streaming: bool,
    class: RequestClass,
    pty_resize: Option<ExecId>,
}

impl Pending {
    pub(crate) fn new(sender: Sender<Reply>, streaming: bool, class: RequestClass) -> Self {
        Pending {
            sender,
            streaming,
            class,
            pty_resize: None,
        }
    }

    /// A PTY resize request: control-class, and its `done` reply is the
    /// ordered geometry boundary on the exec's output stream.
    pub(crate) fn pty_resize(sender: Sender<Reply>, exec: ExecId) -> Self {
        Pending {
            sender,
            streaming: false,
            class: RequestClass::Control,
            pty_resize: Some(exec),
        }
    }
}

/// One inbound stream's routing slot (WK11).
enum StreamSlot {
    /// Actively routed. A read payload's owning request id lets
    /// backpressure cancel the stream at its source; exec pipes carry
    /// `None` (an abandoned exec payload never revokes the lease —
    /// dropping the handle is not cancellation).
    Live {
        sender: SyncSender<StreamEvent>,
        request: Option<RequestId>,
    },
    /// Abandoned (consumer dropped or exceeded the inbound budget):
    /// chunks are dropped, but the registration stays until the terminal
    /// chunk so routing stays exact and a late chunk is never misread as
    /// corruption.
    Abandoned,
}

struct Handshake {
    sender: Mutex<Option<Sender<Result<WorkerMessage, String>>>>,
}

/// The session state installed by the handshake. Placeholder until then;
/// no request can be admitted before `connect` returns, so accessors
/// never observe the placeholder.
#[derive(Clone)]
struct SessionState {
    session: Session,
    capabilities: Capabilities,
    limits: Limits,
    namespace: NamespaceIdentity,
}

/// One framed connection to one worker incarnation.
pub(crate) struct Conn {
    state: Mutex<SessionState>,
    next_request: AtomicU64,
    /// Odd stream ids; the worker mints even ones.
    next_stream: AtomicU64,
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Mutex<HashMap<RequestId, Pending>>,
    streams: Mutex<HashMap<StreamId, StreamSlot>>,
    execs: Mutex<HashMap<ExecId, Sender<ExecEvent>>>,
    /// Exec → its stdout stream, for routing the ordered resize
    /// boundary marker (WK12).
    exec_streams: Mutex<HashMap<ExecId, StreamId>>,
    /// Stream-arrival hook (WK12): the terminal service's wake nudge,
    /// fired after a chunk is routed. One per lease.
    stream_notifier: Mutex<Option<crate::StreamNotifier>>,
    events: Mutex<Option<Sender<Event>>>,
    handshake: Arc<Handshake>,
    bye: Mutex<Option<Sender<ShutdownReason>>>,
    alive: AtomicBool,
    death: Mutex<Option<String>>,
    child: Mutex<Option<Child>>,
    stderr: Option<StderrCapture>,
}

fn placeholder_state() -> SessionState {
    SessionState {
        session: Session {
            incarnation: 0,
            lease: LeaseId(0),
        },
        capabilities: Capabilities {
            observe: false,
            list: false,
            read: false,
            write: false,
            trash: false,
            notify: strop_worker_protocol::NotifyCoverage::Unsupported,
            exec_finite: false,
            exec_service: false,
            pty: false,
        },
        limits: Limits {
            max_frame_bytes: 0,
            max_chunk_bytes: 0,
            max_pending_requests: 0,
            max_batch_steps: 0,
            max_listing_entries: 0,
            max_subscriptions: 0,
            max_streams: 0,
            max_exec_processes: 0,
            max_concurrent_reads: 0,
            control_reserve: 0,
            max_queued_data_chunks: 0,
        },
        namespace: NamespaceIdentity {
            identity: String::new(),
            principal: None,
        },
    }
}

impl Conn {
    /// Establish one connection: handshake on the fresh transport, then
    /// spawn the routing reader thread. Readiness IS the handshake — a
    /// successful spawn alone never satisfies this. `expected_target` is
    /// the target triple the worker must report: this build's own for
    /// local transports, the admitted endpoint's for deployed workers
    /// (WK08 — a container/SSH worker is bound to *its* target, which
    /// the deploy flow verified, never to the client's platform).
    pub(crate) fn connect(
        transport: Transport,
        expected_target: &str,
    ) -> Result<Arc<Self>, ClientError> {
        let Transport {
            mut reader,
            writer,
            child,
            stderr,
        } = transport;
        let handshake = Arc::new(Handshake {
            sender: Mutex::new(None),
        });
        let conn = Arc::new(Self {
            state: Mutex::new(placeholder_state()),
            next_request: AtomicU64::new(0),
            next_stream: AtomicU64::new(1),
            writer: Mutex::new(writer),
            pending: Mutex::new(HashMap::new()),
            streams: Mutex::new(HashMap::new()),
            execs: Mutex::new(HashMap::new()),
            exec_streams: Mutex::new(HashMap::new()),
            stream_notifier: Mutex::new(None),
            events: Mutex::new(None),
            handshake: Arc::clone(&handshake),
            bye: Mutex::new(None),
            alive: AtomicBool::new(true),
            death: Mutex::new(None),
            child: Mutex::new(child),
            stderr,
        });
        // The reply channel is installed before the reader thread exists
        // and before `hello` is sent: a welcome routed before the sender
        // is registered would be dropped and misread as a timeout.
        let (tx, rx) = channel();
        *handshake.sender.lock() = Some(tx);
        let reading = Arc::clone(&conn);
        thread::spawn(move || read_loop(reading, &mut reader));
        codec::write_envelope(
            &mut *conn.writer.lock(),
            &ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                client: EndpointInfo {
                    name: "strop".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    build: None,
                    target: strop_worker_protocol::TARGET_TRIPLE.into(),
                },
            },
        )
        .map_err(|error| ClientError::Handshake(format!("cannot send hello: {error}")))?;
        match rx.recv_timeout(HANDSHAKE_TIMEOUT) {
            Ok(Ok(WorkerMessage::Welcome {
                protocol,
                worker,
                session,
                namespace,
                limits,
                capabilities,
            })) => {
                // Identity is checked before any admitted request; on
                // mismatch the conn drops here, terminating the child.
                Self::check_identity(&worker, expected_target)?;
                if protocol != PROTOCOL_VERSION {
                    return Err(ClientError::Mismatch {
                        field: "protocol",
                        expected: PROTOCOL_VERSION.to_string(),
                        actual: protocol.to_string(),
                    });
                }
                *conn.state.lock() = SessionState {
                    session,
                    capabilities,
                    limits,
                    namespace,
                };
                Ok(conn)
            }
            Ok(Ok(WorkerMessage::Error { error, .. })) => Err(ClientError::Protocol(error)),
            Ok(Ok(_)) => Err(ClientError::Handshake(
                "the first worker message was not a welcome".into(),
            )),
            Ok(Err(error)) => Err(ClientError::Handshake(error)),
            Err(_) => Err(ClientError::Handshake(format!(
                "no welcome within {}s",
                HANDSHAKE_TIMEOUT.as_secs()
            ))),
        }
    }
    /// The worker's reported identity binds to this client's exact
    /// release version and to the *expected* target triple — this
    /// build's own for a local worker, the admitted endpoint's for a
    /// deployed one. A mismatch is a typed refusal, never a downgrade.
    fn check_identity(worker: &EndpointInfo, expected_target: &str) -> Result<(), ClientError> {
        let expected: [(&'static str, &str, &str); 2] = [
            (
                "version",
                env!("CARGO_PKG_VERSION"),
                worker.version.as_str(),
            ),
            ("target", expected_target, worker.target.as_str()),
        ];
        for (field, expected, actual) in expected {
            if expected != actual {
                return Err(ClientError::Mismatch {
                    field,
                    expected: expected.to_owned(),
                    actual: actual.to_owned(),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub(crate) fn alloc_request(&self) -> RequestId {
        RequestId(self.next_request.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn alloc_stream(&self) -> StreamId {
        StreamId(self.next_stream.fetch_add(2, Ordering::Relaxed))
    }

    pub(crate) fn write(&self, message: &ClientMessage) -> Result<(), ClientError> {
        codec::write_envelope(&mut *self.writer.lock(), message)
            .map_err(|error| ClientError::WorkerLost(format!("write: {error}")))
    }

    /// Best-effort frame write for cancellation: never fails the caller.
    pub(crate) fn write_quiet(self: &Arc<Self>, message: &ClientMessage) {
        if self.alive() {
            let _ = codec::write_envelope(&mut *self.writer.lock(), message);
        }
    }

    pub(crate) fn write_chunk(&self, chunk: &StreamChunk) -> Result<(), ClientError> {
        if !self.alive() {
            return Err(ClientError::WorkerLost(self.death_detail()));
        }
        codec::write_chunk(&mut *self.writer.lock(), chunk)
            .map_err(|error| ClientError::WorkerLost(format!("write: {error}")))
    }

    pub(crate) fn child_pid(&self) -> Option<u32> {
        self.child.lock().as_ref().map(|child| child.id())
    }

    /// Admit one request into the pending registry under the negotiated
    /// inbound budgets (WK11): the client mirrors the worker's admission
    /// bounds — control-class requests draw from the reserved slots, bulk
    /// reads additionally from the read budget — so overload fails typed
    /// at the caller instead of queueing behind bulk on the wire.
    pub(crate) fn admit(&self, id: RequestId, pending: Pending) -> Result<(), ClientError> {
        if !self.alive() {
            return Err(ClientError::WorkerLost(self.death_detail()));
        }
        let limits = self.limits();
        let mut registry = self.pending.lock();
        let total = registry.len();
        let admitted = match pending.class {
            RequestClass::Control => total < limits.max_pending_requests,
            RequestClass::Standard => {
                total
                    < limits
                        .max_pending_requests
                        .saturating_sub(limits.control_reserve)
            }
            RequestClass::Bulk => {
                total
                    < limits
                        .max_pending_requests
                        .saturating_sub(limits.control_reserve)
                    && registry
                        .values()
                        .filter(|entry| entry.class == RequestClass::Bulk)
                        .count()
                        < limits.max_concurrent_reads
            }
        };
        if !admitted {
            return Err(ClientError::Refused(Refusal::Busy {
                message: format!("{:?} inbound budget reached", pending.class),
            }));
        }
        registry.insert(id, pending);
        Ok(())
    }

    pub(crate) fn retract(&self, id: RequestId) {
        self.pending.lock().remove(&id);
    }

    pub(crate) fn set_events(&self, sender: Option<Sender<Event>>) {
        *self.events.lock() = sender;
    }

    /// The stream-arrival hook (WK12): fired by the reader thread after
    /// a chunk is routed, so fd-polling consumers (the terminal
    /// service) wake on output instead of their poll timeout.
    pub(crate) fn set_stream_notifier(&self, notifier: crate::StreamNotifier) {
        *self.stream_notifier.lock() = Some(notifier);
    }

    /// Terminate: every pending request and open stream fails typed, the
    /// child (if any) is killed and reaped. Idempotent.
    pub(crate) fn terminate(self: &Arc<Self>, reason: impl Into<String>) {
        if !self.alive.swap(false, Ordering::AcqRel) {
            return;
        }
        let reason = reason.into();
        *self.death.lock() = Some(reason.clone());
        for (_, pending) in self.pending.lock().drain() {
            let _ = pending.sender.send(Reply::Lost(reason.clone()));
        }
        for (_, stream) in self.streams.lock().drain() {
            if let StreamSlot::Live { sender, .. } = stream {
                let _ = sender.send(StreamEvent::Failed(reason.clone()));
            }
        }
        self.exec_streams.lock().clear();
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// The death reason with the worker's bounded diagnostics attached.
    pub(crate) fn death_detail(&self) -> String {
        let mut detail = self.death.lock().clone().unwrap_or_else(|| "closed".into());
        if let Some(stderr) = &self.stderr {
            let text = stderr.text();
            if !text.is_empty() {
                detail.push_str("; worker stderr: ");
                detail.push_str(&text);
            }
        }
        detail
    }

    /// Orderly retirement: `shutdown`, a bounded wait for `bye`, then the
    pub(crate) fn retire(self: &Arc<Self>) {
        if !self.alive() {
            return;
        }
        let (tx, rx) = channel();
        *self.bye.lock() = Some(tx);
        self.write_quiet(&ClientMessage::Shutdown {
            session: self.session(),
        });
        let _ = rx.recv_timeout(Duration::from_secs(2));
        self.terminate("retired at last-owner close".to_owned());
    }

    pub(crate) fn session(&self) -> Session {
        self.state.lock().session
    }

    pub(crate) fn capabilities(&self) -> Capabilities {
        self.state.lock().capabilities
    }

    pub(crate) fn limits(&self) -> Limits {
        self.state.lock().limits
    }

    pub(crate) fn namespace(&self) -> NamespaceIdentity {
        self.state.lock().namespace.clone()
    }
}
