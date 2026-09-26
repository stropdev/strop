//! Worker-owned PTY session (0058 WK12). The child, its terminal and its
//! supervised lease live on the namespace's worker — reached through the
//! worker client over the same wire as every other owned execution —
//! while the VT, bounded history and immutable publications stay here.
//! Editor state receives only model::Update, never transport handles.
//!
//! Ordering contract (the 0055 lifecycle, preserved over the wire):
//!
//! - Input chunks are admitted into one client sequence space and the
//!   worker acknowledges each delivered chunk ([`ExecEvent::Input`]);
//!   acknowledgments release the retained-input budget exactly like the
//!   helper's ACKs did. A child that stops draining fills the worker's
//!   bounded queue and is revoked — input admission fails truthfully
//!   here first (the budget), never by a dropped byte.
//! - Resize rides the worker's ordered PTY control queue; the reply is
//!   applied-geometry, and the worker client's [`StreamEvent::Resized`]
//!   marker arrives on the output stream in exact wire order. The VT
//!   parser's geometry flips only at that marker, so output before it
//!   is parsed under the old geometry and output after it under the
//!   new — the helper's ACK-boundary semantics unchanged.
//! - Exit is truthful: the supervisor's attested status maps to
//!   [`Phase::Exited`], an unattested one (`Lost`, revocation) carries
//!   no code, and a dead worker is a failure, never a guessed exit.
use crate::{
    launch::Launch,
    model::*,
    vt::{Emission, Vt},
    Error,
};
use parking_lot::Mutex;
use std::{
    collections::{HashMap, VecDeque},
    os::unix::net::UnixDatagram,
    sync::LazyLock,
};
use strop_core::worker::CancelToken;
use strop_worker_client::{ExecEvent, PtySession, StreamEvent, Worker};
use strop_worker_protocol::{ExitStatus, PtyGeometry, StreamId};

/// One admitted input or resize awaiting its worker acknowledgment.
struct Pending {
    sequence: u64,
    bytes: usize,
}

/// Stream arrivals nudge the owning service's wake datagram: the
/// connection's reader thread delivers chunks, this registry turns them
/// into the same poll wake the intents use, so output latency never
/// depends on the poll timeout. Entries key on the raw stream id;
/// cross-worker collisions nudge a second service spuriously (it finds
/// no chunk and sleeps again), never drop the real owner's wake.
type WakeRegistry = HashMap<u64, Vec<(u64, UnixDatagram)>>;
static WAKES: LazyLock<Mutex<WakeRegistry>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Install the stream-arrival hook on one lease (idempotent): worker
/// stream chunks nudge the registered service wakes. One hook per
/// lease; the terminal service owns it (readers of finite payloads use
/// blocking reads and need no nudge).
pub(crate) fn install_wake_hook(worker: &Worker) {
    worker.set_stream_notifier(|stream: StreamId| {
        let wakes = WAKES.lock();
        if let Some(entries) = wakes.get(&stream.0) {
            for (_, wake) in entries {
                // A full datagram buffer means the service is already
                // awake; dropping the nudge loses nothing.
                let _ = wake.send(&[1]);
            }
        }
    });
}

fn register_wake(stream: StreamId, session: SessionId, wake: UnixDatagram) {
    WAKES
        .lock()
        .entry(stream.0)
        .or_default()
        .push((session.get(), wake));
}

fn deregister_wake(stream: StreamId, session: SessionId) {
    let mut wakes = WAKES.lock();
    if let Some(entries) = wakes.get_mut(&stream.0) {
        entries.retain(|(owner, _)| *owner != session.get());
        if entries.is_empty() {
            wakes.remove(&stream.0);
        }
    }
}

pub struct Client {
    pty: PtySession,
    token: CancelToken,
    vt: Vt,
    session: SessionId,
    phase: Phase,
    exit: Option<ExitStatus>,
    output_closed: bool,
    failure: Option<String>,
    warning: Option<String>,
    pending: VecDeque<Pending>,
    resizes: VecDeque<(u64, Geometry)>,
    retained_input: usize,
    sequence: u64,
    acknowledged: u64,
    geometry_revision: u64,
    effects: Vec<Effect>,
}

/// Emulator settings owned by the embedder rather than the PTY lease.
pub(crate) struct EmulationSettings<'a> {
    pub keyboard: u8,
    pub palette: Option<&'a Palette>,
}

