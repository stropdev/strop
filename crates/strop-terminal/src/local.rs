//! The in-process PTY every session runs on (0090 §4): a local shell
//! directly, a remote one as `ssh -tt` (see [`crate::transport`]). The
//! master stays in the editor process, so nothing sits between a
//! keystroke and the child but the kernel's terminal.
//!
//! The async invariant (0001 §3) holds by construction: the editor
//! thread only enqueues intents on the service; this PTY runs on the
//! service's actor thread plus two helpers. The master is non-blocking,
//! so input a child stops reading is retained here and flushed as `poll`
//! reports room — the actor never parks on a write while output still
//! arrives. The reader thread blocks on `poll`/`read` only and parks on
//! the bounded output channel when the actor is behind: the PTY's own
//! buffer then applies backpressure to the child, never a dropped byte.
//! The waiter thread owns the supervised child and reports the truthful
//! exit; cancelling its token is the revoke path.
//!
//! **Typeahead.** A held session keeps its input until the child's first
//! output byte. `ssh` reads keys as soon as it starts, but the remote
//! shell has not yet set up its terminal and discards them; the first
//! output is the remote side speaking, and only then is input written.
use crate::{
    launch::Launch,
    model::{Geometry, Phase},
    pty::{self, PtySize, Settlement, StatusRecord},
    Error,
};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Write},
    os::{fd::AsRawFd, unix::net::UnixDatagram},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
};
use strop_core::worker::{CancelHandle, CancelReason, CancelToken};

/// Output chunks queued ahead of the actor before the reader parks.
const OUTPUT_QUEUE: usize = 64;
/// One `read` from the master.
const READ_CHUNK: usize = 64 * 1024;
/// Upper bound on one actor park; output and intents arrive by nudge,
/// so this only recovers a lost wake.
const PARK_MS: i32 = 250;

/// What the reader thread observed on the master.
pub(crate) enum Output {
    Bytes(Vec<u8>),
    /// The slave side closed: no more output will arrive.
    Closed,
}

pub(crate) struct LocalPty {
    master: File,
    /// Cancelling revokes the child (TERM, grace, KILL, reap) on the
    /// waiter thread; dropping the handle does the same.
    revoke: Option<CancelHandle>,
    output: Receiver<Output>,
    /// The settled phase, sent once by the waiter thread.
    exit: Receiver<Phase>,
    /// Queued input not yet written to the master.
    pending: VecDeque<u8>,
    /// Bytes ever queued, and bytes ever written: a caller's mark is
    /// delivered once `written` reaches it.
    queued: u64,
    written: u64,
    /// Typeahead is held until the child's first output byte.
    held: bool,
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
    /// `hold` keeps typeahead until the child first speaks.
    pub(crate) fn spawn(
        launch: &Launch,
        geometry: Geometry,
        hold: bool,
        wake: UnixDatagram,
    ) -> Result<Self, Error> {
        let (token, revoke) = CancelToken::standalone();
        let pty = pty::spawn(launch, size(geometry), pty::DEFAULT_GRACE, &token)
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
        let (exit_tx, exit) = mpsc::sync_channel(1);
        let read_wake = wake
            .try_clone()
            .map_err(|error| io_error("retain pty wake", error))?;
        std::thread::Builder::new()
            .name("strop-pty-read".into())
            .spawn(move || read_output(reader, output_tx, read_wake))
            .map_err(|error| io_error("spawn pty reader", error))?;
        let running = pty.into_running();
        std::thread::Builder::new()
            .name("strop-pty-wait".into())
            .spawn(move || {
                let (code, signal) = match running.wait() {
                    Ok(Settlement::Recorded(StatusRecord::Exited(code))) => {
                        (Some(code as i32), None)
                    }
                    Ok(Settlement::Recorded(StatusRecord::Signaled(signal))) => {
                        (None, Some(signal as i32))
                    }
                    // Revocation and supervision failures: no code is guessed.
                    Ok(Settlement::Revoked) | Err(_) => (None, None),
                };
                let _ = exit_tx.send(Phase::Exited { code, signal });
                nudge(&wake);
            })
            .map_err(|error| io_error("spawn pty waiter", error))?;
        Ok(Self {
            master,
            revoke: Some(revoke),
            output,
            exit,
            pending: VecDeque::new(),
            queued: 0,
            written: 0,
            held: hold,
        })
    }

