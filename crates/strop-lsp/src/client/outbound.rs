//! Physical framing receipts. A notification leaves the bounded ordered queue
//! only after the actual writer flushes it, not after async-lsp queues a Value.
mod prefix;
#[cfg(test)]
mod tests;

use crate::RequestStamp;
use async_lsp::RequestId as RpcId;
use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Method {
    Open,
    Change,
    Close,
    Completion,
    Resolve,
    Other,
}
impl Method {
    fn parse(method: &str) -> Self {
        match method {
            "textDocument/didOpen" => Self::Open,
            "textDocument/didChange" => Self::Change,
            "textDocument/didClose" => Self::Close,
            "textDocument/completion" => Self::Completion,
            "completionItem/resolve" => Self::Resolve,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum WriteRefusal {
    #[error("language-service connection is closed")]
    Closed,
    #[error("language-service write identity space exhausted")]
    IdentityExhausted,
    #[error("another ordered language-service notification still owns its frame")]
    Pending,
    #[error("language-service completion frame mapping is full")]
    Busy,
}

#[derive(Default)]
enum Notification {
    #[default]
    Idle,
    Queued {
        ticket: u64,
        method: Method,
    },
    Writing {
        ticket: u64,
    },
}
struct Submission {
    stamp: RequestStamp,
    method: Method,
    identity: oneshot::Sender<RpcId>,
}
#[derive(Default)]
struct State {
    notification: Notification,
    next: u64,
    written: u64,
    closed: bool,
    requests: VecDeque<Submission>,
    close_wake: Option<Box<dyn FnOnce() + Send>>,
    retirement_requested: bool,
}
#[derive(Default)]
pub(super) struct Outbound {
    state: Mutex<State>,
    settled: Condvar,
}
impl Outbound {
    pub fn set_close_wake(&self, wake: impl FnOnce() + Send + 'static) -> Result<(), WriteRefusal> {
        let mut state = self.state.lock();
        if state.close_wake.is_some() {
            return Err(WriteRefusal::Pending);
        }
        if state.closed {
            drop(state);
            wake();
        } else {
            state.close_wake = Some(Box::new(wake));
        }
        Ok(())
    }

    pub fn begin_notification(&self, method: Method) -> Result<u64, WriteRefusal> {
        debug_assert!(matches!(
            method,
            Method::Open | Method::Change | Method::Close
        ));
        let mut state = self.state.lock();
        if state.closed {
            return Err(WriteRefusal::Closed);
        }
        if !matches!(state.notification, Notification::Idle) {
            return Err(WriteRefusal::Pending);
        }
        let ticket = state
            .next
            .checked_add(1)
            .ok_or(WriteRefusal::IdentityExhausted)?;
        state.next = ticket;
        state.notification = Notification::Queued { ticket, method };
        Ok(ticket)
    }

    /// Wire-worker wait only. The input side still enqueues/coalesces under the
    /// existing count/byte bounds, and never waits for native pipe progress.
    pub fn wait(&self, ticket: u64, mut retire: impl FnMut()) -> bool {
        let mut state = self.state.lock();
        while !state.closed
            && matches!(state.notification,
            Notification::Queued { ticket: owner, .. } | Notification::Writing { ticket: owner }
                if owner == ticket)
        {
            if state.retirement_requested {
                state.retirement_requested = false;
                drop(state);
                retire();
                state = self.state.lock();
            } else {
                self.settled.wait(&mut state);
            }
        }
        state.written == ticket
    }

    pub fn wake_retirement(&self) {
        self.state.lock().retirement_requested = true;
        self.settled.notify_all();
    }

    pub fn abort(&self, ticket: u64) {
        let mut state = self.state.lock();
        if matches!(state.notification,
            Notification::Queued { ticket: owner, .. } | Notification::Writing { ticket: owner }
                if owner == ticket)
        {
            state.notification = Notification::Idle;
        }
        drop(state);
        self.settled.notify_all();
    }

    pub fn submitted(
        &self,
        stamp: RequestStamp,
        method: Method,
    ) -> Result<oneshot::Receiver<RpcId>, WriteRefusal> {
        debug_assert!(matches!(method, Method::Completion | Method::Resolve));
        let mut state = self.state.lock();
        if state.closed {
            return Err(WriteRefusal::Closed);
        }
        if state.requests.len() == 2 {
            return Err(WriteRefusal::Busy);
        }
        let (identity, receiver) = oneshot::channel();
        state.requests.push_back(Submission {
            stamp,
            method,
            identity,
        });
        Ok(receiver)
    }

    pub fn forget_request(&self, stamp: RequestStamp) {
        let mut state = self.state.lock();
        let removed = state
            .requests
            .iter()
            .position(|request| request.stamp == stamp)
            .and_then(|index| state.requests.remove(index));
        drop(state);
        drop(removed);
    }

    fn header(&self, head: prefix::Head) -> io::Result<Option<u64>> {
        let mut state = self.state.lock();
        if matches!(head.method, Method::Completion | Method::Resolve) {
            let id = head
                .id
                .ok_or_else(|| invalid("completion frame has no JSON-RPC request identity"))?;
            if state
                .requests
                .front()
                .is_none_or(|request| request.method != head.method)
            {
                return Err(invalid(
                    "completion frame does not match its submitted request owner",
                ));
            }
            let submission = state.requests.pop_front();
            drop(state);
            if let Some(submission) = submission {
                let _ = submission.identity.send(id);
            }
            return Ok(None);
        }
        let ticket = match state.notification {
            Notification::Queued { ticket, method } if method == head.method => Some(ticket),
            _ => None,
        };
        if let Some(ticket) = ticket {
            state.notification = Notification::Writing { ticket };
        }
        Ok(ticket)
    }

    fn flushed(&self, ticket: u64) {
        let mut state = self.state.lock();
        if matches!(state.notification, Notification::Writing { ticket: owner } if owner == ticket)
        {
            state.written = ticket;
            state.notification = Notification::Idle;
        }
        drop(state);
        self.settled.notify_all();
    }

    pub fn close(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        state.notification = Notification::Idle;
        let requests = std::mem::take(&mut state.requests);
        let wake = state.close_wake.take();
        drop(state);
        drop(requests);
        if let Some(wake) = wake {
            wake();
        }
        self.settled.notify_all();
    }
}

pub(super) struct FramedWriter<T> {
    inner: T,
    outbound: Arc<Outbound>,
    frames: Frames,
}
impl<T> FramedWriter<T> {
    pub fn new(inner: T, outbound: Arc<Outbound>) -> Self {
        Self {
            inner,
            outbound,
            frames: Frames::default(),
        }
    }
}
impl<T> Drop for FramedWriter<T> {
    fn drop(&mut self) {
        self.outbound.close();
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for FramedWriter<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write(context, bytes) {
            Poll::Ready(Ok(written)) => {
                this.frames.push(&bytes[..written], &this.outbound)?;
                Poll::Ready(Ok(written))
            }
            result => result,
        }
    }
    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_flush(context) {
            Poll::Ready(Ok(())) => {
                if this.frames.remaining == 0 {
                    if let Some(ticket) = this.frames.finished.take() {
                        this.outbound.flushed(ticket);
                    }
                }
                Poll::Ready(Ok(()))
            }
            result => result,
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

#[derive(Default)]
struct Frames {
    header: Vec<u8>,
    remaining: usize,
    prefix: Vec<u8>,
    identified: bool,
    writing: Option<u64>,
    finished: Option<u64>,
}
impl Frames {
    fn push(&mut self, mut bytes: &[u8], outbound: &Outbound) -> io::Result<()> {
        while !bytes.is_empty() {
            if self.remaining == 0 {
                if self.header.len() == super::frame_limits::MAX_HEADER_BYTES {
                    return Err(invalid(
                        "outgoing language-service header exceeds its bound",
                    ));
                }
                self.header.push(bytes[0]);
                bytes = &bytes[1..];
                if self.header.ends_with(b"\r\n\r\n") {
                    self.remaining = super::frame_limits::content_length(&self.header)?;
                    self.header.clear();
                    self.prefix.clear();
                    self.identified = false;
                    self.writing = None;
                }
            } else {
                let take = bytes.len().min(self.remaining);
                if !self.identified {
                    let capture = take.min(prefix::MAX_PREFIX_BYTES - self.prefix.len());
                    self.prefix.extend_from_slice(&bytes[..capture]);
                    if let Some(head) = prefix::decode(&self.prefix)? {
                        self.writing = outbound.header(head)?;
                        self.identified = true;
                    } else if self.prefix.len() == prefix::MAX_PREFIX_BYTES
                        || take == self.remaining
                    {
                        return Err(invalid(
                            "outgoing JSON-RPC identity is not in its bounded prefix",
                        ));
                    }
                }
                self.remaining -= take;
                bytes = &bytes[take..];
                if self.remaining == 0 {
                    if let Some(ticket) = self.writing.take() {
                        debug_assert!(
                            self.finished.is_none(),
                            "one notification owns the ordered wire"
                        );
                        self.finished = Some(ticket);
                    }
                }
            }
        }
        Ok(())
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