impl Client {
    pub(crate) fn spawn(
        session: SessionId,
        worker: Worker,
        launch: &Launch,
        geometry: Geometry,
        emulation: EmulationSettings<'_>,
        wake: UnixDatagram,
        token: &CancelToken,
    ) -> Result<Self, Error> {
        use std::os::unix::ffi::OsStrExt;
        let mut vt = Vt::new(session, geometry, emulation.palette)?;
        vt.keyboard_capabilities(emulation.keyboard)?;
        let spec = strop_worker_protocol::ExecSpec {
            program: launch.program.as_bytes().to_vec(),
            argv: launch
                .arguments
                .iter()
                .map(|argument| argument.as_bytes().to_vec())
                .collect(),
            cwd: launch.directory.as_os_str().as_bytes().to_vec(),
            env: launch
                .environment
                .iter()
                .map(|(name, value)| strop_worker_protocol::request::EnvVar {
                    name: name.as_bytes().to_vec(),
                    value: value.as_bytes().to_vec(),
                })
                .collect(),
            service: true,
            pty: Some(PtyGeometry {
                columns: geometry.columns,
                rows: geometry.rows,
            }),
        };
        let pty = worker.exec_pty(token, spec).map_err(|error| match error {
            strop_worker_client::ClientError::Cancelled => Error::Closed,
            other => Error::Unavailable(other.to_string()),
        })?;
        register_wake(pty.output_stream(), session, wake);
        Ok(Self {
            pty,
            token: token.clone(),
            vt,
            session,
            phase: Phase::Running,
            exit: None,
            output_closed: false,
            failure: None,
            warning: None,
            pending: VecDeque::new(),
            resizes: VecDeque::new(),
            retained_input: 0,
            sequence: 0,
            acknowledged: 0,
            geometry_revision: geometry.revision,
            effects: Vec::new(),
        })
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Wait for service wakes (intents and stream-arrival nudges share
    /// the one datagram). The timeout only bounds wake-loss recovery;
    /// ordinary output arrives by nudge, never by poll.
    pub(crate) fn wait(&self, wake: &UnixDatagram) -> Result<(), Error> {
        use std::os::fd::AsRawFd;
        let mut descriptors = [libc::pollfd {
            fd: wake.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: the service owns the descriptor and the pollfd storage.
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), 1, 250) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(Error::Io {
                    operation: "wait for terminal worker input",
                    detail: error.to_string(),
                });
            }
        }
        Ok(())
    }

    pub fn input(
        &mut self,
        input: &strop_core::frontend_input::Input,
        confirmed: bool,
    ) -> Result<u64, Error> {
        if self.phase != Phase::Running {
            return Err(Error::Closed);
        }
        let emission = self.vt.input(input, confirmed)?;
        let sequence = self.feed(&emission.reply)?;
        self.effects(emission.effects);
        Ok(sequence)
    }

    pub fn focus(&mut self, focused: bool) -> Result<u64, Error> {
        if self.phase != Phase::Running {
            return Err(Error::Closed);
        }
        let emission = self.vt.focus(focused)?;
        self.feed(&emission.reply)
    }

