//! Pull-side payload readers over inbound streams (0058 WK04).
//!
//! A [`ReadPayload`] turns one stream's ordered chunks into an
//! [`io::Read`]. Integrity rule: when the worker announced a size, a
//! stream that ends short of it is an error — never silent truncation
//! (the wire has no stream-error frame; the announced size is the
//! anchor). Cancellation mid-payload surfaces the same way, since the
//! worker ends the stream without its announced bytes.

use std::io;
use std::sync::mpsc::Receiver;

use strop_core::worker::CancelToken;

use crate::connection::StreamEvent;

/// Clears the caller's cancel hook when the payload completes or is
/// dropped. Engine callers use one token strictly sequentially
/// (observe → read), so clearing at payload end never disturbs a later
/// request's hook on the same token.
struct CancelGuard(CancelToken);

impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.0.clear_cancel_resource();
    }
}

/// One inbound payload as a blocking reader.
pub struct ReadPayload {
    receiver: Receiver<StreamEvent>,
    announced: Option<u64>,
    received: u64,
    current: Vec<u8>,
    offset: usize,
    finished: bool,
    guard: Option<CancelGuard>,
}

impl ReadPayload {
    pub(crate) fn new(receiver: Receiver<StreamEvent>, announced: Option<u64>) -> Self {
        Self {
            receiver,
            announced,
            received: 0,
            current: Vec::new(),
            offset: 0,
            finished: false,
            guard: None,
        }
    }

    /// A payload whose stream may outlive its request: the caller's
    /// cancel hook stays registered until the payload completes or is
    /// dropped, so a mid-stream cancellation still reaches the worker.
    pub(crate) fn streaming(
        receiver: Receiver<StreamEvent>,
        announced: Option<u64>,
        token: CancelToken,
    ) -> Self {
        let mut payload = Self::new(receiver, announced);
        payload.guard = Some(CancelGuard(token));
        payload
    }
}

impl io::Read for ReadPayload {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.offset < self.current.len() {
                let count = (self.current.len() - self.offset).min(target.len());
                target[..count].copy_from_slice(&self.current[self.offset..self.offset + count]);
                self.offset += count;
                return Ok(count);
            }
            if self.finished {
                return Ok(0);
            }
            match self.receiver.recv() {
                Ok(StreamEvent::Chunk(chunk)) => {
                    self.received += chunk.bytes.len() as u64;
                    if chunk.last {
                        self.finished = true;
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
