//! In-process PTY for local sessions (0090 §4). The child is launched
//! and supervised by `strop-worker`'s own PTY supervisor — the same
//! session setup, readiness handshake and revoke-before-reap teardown
//! the remote worker uses — but the master stays in the editor process,
//! so no worker round trip sits between a keystroke and the shell.
//!
//! The async invariant (0001 §3) holds by construction: the editor
//! thread only enqueues intents on the service; this transport runs on
//! the service's actor thread plus two helpers. The master is
//! non-blocking, so input a child stops reading is retained here and
//! flushed as `poll` reports room — the actor never parks on a write
//! while output still arrives. The reader thread blocks on `poll`/`read`
//! only and parks on the bounded output channel when the actor is
//! behind: the PTY's own buffer then applies backpressure to the child,
//! never a dropped byte. The waiter thread owns the supervisor lease and
//! reports the truthful exit; cancelling the lease is the revoke path.
use crate::{launch::Launch, model::Geometry, Error};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Write},
    os::{fd::AsRawFd, unix::net::UnixDatagram},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
};
use strop_core::worker::{CancelHandle, CancelReason, CancelToken};
use strop_worker::exec::{launch_pty, resize_pty, ExecSpec, PtySize, Settlement, StatusRecord};
use strop_worker_client::{ExecEvent, StreamEvent};
use strop_worker_protocol::{ExitStatus, StreamChunk};

/// Output chunks queued ahead of the actor before the reader parks.
const OUTPUT_QUEUE: usize = 64;
/// One `read` from the master.
const READ_CHUNK: usize = 64 * 1024;

pub(crate) struct LocalPty {
    master: File,
    /// Cancelling revokes the lease (TERM, grace, KILL, reap) on the
    /// waiter thread; dropping the handle does the same.
    lease: Option<CancelHandle>,
    output: Receiver<StreamEvent>,
    events: Receiver<ExecEvent>,
    events_tx: SyncSender<ExecEvent>,
    /// Admitted input not yet written to the master.
    pending: VecDeque<u8>,
    /// Wire sequences awaiting delivery: `(sequence, written-bytes mark)`.
    acks: VecDeque<(u64, u64)>,
    written: u64,
    sequence: u64,
}

fn io_error(operation: &'static str, error: io::Error) -> Error {
    Error::Io {
        operation,
        detail: error.to_string(),
    }
}

fn nudge(wake: &UnixDatagram) {
    // A full datagram buffer means the service is already awake.
    let _ = wake.send(&[1]);
}

impl LocalPty {
    /// Spawn `launch` on a fresh PTY with `geometry`; `wake` is nudged on
    /// every output arrival and on exit, so the service never polls.
    pub(crate) fn spawn(
        launch: &Launch,
        geometry: Geometry,
        wake: UnixDatagram,
    ) -> Result<Self, Error> {
        use std::os::unix::ffi::OsStrExt;
        let spec = ExecSpec::new(
            &launch.program,
            launch.arguments.iter().cloned(),
            &launch.directory,
        )
        .and_then(|spec| {
            spec.with_env_overlay(
                launch
                    .environment
                    .iter()
                    .map(|(name, value)| (name.as_bytes().to_vec(), value.as_bytes().to_vec())),
            )
        })
        .map_err(|error| Error::Protocol(error.to_string()))?;
        let (token, lease) = CancelToken::standalone();
        let pty = launch_pty(
            &spec,
            PtySize {
                columns: geometry.columns,
                rows: geometry.rows,
            },
            &token,
        )
        .map_err(|error| Error::Unavailable(error.to_string()))?;
        let master = pty
            .master()
            .try_clone()
            .map_err(|error| io_error("retain pty master", error))?;
        set_nonblocking(&master)?;
        let reader = master
            .try_clone()
            .map_err(|error| io_error("retain pty reader", error))?;
        let (output_tx, output) = mpsc::sync_channel(OUTPUT_QUEUE);
        let (events_tx, events) = mpsc::sync_channel(OUTPUT_QUEUE);
        let read_wake = wake
            .try_clone()
            .map_err(|error| io_error("retain pty wake", error))?;
        std::thread::Builder::new()
            .name("strop-pty-read".into())
            .spawn(move || read_output(reader, output_tx, read_wake))
            .map_err(|error| io_error("spawn pty reader", error))?;
        let exit_tx = events_tx.clone();
        let running = pty.into_running();
        std::thread::Builder::new()
            .name("strop-pty-wait".into())
            .spawn(move || {
                let status = match running.wait() {
                    Ok(Settlement::Recorded(StatusRecord::Exited(code))) => {
                        ExitStatus::Exit(code as i32)
                    }
                    Ok(Settlement::Recorded(StatusRecord::Signaled(signal))) => {
                        ExitStatus::Signal(signal as i32)
                    }
                    // Revocation and supervision failures: no code is guessed.
                    Ok(Settlement::Revoked) | Err(_) => ExitStatus::Lost,
                };
                let _ = exit_tx.send(ExecEvent::Exit(status));
                nudge(&wake);
            })
            .map_err(|error| io_error("spawn pty waiter", error))?;
        Ok(Self {
            master,
            lease: Some(lease),
            output,
            events,
            events_tx,
            pending: VecDeque::new(),
            acks: VecDeque::new(),
            written: 0,
            sequence: 0,
        })
    }

