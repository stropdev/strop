//! Admitted process and PTY session handles; dropping a handle does not
//! terminate its child, but releases the caller's cancellation registration.

use std::sync::{
    mpsc::{Receiver, RecvTimeoutError, TryRecvError},
    Arc,
};
use std::time::Duration;

use strop_core::worker::CancelToken;
use strop_worker_protocol::{
    ClientMessage, ExecId, ExitStatus, PtyGeometry, StreamChunk, StreamId,
};

use crate::connection::{self, Conn};
use crate::payload::CancelGuard;
use crate::{ClientError, ExecEvent, ReadPayload, StreamEvent, Worker};

/// One admitted exec's client-side handle. Dropping the handle never
/// kills the process; cancellation is explicit through
/// [`ExecControl::cancel`], and the exit event arrives regardless.
pub struct ExecHandle {
    pub(super) id: ExecId,
    pub(super) conn: Arc<Conn>,
    pub(super) stdin: Option<strop_worker_protocol::StreamId>,
    pub(super) stdin_sequence: u64,
    pub(super) stdout: ReadPayload,
    pub(super) stderr: ReadPayload,
    pub(super) exit: Receiver<connection::ExecEvent>,
    pub(super) cancel: CancelGuard,
}

impl std::fmt::Debug for ExecHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
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
        loop {
            match self.exit.recv() {
                Ok(connection::ExecEvent::Exit(status)) => return status,
                Ok(connection::ExecEvent::Input { .. }) => {}
                Err(_) => return ExitStatus::Lost,
            }
        }
    }
}

/// An exec's relayed stdin after [`ExecHandle::into_parts`]: the wire
/// writer plus its sequence counter, ownable by one pump thread.
pub struct ExecStdin {
    conn: Arc<Conn>,
    stream: Option<strop_worker_protocol::StreamId>,
    sequence: u64,
}

impl ExecStdin {
    /// `true` when this exec admitted a relayed stdin (leased services).
    pub fn is_relayed(&self) -> bool {
        self.stream.is_some()
    }

    /// Write one chunk; `last` delivers EOF, not revocation.
    pub fn write(&mut self, bytes: &[u8], last: bool) -> Result<(), ClientError> {
        let Some(stream) = self.stream else {
            return Err(ClientError::Refused(
                strop_worker_protocol::Refusal::UnknownHandle {
                    message: "exec has no relayed stdin".into(),
                },
            ));
        };
        let sequence = self.sequence;
        self.sequence += 1;
        self.conn.write_chunk(&StreamChunk {
            stream,
            sequence,
            last,
            bytes: bytes.to_vec(),
        })
    }

    /// Retain cancellation authority for this exact exec incarnation
    /// while the stdin writer moves to its own pump thread.
    pub fn control(&self, worker: Worker, exec: ExecId) -> ExecControl {
        ExecControl {
            worker,
            conn: Arc::clone(&self.conn),
            exec,
        }
    }
}

/// Cancellation authority bound to the connection that admitted an
/// exec. A replacement worker may reuse an exec id but never inherits
/// this lease's authority.
pub struct ExecControl {
    worker: Worker,
    conn: Arc<Conn>,
    exec: ExecId,
}

impl ExecControl {
    pub fn cancel(&self, token: &CancelToken) -> Result<(), ClientError> {
        self.worker
            .exec_cancel_on(token, self.exec, Arc::clone(&self.conn))
    }
}

/// An exec's terminal-status waiter after [`ExecHandle::into_parts`].
pub struct ExecExit {
    events: Receiver<connection::ExecEvent>,
    _cancel: CancelGuard,
}

impl ExecExit {
    /// Wait for the terminal status. `Lost` means the supervisor could
    /// not attest the exit — it is never silently mapped to a code.
    pub fn wait(self) -> ExitStatus {
        loop {
            match self.events.recv() {
                Ok(connection::ExecEvent::Exit(status)) => return status,
                Ok(connection::ExecEvent::Input { .. }) => {}
                Err(_) => return ExitStatus::Lost,
            }
        }
    }
}

