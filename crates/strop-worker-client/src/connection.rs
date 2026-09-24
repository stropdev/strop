//! Connection machinery: one framed transport to one worker incarnation.
//!
//! A [`Conn`] owns the writer (shared, frame-atomic), the reader thread
//! (routing results, events and stream chunks), the child process handle
//! when the transport is a spawned process, and the worker's bounded
//! stderr capture for diagnostics. When the reader sees EOF, corruption
//! or `bye`, the connection terminates: every pending request and open
//! stream fails typed, and the next caller respawns through the connector
//! (a fresh incarnation — stale prepared authority fails closed there).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use strop_worker_protocol::codec::{self, Incoming, StreamChunk};
use strop_worker_protocol::frame::{self, FrameDecoder};
use strop_worker_protocol::{
    Capabilities, ClientMessage, EndpointInfo, Event, ExecId, LeaseId, Limits, NamespaceIdentity,
    ProtocolError, RequestId, Session, ShutdownReason, StreamId, WorkerMessage, PROTOCOL_VERSION,
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
    fn spawn(mut source: impl Read + Send + 'static) -> Self {
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
    pub exits: Vec<(ExecId, Receiver<ExitEvent>)>,
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
/// One inbound stream's events (worker → client payloads).
pub(crate) enum StreamEvent {
    Chunk(StreamChunk),
    Failed(String),
}

enum Pending {
    Plain(Sender<Reply>),
    Streaming(Sender<Reply>),
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
    streams: Mutex<HashMap<StreamId, Sender<StreamEvent>>>,
    execs: Mutex<HashMap<ExecId, Sender<ExitEvent>>>,
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
        },
        namespace: NamespaceIdentity {
            identity: String::new(),
            principal: None,
        },
    }
}
/// An exec's terminal status routed from the event stream.
pub(crate) struct ExitEvent(pub strop_worker_protocol::ExitStatus);

