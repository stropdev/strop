//! Pull-side payload readers over inbound streams (0058 WK04).
//!
//! A [`ReadPayload`] turns one stream's ordered chunks into an
//! [`io::Read`]. Integrity rule: when the worker announced a size, a
//! stream that ends short of it is an error — never silent truncation
//! (the wire has no stream-error frame; the announced size is the
//! anchor). Cancellation mid-payload surfaces the same way, since the
//! worker ends the stream without its announced bytes.

use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use strop_core::worker::CancelToken;
use strop_worker_protocol::{ClientMessage, RequestId, StreamId};

use crate::connection::{Conn, StreamEvent};

/// Clears the caller's cancel hook when the payload completes or is
/// dropped. Engine callers use one token strictly sequentially
/// (observe → read), so clearing at payload end never disturbs a later
/// request's hook on the same token.
pub(crate) struct CancelGuard(pub(crate) CancelToken);

impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.0.clear_cancel_resource();
    }
}

/// Credits belong to the exact worker incarnation that minted a
/// finite file or process-output stream. Only file reads have a
/// request to cancel on early drop; exec revocation is explicit.
struct FlowControl {
    conn: Arc<Conn>,
    stream: StreamId,
    request: Option<RequestId>,
}

/// One inbound payload as a blocking reader.
pub struct ReadPayload {
    receiver: Receiver<StreamEvent>,
    announced: Option<u64>,
    received: u64,
    current: Vec<u8>,
    offset: usize,
    finished: bool,
    /// True only after the wire's `last` marker, not after a local
    /// cancellation/read error that still leaves a producer alive.
    stream_ended: bool,
    guard: Option<CancelGuard>,
    flow: Option<FlowControl>,
    /// Finite/process output waits re-check cancellation off the input
    /// path so a quiet child cannot strand a superseded search or Git job.
    cancel: Option<CancelToken>,
}

impl ReadPayload {
    pub(crate) fn new(
        receiver: Receiver<StreamEvent>,
        announced: Option<u64>,
        cancel: Option<CancelToken>,
    ) -> Self {
        Self {
            receiver,
            announced,
            received: 0,
            current: Vec::new(),
            offset: 0,
            finished: false,
            stream_ended: false,
            guard: None,
            flow: None,
            cancel,
        }
    }

    /// A payload whose stream may outlive its request: the caller's
    /// cancel hook stays registered until the payload completes or is
    /// dropped, so a mid-stream cancellation still reaches the worker.
    pub(crate) fn streaming(
        receiver: Receiver<StreamEvent>,
        announced: Option<u64>,
        guard: CancelGuard,
        conn: Arc<Conn>,
        stream: StreamId,
        request: RequestId,
    ) -> Self {
        let mut payload = Self::new(receiver, announced, None);
        payload.guard = Some(guard);
        payload.flow = Some(FlowControl {
            conn,
            stream,
            request: Some(request),
        });
        payload
    }

    pub(crate) fn exec(
        receiver: Receiver<StreamEvent>,
        token: CancelToken,
        conn: Arc<Conn>,
        stream: StreamId,
    ) -> Self {
        let mut payload = Self::new(receiver, None, Some(token));
        payload.flow = Some(FlowControl {
            conn,
            stream,
            request: None,
        });
        payload
    }
}

impl io::Read for ReadPayload {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        loop {
            if self.offset == self.current.len() && !self.current.is_empty() {
                self.current.clear();
                self.offset = 0;
                if !self.finished {
                    if let Some(flow) = &self.flow {
                        flow.conn.write_quiet(&ClientMessage::StreamCredit {
                            session: flow.conn.session(),
                            stream: flow.stream,
                            chunks: 1,
                        });
                    }
                }
            }
            if self.offset < self.current.len() {
                let count = (self.current.len() - self.offset).min(target.len());
                target[..count].copy_from_slice(&self.current[self.offset..self.offset + count]);
                self.offset += count;
                return Ok(count);
            }
            if self.finished {
                return Ok(0);
            }
            let event = if let Some(cancel) = &self.cancel {
                loop {
                    if cancel.is_cancelled() {
                        self.finished = true;
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "worker payload cancelled",
                        ));
                    }
                    match self.receiver.recv_timeout(Duration::from_millis(25)) {
                        Ok(event) => break Ok(event),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break Err(()),
                    }
                }
            } else {
                self.receiver.recv().map_err(|_| ())
            };
            match event {
                Ok(StreamEvent::Chunk(chunk)) => {
                    self.received += chunk.bytes.len() as u64;
                    if chunk.last {
                        self.finished = true;
                        self.stream_ended = true;
                        self.guard = None;
                        if let Some(announced) = self.announced {
                            let total = self.received;
                            if total != announced {
                                return Err(io::Error::new(
                                    io::ErrorKind::UnexpectedEof,
                                    format!(
                                        "worker ended the payload at {total} of {announced} announced bytes"
                                    ),
                                ));
                            }
                        }
                    }
                    self.current = chunk.bytes;
                    self.offset = 0;
                }
                // A PTY geometry boundary is not payload data (WK12);
                // only the terminal's own parser consumes it.
                Ok(StreamEvent::Resized) => {}
                Ok(StreamEvent::Failed(reason)) => {
                    self.finished = true;
                    self.guard = None;
                    return Err(io::Error::new(io::ErrorKind::ConnectionAborted, reason));
                }
                Err(_) => {
                    self.finished = true;
                    self.guard = None;
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "payload stream closed before its last chunk",
                    ));
                }
            }
        }
    }
}

impl Drop for ReadPayload {
    fn drop(&mut self) {
        if !self.stream_ended {
            if let Some(flow) = &self.flow {
                let session = flow.conn.session();
                match flow.request {
                    Some(id) => {
                        flow.conn
                            .write_quiet(&ClientMessage::Cancel { session, id });
                    }
                    None => {
                        // Dropping exec output never kills its child:
                        // drain and discard so a full credit window
                        // cannot strand that child's settlement.
                        flow.conn.write_quiet(&ClientMessage::StreamAbandon {
                            session,
                            stream: flow.stream,
                        });
                    }
                }
            }
        }
    }
}