    /// Queue input and write what the master takes now. Returns the mark
    /// [`Self::delivered`] reaches once every byte is written.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<u64, Error> {
        self.pending.extend(bytes);
        self.queued += bytes.len() as u64;
        self.flush()?;
        Ok(self.queued)
    }

    /// The mark every input queued so far will be delivered at.
    pub(crate) fn queued(&self) -> u64 {
        self.queued
    }

    /// Bytes written to the master so far.
    pub(crate) fn delivered(&self) -> u64 {
        self.written
    }

    /// Write as much retained input as the master takes right now.
    pub(crate) fn flush(&mut self) -> Result<(), Error> {
        while !self.held && !self.pending.is_empty() {
            let (front, _) = self.pending.as_slices();
            match (&self.master).write(front) {
                Ok(0) => break,
                Ok(count) => {
                    self.pending.drain(..count);
                    self.written += count as u64;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                // EIO: the child closed its side; the exit follows.
                Err(error) if error.raw_os_error() == Some(libc::EIO) => {
                    self.pending.clear();
                    break;
                }
                Err(error) => return Err(io_error("write pty input", error)),
            }
        }
        Ok(())
    }

    /// The next output arrival. The child's first byte releases held
    /// typeahead.
    pub(crate) fn try_output(&mut self) -> Result<Output, TryRecvError> {
        let output = self.output.try_recv()?;
        if self.held && matches!(&output, Output::Bytes(bytes) if !bytes.is_empty()) {
            self.held = false;
        }
        Ok(output)
    }

    pub(crate) fn try_exit(&self) -> Result<Phase, TryRecvError> {
        self.exit.try_recv()
    }

    /// `TIOCSWINSZ` on the master: applied now, so the VT flips now. For
    /// `ssh -tt` the SIGWINCH makes ssh send the remote a window change.
    pub(crate) fn resize(&self, geometry: Geometry) -> Result<(), Error> {
        pty::resize(&self.master, size(geometry))
            .map_err(|error| Error::Unavailable(error.to_string()))
    }

    /// Revoke the child: the waiter thread runs TERM, grace, KILL and the
    /// reap, then reports the exit.
    pub(crate) fn terminate(&mut self) {
        if let Some(revoke) = self.revoke.take() {
            revoke.cancel(CancelReason::OwnerClosed);
        }
    }

    /// Park until a wake (an intent or an output nudge), or until the
    /// master can take retained input.
    pub(crate) fn wait(&self, wake: &UnixDatagram) -> Result<(), Error> {
        let writable = !self.held && !self.pending.is_empty();
        let mut descriptors = [
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if writable {
                    self.master.as_raw_fd()
                } else {
                    -1
                },
                events: libc::POLLOUT,
                revents: 0,
            },
        ];
        // SAFETY: the caller owns the wake, this PTY owns the master, and
        // the pollfd storage lives for the duration of the call.
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, PARK_MS) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(io_error("wait for terminal input", error));
            }
        }
        Ok(())
    }
}

fn size(geometry: Geometry) -> PtySize {
    PtySize {
        columns: geometry.columns,
        rows: geometry.rows,
    }
}

fn set_nonblocking(file: &File) -> Result<(), Error> {
    // SAFETY: `file` is a live descriptor this PTY owns; F_GETFL and
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
/// ending with one [`Output::Closed`] when the slave side closes (Linux
/// reports EIO, not EOF, once the last slave descriptor is gone).
fn read_output(master: File, output: SyncSender<Output>, wake: UnixDatagram) {
    use std::io::Read;
    let mut buffer = vec![0u8; READ_CHUNK];
    let finish = |output: &SyncSender<Output>| {
        let _ = output.send(Output::Closed);
        nudge(&wake);
    };
    loop {
        match (&master).read(&mut buffer) {
            Ok(0) => return finish(&output),
            Ok(count) => {
                if output
                    .send(Output::Bytes(buffer[..count].to_vec()))
                    .is_err()
                {
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

#[cfg(test)]
mod tests;