impl ExecHandle {
    /// Decompose the handle for consumers that pump each stream on its
    /// own thread (0058 WK10: LSP-class leased services, bounded git
    /// captures): the exec identity, the relayed stdin writer, the two
    /// output payloads and the exit waiter. Same lease semantics —
    /// dropping never kills the process; cancellation stays explicit
    /// through the session-bound [`ExecStdin::control`].
    pub fn into_parts(self) -> (ExecId, ExecStdin, ReadPayload, ReadPayload, ExecExit) {
        (
            self.id,
            ExecStdin {
                conn: self.conn,
                stream: self.stdin,
                sequence: self.stdin_sequence,
            },
            self.stdout,
            self.stderr,
            ExecExit {
                events: self.exit,
                _cancel: self.cancel,
            },
        )
    }
}

/// One admitted PTY session (0058 WK12): the client half of the
/// worker-owned terminal. Input chunks are acknowledged
/// ([`ExecEvent::Input`]) in delivery order; output and the ordered
/// resize boundaries arrive on one receiver in wire order; the exit is
/// the supervisor's attested status. Holding the session holds the
/// worker lease; dropping never kills the child — termination is
/// explicit through [`PtySession::terminate`].
pub struct PtySession {
    pub(super) worker: Worker,
    pub(super) conn: Arc<Conn>,
    pub(super) id: ExecId,
    pub(super) stdin: StreamId,
    pub(super) stdin_sequence: u64,
    pub(super) output_stream: StreamId,
    pub(super) output: Receiver<StreamEvent>,
    pub(super) events: Receiver<ExecEvent>,
    pub(super) control: CancelToken,
    /// Keeps the standalone control token alive (a dropped handle
    /// cancels).
    pub(super) _lease: strop_core::worker::CancelHandle,
    pub(super) _spawn_cancel: CancelGuard,
}

impl PtySession {
    pub fn id(&self) -> ExecId {
        self.id
    }

    /// The output stream's id (wake-registration key for fd-polling
    /// consumers).
    pub fn output_stream(&self) -> StreamId {
        self.output_stream
    }

    /// Feed input: one ordered chunk, acknowledged once delivered.
    /// Never half-closes — a PTY has no input EOF.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        let conn = &self.conn;
        let sequence = self.stdin_sequence;
        self.stdin_sequence += 1;
        conn.write_chunk(&StreamChunk {
            stream: self.stdin,
            sequence,
            last: false,
            bytes: bytes.to_vec(),
        })
    }

    /// Consume one output chunk or ordered resize marker. Credits
    /// return only as the receiver drains; a stalled terminal parks
    /// its worker pump instead of losing VT bytes at a full client
    /// queue.
    pub fn try_output(&self) -> Result<StreamEvent, TryRecvError> {
        self.output.try_recv().map(|event| self.consumed(event))
    }

    /// Bounded blocking receive for non-rendering consumers and tests.
    pub fn recv_output_timeout(&self, timeout: Duration) -> Result<StreamEvent, RecvTimeoutError> {
        self.output
            .recv_timeout(timeout)
            .map(|event| self.consumed(event))
    }

    fn consumed(&self, event: StreamEvent) -> StreamEvent {
        if let StreamEvent::Chunk(chunk) = &event {
            if !chunk.last {
                self.conn.write_quiet(&ClientMessage::StreamCredit {
                    session: self.conn.session(),
                    stream: self.output_stream,
                    chunks: 1,
                });
            }
        }
        event
    }

    /// The lifecycle receiver: delivered-input acknowledgments and the
    /// terminal status.
    pub fn events(&self) -> &Receiver<ExecEvent> {
        &self.events
    }

    /// Apply a new geometry. The reply means the ioctl landed in input
    /// order; the parser boundary arrives as [`StreamEvent::Resized`]
    /// on the output receiver.
    pub fn resize(&self, geometry: PtyGeometry) -> Result<(), ClientError> {
        self.worker
            .exec_resize_on(&self.control, self.id, geometry, Arc::clone(&self.conn))
    }

    /// Whether the session's control token was cancelled (the terminal
    /// client's close detection; 0058 WK12).
    pub fn token_cancelled(&self) -> bool {
        self.control.is_cancelled()
    }

    /// Revoke the session's lease (TERM/grace/KILL, then the exit
    /// event). Explicit, never implied by a drop.
    pub fn terminate(&self) -> Result<(), ClientError> {
        self.worker
            .exec_cancel_on(&self.control, self.id, Arc::clone(&self.conn))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.conn.write_quiet(&ClientMessage::StreamAbandon {
            session: self.conn.session(),
            stream: self.output_stream,
        });
    }
}
