//! The bridge between a worker-leased server's blocking streams and the
//! tokio mainloop (0058 WK10). A remote/container server spawned through
//! an admitted worker lease is the worker's supervised exec: its stdout/
//! stderr arrive as bounded stream chunks on the client connection and
//! its stdin rides relayed chunks the other way. The mainloop speaks
//! tokio `AsyncRead`/`AsyncWrite`, so this module adapts the client's
//! blocking payloads with one pump thread per direction and bounded
//! queues between them — no unbounded buffering, and a stalled mainloop
//! backpressures the worker's pump instead of growing memory.
//!
//! Lease semantics are the established ones: the server sees stdin EOF
//! when the mainloop's write half closes (half-close, never revocation);
//! revocation is explicit through [`WorkerLease::settle`], which waits a
//! bounded grace for the exit event before `exec_cancel` (TERM/grace/
//! KILL on the worker side). The terminal status is classified — exit
//! code, signal, or unattested (`Lost`) — never silently mapped.

use std::io;
use std::pin::Pin;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use strop_worker_client::{ExecControl, ExecExit, ExecHandle, ExecStdin, Worker};
use strop_worker_protocol::ExitStatus;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Queue depth between the mainloop and a pump thread (chunks of up to
/// the wire's 32 KiB pump size): bounded on both sides, so neither a
/// flooded server nor a stuck mainloop grows memory without bound.
const QUEUE_CHUNKS: usize = 64;

/// How long [`WorkerLease::settle`] waits for the exit event after the
/// lease is revoked — the worker's own TERM/grace/KILL runs first, so
/// this only bounds the event's delivery, not the teardown.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared wake state between a pump thread and the mainloop's poller.
struct Wake {
    waker: Mutex<Option<Waker>>,
}

impl Wake {
    fn store(&self, waker: &Waker) {
        *self.waker.lock() = Some(waker.clone());
    }
    fn wake(&self) {
        let waker = self.waker.lock().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
/// EOF/error must close the sender *before* waking a waiting poller.
/// Waking first lets it observe an empty-but-still-open queue, park
/// again and miss the final close forever.
fn finish_stdout(sender: SyncSender<Vec<u8>>, wake: &Wake) {
    drop(sender);
    wake.wake();
}

/// The server stdout as an async reader: the pump thread blocks on the
/// client payload and parks chunks in the bounded queue; EOF or a
/// payload error both terminate the stream honestly.
pub(super) struct WorkerStdout {
    receiver: Receiver<Vec<u8>>,
    wake: Arc<Wake>,
    /// The pump's terminal read error, if one ended the stream.
    error: Arc<Mutex<Option<io::Error>>>,
    /// The current chunk and the read offset into it — the wire's own
    /// chunk, never copied.
    current: Vec<u8>,
    offset: usize,
}

impl AsyncRead for WorkerStdout {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.offset == self.current.len() {
            match self.receiver.try_recv() {
                Ok(chunk) => {
                    self.current = chunk;
                    self.offset = 0;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.wake.store(context.waker());
                    // Re-check after storing: a chunk that landed between
                    // the first try and the store must not be missed.
                    match self.receiver.try_recv() {
                        Ok(chunk) => {
                            self.current = chunk;
                            self.offset = 0;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => return Poll::Pending,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
            }
        }
        if self.offset == self.current.len() {
            // The channel is closed: the pump's read error, if any, is
            // the stream's truth; otherwise a clean EOF.
            if let Some(error) = self.error.lock().take() {
                return Poll::Ready(Err(error));
            }
            return Poll::Ready(Ok(()));
        }
        let take = (self.current.len() - self.offset).min(buffer.remaining());
        buffer.put_slice(&self.current[self.offset..self.offset + take]);
        self.offset += take;
        if self.offset == self.current.len() {
            self.current = Vec::new();
            self.offset = 0;
        }
        Poll::Ready(Ok(()))
    }
}

/// One message toward the server's stdin.
enum StdinMsg {
    Bytes(Vec<u8>),
    /// The mainloop flushed: acknowledge once the queued bytes are on
    /// the wire, so a following half-close cannot overtake them.
    Flush(Arc<FlushAck>),
    /// Half-close: EOF to the server, never revocation.
    Close,
}

/// The outcome of queueing one stdin message toward the pump thread.
enum Queued {
    Sent,
    /// The bounded queue is full: the poller parks (waker stored) and
    /// the pump's next drain wakes it.
    Full(StdinMsg),
    /// The pump is gone (wire error or teardown): writes fail honestly.
    Gone,
}

/// A flush acknowledgement: the writer thread sets `done` after the
/// queue drained onto the wire, then wakes the poller.
struct FlushAck {
    done: std::sync::atomic::AtomicBool,
    wake: Wake,
}

/// The server stdin as an async writer over the relayed stream.
pub(super) struct WorkerStdin {
    sender: SyncSender<StdinMsg>,
    wake: Arc<Wake>,
    /// Set when the writer thread hit a wire error: writes fail broken
    /// pipe, never silently accepted.
    broken: Arc<Mutex<Option<String>>>,
    /// A full queue retains this one message across poll_write wakes,
    /// rather than copying the caller's bytes on every retry.
    pending_write: Option<Vec<u8>>,
    /// One pending flush survives repeated polls; otherwise every wake
    /// would enqueue a new acknowledgment and remain Pending forever.
    pending_flush: Option<Arc<FlushAck>>,
    flush_queued: bool,
}

impl WorkerStdin {
    /// Queue one message, parking the poller when the bounded queue is
    /// full. The waker is stored before the re-check so a drain that
    /// lands between the two tries is never missed.
    fn queue(&self, context: &Context<'_>, message: StdinMsg) -> Queued {
        match self.sender.try_send(message) {
            Ok(()) => Queued::Sent,
            Err(std::sync::mpsc::TrySendError::Full(message)) => {
                self.wake.store(context.waker());
                match self.sender.try_send(message) {
                    Ok(()) => Queued::Sent,
                    Err(std::sync::mpsc::TrySendError::Full(message)) => Queued::Full(message),
                    Err(std::sync::mpsc::TrySendError::Disconnected(_)) => Queued::Gone,
                }
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => Queued::Gone,
        }
    }

    fn broken_reason(&self) -> Option<String> {
        self.broken.lock().clone()
    }

    fn gone() -> io::Error {
        io::Error::new(io::ErrorKind::BrokenPipe, "server stdin pump is gone")
    }
}

impl AsyncWrite for WorkerStdin {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if let Some(reason) = this.broken_reason() {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, reason)));
        }
        if buffer.is_empty() && this.pending_write.is_none() {
            return Poll::Ready(Ok(0));
        }
        let bytes = match this.pending_write.take() {
            Some(bytes) => {
                debug_assert_eq!(bytes.len(), buffer.len(), "pending write must be retried");
                bytes
            }
            None => buffer.to_vec(),
        };
        let length = bytes.len();
        match this.queue(context, StdinMsg::Bytes(bytes)) {
            Queued::Sent => Poll::Ready(Ok(length)),
            Queued::Full(StdinMsg::Bytes(bytes)) => {
                this.pending_write = Some(bytes);
                Poll::Pending
            }
            Queued::Full(_) => {
                debug_assert!(false, "poll_write queues only byte messages");
                Poll::Ready(Err(io::Error::other(
                    "stdin queue returned a non-byte write",
                )))
            }
            Queued::Gone => Poll::Ready(Err(Self::gone())),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(reason) = self.broken_reason() {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, reason)));
        }
        let ack = self
            .pending_flush
            .get_or_insert_with(|| {
                Arc::new(FlushAck {
                    done: std::sync::atomic::AtomicBool::new(false),
                    wake: Wake {
                        waker: Mutex::new(None),
                    },
                })
            })
            .clone();
        if !self.flush_queued {
            match self.queue(context, StdinMsg::Flush(ack.clone())) {
                Queued::Sent => self.flush_queued = true,
                Queued::Full(_) => return Poll::Pending,
                Queued::Gone => return Poll::Ready(Err(Self::gone())),
            }
        }
        ack.wake.store(context.waker());
        if ack.done.load(std::sync::atomic::Ordering::Acquire) {
            self.pending_flush = None;
            self.flush_queued = false;
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        // A full queue must not lose the EOF: park until the pump drains
        // and admits the close. `Gone` means the pump already delivered
        // its own EOF on teardown — shutdown is complete.
        match self.queue(context, StdinMsg::Close) {
            Queued::Sent | Queued::Gone => Poll::Ready(Ok(())),
            Queued::Full(_) => Poll::Pending,
        }
    }
}

impl Drop for WorkerStdin {
    /// Dropping the write half without an explicit shutdown still
    /// delivers EOF: the pump sees the channel close and half-closes
    /// the relayed stdin. Never a revocation.
    fn drop(&mut self) {
        let _ = self.sender.try_send(StdinMsg::Close);
    }
}

/// The lease on the worker-owned server process: identity plus the
/// cancel path. `settle` is the teardown the runtime thread runs once
/// the mainloop has ended — the same ordering the supervised ssh/docker
/// launches had (stdin EOF, grace, then revoke).
pub(super) struct WorkerLease {
    control: ExecControl,
    status: Arc<Mutex<Option<ExitStatus>>>,
}

impl WorkerLease {
    /// The classified terminal status, if the server has settled.
    pub(super) fn status(&self) -> Option<ExitStatus> {
        *self.status.lock()
    }

    /// Wait `grace` for the server to follow its stdin EOF out; if it
    /// has not exited, revoke the lease (`exec_cancel`: TERM/grace/KILL
    /// on the worker side) and boundedly await the exit event the
    /// teardown publishes. Runs on the runtime thread after the
    /// mainloop ended — blocking there stalls nothing else.
    pub(super) fn settle(&self, grace: Duration) {
        let deadline = Instant::now() + grace;
        while self.status().is_none() && Instant::now() < deadline {
            std::thread::park_timeout(Duration::from_millis(20));
        }
        if self.status().is_some() {
            return;
        }
        let (token, _handle) = strop_core::worker::CancelToken::standalone();
        let _ = self.control.cancel(&token);
        let deadline = Instant::now() + SETTLE_TIMEOUT;
        while self.status().is_none() && Instant::now() < deadline {
            std::thread::park_timeout(Duration::from_millis(20));
        }
    }
}

/// Everything a worker-leased spawn wires into the runtime: the async
/// stdio pair for the mainloop, the bounded stderr tail for failure
/// hints, the lease for teardown, and the classified exit status.
pub(super) struct WorkerIo {
    pub stdout: WorkerStdout,
    pub stdin: WorkerStdin,
    pub stderr_tail: Arc<parking_lot::Mutex<Vec<u8>>>,
    pub lease: WorkerLease,
}

/// Start the pump threads for one admitted exec. The handle is
/// decomposed: stdout and stdin ride bounded queues with waker
/// handoff, stderr drains into the shared tail (the same 8 KiB cap the
/// supervised launches keep), and the exit waiter records the
/// classified status. Every thread exits with its channel — a dead
/// mainloop never strands a pump.
pub(super) fn start(handle: ExecHandle, worker: Worker, tail_cap: usize) -> WorkerIo {
    let (exec, stdin, stdout, stderr, exit) = handle.into_parts();
    let control = stdin.control(worker, exec);

    let (stdout_sender, stdout_receiver) = std::sync::mpsc::sync_channel(QUEUE_CHUNKS);
    let stdout_wake = Arc::new(Wake {
        waker: Mutex::new(None),
    });
    let stdout_error = Arc::new(Mutex::new(None));
    {
        let wake = stdout_wake.clone();
        let error = stdout_error.clone();
        std::thread::spawn(move || {
            let mut stdout = stdout;
            let mut buffer = [0_u8; 32 * 1024];
            loop {
                match std::io::Read::read(&mut stdout, &mut buffer) {
                    Ok(0) => {
                        finish_stdout(stdout_sender, &wake);
                        return;
                    }
                    Ok(read) => {
                        if stdout_sender.send(buffer[..read].to_vec()).is_err() {
                            return; // mainloop gone: the lease's settle owns teardown
                        }
                        wake.wake();
                    }
                    Err(failure) => {
                        *error.lock() = Some(failure);
                        finish_stdout(stdout_sender, &wake);
                        return;
                    }
                }
            }
        });
    }

    let (stdin_sender, stdin_receiver) = std::sync::mpsc::sync_channel::<StdinMsg>(QUEUE_CHUNKS);
    let stdin_wake = Arc::new(Wake {
        waker: Mutex::new(None),
    });
    let stdin_broken = Arc::new(Mutex::new(None));
    {
        let wake = stdin_wake.clone();
        let broken = stdin_broken.clone();
        std::thread::spawn(move || {
            let mut stdin: ExecStdin = stdin;
            loop {
                match stdin_receiver.recv() {
                    Ok(StdinMsg::Bytes(bytes)) => {
                        if let Err(failure) = stdin.write(&bytes, false) {
                            *broken.lock() = Some(failure.to_string());
                            return;
                        }
                    }
                    Ok(StdinMsg::Flush(ack)) => {
                        ack.done.store(true, std::sync::atomic::Ordering::Release);
                        ack.wake.wake();
                    }
                    Ok(StdinMsg::Close) | Err(_) => {
                        // Explicit half-close or every sender dropped:
                        // EOF to the server, then the pump retires.
                        let _ = stdin.write(&[], true);
                        return;
                    }
                }
                wake.wake();
            }
        });
    }

    let stderr_tail = Arc::new(parking_lot::Mutex::new(Vec::new()));
    {
        let tail = stderr_tail.clone();
        std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut buffer = [0_u8; 4096];
            loop {
                match std::io::Read::read(&mut stderr, &mut buffer) {
                    Ok(0) => return,
                    Ok(read) => {
                        let mut guard = tail.lock();
                        guard.extend_from_slice(&buffer[..read]);
                        let overflow = guard.len().saturating_sub(tail_cap);
                        if overflow > 0 {
                            guard.drain(..overflow);
                        }
                    }
                    Err(_) => return,
                }
            }
        });
    }

    let status = Arc::new(Mutex::new(None));
    {
        let slot = status.clone();
        std::thread::spawn(move || {
            let exit: ExecExit = exit;
            *slot.lock() = Some(exit.wait());
        });
    }

    WorkerIo {
        stdout: WorkerStdout {
            receiver: stdout_receiver,
            wake: stdout_wake,
            error: stdout_error,
            current: Vec::new(),
            offset: 0,
        },
        stdin: WorkerStdin {
            sender: stdin_sender,
            wake: stdin_wake,
            broken: stdin_broken,
            pending_write: None,
            pending_flush: None,
            flush_queued: false,
        },
        stderr_tail,
        lease: WorkerLease { control, status },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_stdin_queue_retries_one_owned_write_without_duplication() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        sender.send(StdinMsg::Bytes(b"earlier".to_vec())).unwrap();
        let mut stdin = WorkerStdin {
            sender,
            wake: Arc::new(Wake {
                waker: Mutex::new(None),
            }),
            broken: Arc::new(Mutex::new(None)),
            pending_write: None,
            pending_flush: None,
            flush_queued: false,
        };
        let mut context = Context::from_waker(std::task::Waker::noop());
        for _ in 0..2 {
            assert!(matches!(
                Pin::new(&mut stdin).poll_write(&mut context, b"retry-me"),
                Poll::Pending
            ));
        }
        let StdinMsg::Bytes(first) = receiver.recv().unwrap() else {
            panic!("queued bytes were not first");
        };
        assert_eq!(first, b"earlier");
        assert!(matches!(
            Pin::new(&mut stdin).poll_write(&mut context, b"retry-me"),
            Poll::Ready(Ok(8))
        ));
        let StdinMsg::Bytes(second) = receiver.recv().unwrap() else {
            panic!("the retry did not carry bytes");
        };
        assert_eq!(second, b"retry-me");
        assert!(receiver.try_recv().is_err(), "a pending write ran twice");
    }

    #[test]
    fn closing_stdout_wakes_a_parked_reader_after_eof_is_visible() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::task::{Wake as TaskWake, Waker};

        struct OnWake {
            reader: Mutex<WorkerStdout>,
            eof: AtomicBool,
        }
        impl TaskWake for OnWake {
            fn wake(self: Arc<Self>) {
                self.wake_by_ref();
            }
            fn wake_by_ref(self: &Arc<Self>) {
                let mut reader = self.reader.lock();
                let mut byte = [0_u8; 1];
                let mut buffer = ReadBuf::new(&mut byte);
                let context = &mut Context::from_waker(Waker::noop());
                if let Poll::Ready(Ok(())) = Pin::new(&mut *reader).poll_read(context, &mut buffer)
                {
                    self.eof
                        .store(buffer.filled().is_empty(), Ordering::Release);
                }
            }
        }

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let wake = Arc::new(Wake {
            waker: Mutex::new(None),
        });
        let listener = Arc::new(OnWake {
            reader: Mutex::new(WorkerStdout {
                receiver,
                wake: wake.clone(),
                error: Arc::new(Mutex::new(None)),
                current: Vec::new(),
                offset: 0,
            }),
            eof: AtomicBool::new(false),
        });
        let waker = Waker::from(listener.clone());
        let mut byte = [0_u8; 1];
        let mut buffer = ReadBuf::new(&mut byte);
        let mut context = Context::from_waker(&waker);
        assert!(matches!(
            Pin::new(&mut *listener.reader.lock()).poll_read(&mut context, &mut buffer),
            Poll::Pending
        ));
        finish_stdout(sender, &wake);
        assert!(
            listener.eof.load(Ordering::Acquire),
            "EOF had no final wake"
        );
    }
}