impl Conn {
    /// Establish one connection: handshake on the fresh transport, then
    /// spawn the routing reader thread. Readiness IS the handshake — a
    /// successful spawn alone never satisfies this.
    pub(crate) fn connect(transport: Transport) -> Result<Arc<Self>, ClientError> {
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
                    target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
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
                Self::check_identity(&worker)?;
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
            Ok(Ok(WorkerMessage::Error { error, .. })) => {
                Err(ClientError::Handshake(format!("refused: {error}")))
            }
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
    fn check_identity(worker: &EndpointInfo) -> Result<(), ClientError> {
        let target = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
        let expected: [(&'static str, &str, &str); 2] = [
            (
                "version",
                env!("CARGO_PKG_VERSION"),
                worker.version.as_str(),
            ),
            ("target", target.as_str(), worker.target.as_str()),
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
        codec::write_chunk(&mut *self.writer.lock(), chunk)
            .map_err(|error| ClientError::WorkerLost(format!("write: {error}")))
    }

    pub(crate) fn child_pid(&self) -> Option<u32> {
        self.child.lock().as_ref().map(|child| child.id())
    }

    pub(crate) fn register_plain(&self, id: RequestId, sender: Sender<Reply>) {
        self.pending.lock().insert(id, Pending::Plain(sender));
    }

    pub(crate) fn register_streaming(&self, id: RequestId, sender: Sender<Reply>) {
        self.pending.lock().insert(id, Pending::Streaming(sender));
    }

    pub(crate) fn retract(&self, id: RequestId) {
        self.pending.lock().remove(&id);
    }

    pub(crate) fn set_events(&self, sender: Option<Sender<Event>>) {
        *self.events.lock() = sender;
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
            let sender = match pending {
                Pending::Plain(sender) => sender,
                Pending::Streaming(sender) => sender,
            };
            let _ = sender.send(Reply::Lost(reason.clone()));
        }
        for (_, stream) in self.streams.lock().drain() {
            let _ = stream.send(StreamEvent::Failed(reason.clone()));
        }
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

/// The reader thread: decode frames, route results/events/chunks, and on
/// EOF or corruption terminate the connection so every waiter fails typed.
fn read_loop(conn: Arc<Conn>, reader: &mut impl Read) {
    let mut decoder = FrameDecoder::default();
    loop {
        let body = match frame::read_frame(reader, &mut decoder) {
            Ok(Some(body)) => body,
            Ok(None) => {
                conn.terminate("worker closed the protocol stream".to_owned());
                return;
            }
            Err(error) => {
                conn.terminate(format!("protocol stream failed: {error}"));
                return;
            }
        };
        match codec::decode_body::<WorkerMessage>(&body) {
            Ok(Incoming::Envelope(message)) => route_envelope(&conn, message),
            Ok(Incoming::Chunk(chunk)) => route_chunk(&conn, chunk),
            Err(error) => {
                conn.terminate(format!("undecodable worker frame: {error}"));
                return;
            }
        }
        if !conn.alive() {
            return;
        }
    }
}

fn route_envelope(conn: &Arc<Conn>, message: WorkerMessage) {
    match message {
        WorkerMessage::Welcome { .. } => {
            if let Some(sender) = conn.handshake.sender.lock().take() {
                let _ = sender.send(Ok(message));
            }
        }
        WorkerMessage::Result { id, outcome } => {
            let pending = conn.pending.lock().remove(&id);
            match pending {
                Some(Pending::Plain(sender)) => {
                    let _ = sender.send(Reply::Outcome {
                        outcome,
                        attachments: Attachments::default(),
                    });
                }
                Some(Pending::Streaming(sender)) => {
                    let mut attachments = Attachments::default();
                    match &outcome {
                        strop_worker_protocol::ResultOutcome::ReadOpened { stream, .. } => {
                            let (tx, rx) = channel();
                            conn.streams.lock().insert(*stream, tx);
                            attachments.streams.push((*stream, rx));
                        }
                        strop_worker_protocol::ResultOutcome::ExecStarted {
                            exec,
                            stdout,
                            stderr,
                            ..
                        } => {
                            for stream in [stdout, stderr] {
                                let (tx, rx) = channel();
                                conn.streams.lock().insert(*stream, tx);
                                attachments.streams.push((*stream, rx));
                            }
                            let (tx, rx) = channel();
                            conn.execs.lock().insert(*exec, tx);
                            attachments.exits.push((*exec, rx));
                        }
                        _ => {}
                    }
                    let _ = sender.send(Reply::Outcome {
                        outcome,
                        attachments,
                    });
                }
                None => {}
            }
        }
        WorkerMessage::Event { event } => {
            if let Event::ExecExit { exec, status } = event {
                if let Some(sender) = conn.execs.lock().remove(&exec) {
                    let _ = sender.send(ExitEvent(status));
                    return;
                }
            }
            if let Some(sender) = conn.events.lock().as_ref() {
                let _ = sender.send(event);
            }
        }
        WorkerMessage::Error { id, error } => match id {
            Some(id) => {
                if let Some(pending) = conn.pending.lock().remove(&id) {
                    let sender = match pending {
                        Pending::Plain(sender) => sender,
                        Pending::Streaming(sender) => sender,
                    };
                    let _ = sender.send(Reply::Protocol(error));
                }
            }
            None => conn.terminate(format!("worker reported a session failure: {error}")),
        },
        WorkerMessage::Bye { reason } => {
            if let Some(waiter) = conn.bye.lock().take() {
                let _ = waiter.send(reason);
            }
            conn.terminate(format!("worker exited ({reason:?})"));
        }
    }
}

fn route_chunk(conn: &Arc<Conn>, chunk: StreamChunk) {
    let sender = conn.streams.lock().get(&chunk.stream).cloned();
    match sender {
        Some(sender) => {
            let last = chunk.last;
            let stream = chunk.stream;
            // A consumer that abandoned its payload leaves the stream
            // registered until its last chunk, so routing stays exact.
            let _ = sender.send(StreamEvent::Chunk(chunk));
            if last {
                conn.streams.lock().remove(&stream);
            }
        }
        None => {
            // A chunk for an unknown stream: the worker violated the
            // session. This is corruption, not data.
            conn.terminate(format!("chunk on unknown stream {}", chunk.stream.0));
        }
    }
}