    /// Queue input; the returned wire sequence is acknowledged on
    /// [`ExecEvent::Input`] once every byte reached the master.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<u64, Error> {
        self.pending.extend(bytes);
        self.sequence += 1;
        let mark = self.written + self.pending.len() as u64;
        self.acks.push_back((self.sequence, mark));
        self.flush()?;
        Ok(self.sequence)
    }

    /// Write as much retained input as the master takes right now.
    pub(crate) fn flush(&mut self) -> Result<(), Error> {
        while !self.pending.is_empty() {
            let (front, _) = self.pending.as_slices();
            match (&self.master).write(front) {
                Ok(0) => break,
                Ok(count) => {
                    self.pending.drain(..count);
                    self.written += count as u64;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                // EIO: the child closed its side; the exit event follows.
                Err(error) if error.raw_os_error() == Some(libc::EIO) => {
                    self.pending.clear();
                    break;
                }
                Err(error) => return Err(io_error("write pty input", error)),
            }
        }
        while self
            .acks
            .front()
            .is_some_and(|(_, mark)| *mark <= self.written)
        {
            if let Some((sequence, _)) = self.acks.pop_front() {
                let _ = self.events_tx.try_send(ExecEvent::Input { sequence });
            }
        }
        Ok(())
    }

    pub(crate) fn has_pending_input(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn try_output(&self) -> Result<StreamEvent, TryRecvError> {
        self.output.try_recv()
    }

    pub(crate) fn events(&self) -> &Receiver<ExecEvent> {
        &self.events
    }

    /// `TIOCSWINSZ` on the master: applied now, so the VT flips now.
    pub(crate) fn resize(&self, geometry: Geometry) -> Result<(), Error> {
        resize_pty(
            &self.master,
            PtySize {
                columns: geometry.columns,
                rows: geometry.rows,
            },
        )
        .map_err(|error| Error::Unavailable(error.to_string()))
    }

    /// Revoke the lease: the waiter thread runs TERM, grace, KILL and the
    /// reap, then reports the exit.
    pub(crate) fn terminate(&mut self) {
        if let Some(lease) = self.lease.take() {
            lease.cancel(CancelReason::OwnerClosed);
        }
    }

    pub(crate) fn master_fd(&self) -> i32 {
        self.master.as_raw_fd()
    }
}

fn set_nonblocking(file: &File) -> Result<(), Error> {
    // SAFETY: `file` is a live descriptor this transport owns; F_GETFL and
    // F_SETFL on it have no memory effects.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags == -1 {
        return Err(io_error("read pty flags", io::Error::last_os_error()));
    }
    // SAFETY: as above; the flag word was just read from the same descriptor.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io_error("set pty nonblocking", io::Error::last_os_error()));
    }
    Ok(())
}

/// The reader thread: merged child output as bounded chunks, in order,
/// ending with one `last` marker when the slave side closes (Linux
/// reports EIO, not EOF, once the last slave descriptor is gone).
fn read_output(master: File, output: SyncSender<StreamEvent>, wake: UnixDatagram) {
    use std::io::Read;
    let mut buffer = vec![0u8; READ_CHUNK];
    let finish = |output: &SyncSender<StreamEvent>| {
        let _ = output.send(StreamEvent::Chunk(StreamChunk {
            stream: strop_worker_protocol::StreamId(0),
            sequence: 0,
            last: true,
            bytes: Vec::new(),
        }));
        nudge(&wake);
    };
    loop {
        match (&master).read(&mut buffer) {
            Ok(0) => return finish(&output),
            Ok(count) => {
                let chunk = StreamEvent::Chunk(StreamChunk {
                    stream: strop_worker_protocol::StreamId(0),
                    sequence: 0,
                    last: false,
                    bytes: buffer[..count].to_vec(),
                });
                if output.send(chunk).is_err() {
                    return;
                }
                nudge(&wake);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let mut descriptor = libc::pollfd {
                    fd: master.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: this thread owns `master` and the pollfd storage;
                // an infinite timeout is right — the descriptor's close or
                // EIO ends the loop.
                let ready = unsafe { libc::poll(&mut descriptor, 1, -1) };
                if ready < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    return finish(&output);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return finish(&output),
        }
    }
}