    pub fn resize(&mut self, geometry: Geometry) -> Result<u64, Error> {
        if self.phase != Phase::Running {
            return Err(Error::Closed);
        }
        if !geometry.valid() || geometry.revision <= self.geometry_revision {
            return Err(Error::Protocol("stale or invalid terminal geometry".into()));
        }
        // Applied-geometry semantics: the worker's reply means the ioctl
        // landed; the parser flips at the ordered output marker.
        self.pty
            .resize(PtyGeometry {
                columns: geometry.columns,
                rows: geometry.rows,
            })
            .map_err(|error| Error::Unavailable(error.to_string()))?;
        let charge = 16usize;
        if charge > (MAX_INPUT_BYTES + 128).saturating_sub(self.retained_input)
            || self.pending.len() >= 128
        {
            return Err(Error::InputFull);
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(Error::Capacity("terminal input sequence exhausted"))?;
        self.retained_input += charge;
        self.sequence = sequence;
        self.pending.push_back(Pending {
            sequence,
            bytes: charge,
        });
        self.resizes.push_back((sequence, geometry));
        self.geometry_revision = geometry.revision;
        Ok(sequence)
    }

    /// One input emission as bounded ordered chunks. Oversized text is
    /// split well under the wire ceiling; the last chunk's sequence
    /// acknowledges the whole emission (chunks are delivered in order).
    fn feed(&mut self, bytes: &[u8]) -> Result<u64, Error> {
        const FEED_CHUNK: usize = 128 * 1024;
        if bytes.is_empty() {
            let sequence = self
                .sequence
                .checked_add(1)
                .ok_or(Error::Capacity("terminal input sequence exhausted"))?;
            self.sequence = sequence;
            return Ok(sequence);
        }
        let chunks = bytes.chunks(FEED_CHUNK);
        let count = chunks.len();
        let total: usize = bytes.len() + 8 * count;
        if total > (MAX_INPUT_BYTES + 128).saturating_sub(self.retained_input)
            || self.pending.len() + count > 128
        {
            return Err(Error::InputFull);
        }
        let mut last = self.sequence;
        for chunk in chunks {
            let sequence = self
                .sequence
                .checked_add(1)
                .ok_or(Error::Capacity("terminal input sequence exhausted"))?;
            self.pty
                .feed(chunk)
                .map_err(|error| Error::Unavailable(error.to_string()))?;
            self.retained_input += chunk.len() + 8;
            self.sequence = sequence;
            self.pending.push_back(Pending {
                sequence,
                bytes: chunk.len() + 8,
            });
            last = sequence;
        }
        Ok(last)
    }

    pub fn stop(&mut self) -> Result<(), Error> {
        if !self.phase.live() {
            return Ok(());
        }
        self.phase = Phase::Closing;
        if !self.pending.is_empty() {
            self.warning = Some("terminal stopped; unacknowledged input was revoked".into());
        }
        self.pty
            .terminate()
            .map_err(|error| Error::Unavailable(error.to_string()))
    }

    /// Consume a bounded turn: output chunks, the ordered resize
    /// markers and input acknowledgments, then settlement. The VT
    /// geometry flips only at a marker, preserving the 0055 resize
    /// boundary over the wire.
    pub fn poll(&mut self) -> Result<Option<Update>, Error> {
        if !self.phase.live() {
            return Ok(None);
        }
        if self.token.is_cancelled() && self.phase != Phase::Closing {
            self.stop()?;
        }
        let mut changed = false;
        let mut dirty = false;
        for _ in 0..32 {
            if self.output_closed {
                break;
            }
            let event = match self.pty.try_output() {
                Ok(event) => event,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.output_closed = true;
                    if self.exit.is_none() && self.failure.is_none() {
                        self.failure =
                            Some("terminal output channel closed without a status".into());
                    }
                    changed = true;
                    break;
                }
            };
            changed = true;
            match event {
                StreamEvent::Chunk(chunk) => {
                    // The worker orders the terminal chunk before ExecExit
                    // on one wire, but the client routes them into separate
                    // bounded channels. A 32-chunk poll turn may observe
                    // the exit first while older chunks remain queued.
                    // Only the output stream's final marker permits phase
                    // settlement below; the exit event is retained meanwhile.
                    if !chunk.bytes.is_empty() {
                        let emission = self.vt.feed(&chunk.bytes)?;
                        self.emission(emission)?;
                        dirty = true;
                    }
                    if chunk.last {
                        self.output_closed = true;
                    }
                }
                StreamEvent::Resized => {
                    let Some((sequence, geometry)) = self.resizes.pop_front() else {
                        return Err(Error::Protocol(
                            "unexpected terminal resize boundary".into(),
                        ));
                    };
                    self.vt.resize(geometry)?;
                    self.cover(sequence);
                    dirty = true;
                }
                StreamEvent::Failed(reason) => {
                    self.failure = Some(reason);
                    self.output_closed = true;
                }
            }
        }
        loop {
            match self.pty.events().try_recv() {
                Ok(ExecEvent::Input { sequence }) => {
                    self.cover(sequence);
                    changed = true;
                }
                Ok(ExecEvent::Exit(status)) => {
                    self.exit = Some(status);
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if self.exit.is_none() {
                        self.exit = Some(ExitStatus::Lost);
                    }
                    break;
                }
            }
        }
        if self.output_closed && self.exit.is_some() {
            let status = self.exit.take();
            self.phase = if let Some(failure) = self.failure.take() {
                Phase::Failed(failure)
            } else {
                match status {
                    Some(ExitStatus::Exit(code)) => Phase::Exited {
                        code: Some(code),
                        signal: None,
                    },
                    Some(ExitStatus::Signal(signal)) => Phase::Exited {
                        code: None,
                        signal: Some(signal),
                    },
                    // Revocation and every unattested settlement: no
                    // code is ever guessed.
                    Some(ExitStatus::Lost) | None => Phase::Exited {
                        code: None,
                        signal: None,
                    },
                }
            };
            self.pending.clear();
            self.resizes.clear();
            self.retained_input = 0;
            changed = true;
        }
        if !changed && self.effects.is_empty() && self.warning.is_none() {
            return Ok(None);
        }
        let frame = if dirty {
            let (frame, emission) = self.vt.snapshot()?;
            self.emission(emission)?;
            Some(frame)
        } else {
            None
        };
        Ok(Some(Update {
            session: self.session,
            phase: self.phase.clone(),
            frame,
            effects: std::mem::take(&mut self.effects),
            acknowledged_input: self.acknowledged,
            warning: self.warning.take().or_else(|| self.failure.clone()),
        }))
    }

    /// One delivered sequence: inputs and ordered resizes share one
    /// client sequence space, and the worker delivers in order, so an
    /// acknowledgment covers every earlier admission.
    fn cover(&mut self, sequence: u64) {
        while self
            .pending
            .front()
            .is_some_and(|pending| pending.sequence <= sequence)
        {
            if let Some(pending) = self.pending.pop_front() {
                self.retained_input -= pending.bytes;
            }
        }
        self.acknowledged = self.acknowledged.max(sequence);
    }

    fn emission(&mut self, emission: Emission) -> Result<(), Error> {
        self.effects(emission.effects);
        if !emission.reply.is_empty() && self.phase == Phase::Running && self.exit.is_none() {
            self.feed(&emission.reply)?;
        }
        Ok(())
    }

    fn effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            // Effects are state/flags, not an unbounded event history. Preserve
            // the latest title/cwd and one notification of each denied action.
            if let Some(existing) = self.effects.iter_mut().find(|existing| {
                std::mem::discriminant(*existing) == std::mem::discriminant(&effect)
            }) {
                *existing = effect;
            } else {
                self.effects.push(effect);
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        deregister_wake(self.pty.output_stream(), self.session);
        if self.phase.live() {
            let _ = self.pty.terminate();
        }
    }
}
