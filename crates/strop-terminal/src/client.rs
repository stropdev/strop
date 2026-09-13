//! Worker-only session owner. Polling and destruction may perform native work;
//! editor state receives only model::Update, never this type or its descriptors.
use crate::{
    launch::{Launch, WireLaunch},
    model::*,
    protocol,
    vt::{Emission, Vt},
    Error,
};
use std::{
    collections::VecDeque,
    io,
    os::{
        fd::AsRawFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Command, Stdio},
};
use strop_core::{process::OwnedProcess, worker::CancelToken};

struct Connection {
    channel: UnixStream,
    process: OwnedProcess,
}
impl Drop for Connection {
    fn drop(&mut self) {
        // A panicking worker no longer drains output. Close both halves before
        // OwnedProcess waits, so helper backpressure cannot deadlock destruction.
        let _ = self.channel.shutdown(std::net::Shutdown::Both);
    }
}
struct Pending {
    sequence: u64,
    bytes: usize,
    resize: Option<Geometry>,
}

pub struct Client {
    connection: Connection,
    token: CancelToken,
    reader: protocol::Reader,
    writer: protocol::Writer,
    vt: Vt,
    session: SessionId,
    phase: Phase,
    final_phase: Option<Phase>,
    failure: Option<String>,
    warning: Option<String>,
    pending: VecDeque<Pending>,
    retained_input: usize,
    sequence: u64,
    acknowledged: u64,
    geometry_revision: u64,
    effects: Vec<Effect>,
}
impl Client {
    pub fn spawn(
        session: SessionId,
        launch: &Launch,
        geometry: Geometry,
        keyboard: u8,
        token: &CancelToken,
    ) -> Result<Self, Error> {
        let body = WireLaunch::encode(launch, geometry)?;
        let mut vt = Vt::new(session, geometry)?;
        vt.keyboard_capabilities(keyboard)?;
        let (channel, inherited) =
            UnixStream::pair().map_err(|error| io_error("create helper channel", error))?;
        channel
            .set_nonblocking(true)
            .map_err(|error| io_error("configure helper channel", error))?;
        let lease = channel
            .try_clone()
            .map_err(|error| io_error("retain helper lease", error))?;
        let fd = inherited.as_raw_fd();
        let executable =
            std::env::current_exe().map_err(|error| io_error("locate terminal helper", error))?;
        let mut command = Command::new(executable);
        command
            .args(["--terminal-helper", &fd.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: inherited is owned through spawn. Only this child's fd loses
        // CLOEXEC; no other concurrent process can inherit the private socket.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags == -1 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let process = OwnedProcess::spawn_leased(&mut command, token, lease)
            .map_err(|failure| Error::Unavailable(failure.message))?;
        drop(inherited);
        let connection = Connection { channel, process };
        let mut writer = protocol::Writer::default();
        writer.push(protocol::LAUNCH, body)?;
        Ok(Self {
            connection,
            token: token.clone(),
            reader: protocol::Reader::default(),
            writer,
            vt,
            session,
            phase: Phase::Starting,
            final_phase: None,
            failure: None,
            warning: None,
            pending: VecDeque::new(),
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

    pub(crate) fn wait(&self, wake: &std::os::unix::net::UnixDatagram) -> Result<(), Error> {
        let writable = self.writer.pending() && self.phase != Phase::Closing;
        let mut descriptors = [
            libc::pollfd {
                fd: self.connection.channel.as_raw_fd(),
                events: libc::POLLIN | if writable { libc::POLLOUT } else { 0 },
                revents: 0,
            },
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: the worker owns both descriptors and writable pollfd storage.
        let result = unsafe {
            libc::poll(
                descriptors.as_mut_ptr(),
                descriptors.len() as libc::nfds_t,
                250,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(io_error("wait for terminal worker input", error));
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
        let sequence = self.admit(protocol::INPUT, emission.reply, None)?;
        self.effects(emission.effects);
        Ok(sequence)
    }

    pub fn focus(&mut self, focused: bool) -> Result<u64, Error> {
        if self.phase != Phase::Running {
            return Err(Error::Closed);
        }
        let emission = self.vt.focus(focused)?;
        self.admit(protocol::INPUT, emission.reply, None)
    }

    pub fn resize(&mut self, geometry: Geometry) -> Result<u64, Error> {
        if self.phase != Phase::Running {
            return Err(Error::Closed);
        }
        if !geometry.valid() || geometry.revision <= self.geometry_revision {
            return Err(Error::Protocol("stale or invalid terminal geometry".into()));
        }
        let bytes =
            serde_json::to_vec(&geometry).map_err(|error| Error::Protocol(error.to_string()))?;
        let sequence = self.admit(protocol::RESIZE, bytes, Some(geometry))?;
        self.geometry_revision = geometry.revision;
        Ok(sequence)
    }

    pub fn stop(&mut self) -> Result<(), Error> {
        if !self.phase.live() {
            return Ok(());
        }
        self.phase = Phase::Closing;
        if !self.pending.is_empty() {
            self.warning = Some("terminal stopped; unacknowledged input was revoked".into());
        }
        self.writer = protocol::Writer::default();
        self.connection
            .process
            .terminate()
            .map_err(|failure| Error::Unavailable(failure.message))
    }

    /// Consume a bounded turn. ACK precedes output under the new geometry, so
    /// resize is applied to the parser only at that ordered transport boundary.
    pub fn poll(&mut self) -> Result<Option<Update>, Error> {
        if !self.phase.live() {
            return Ok(None);
        }
        if self.token.is_cancelled() && self.phase.live() && self.phase != Phase::Closing {
            self.stop()?;
        }
        let mut changed = false;
        let mut dirty = false;
        for _ in 0..32 {
            if self.final_phase.is_some() {
                break;
            }
            let Some(packet) = self.reader.next(&mut self.connection.channel)? else {
                break;
            };
            changed = true;
            match packet.kind {
                protocol::READY => {
                    if !matches!(self.phase, Phase::Starting | Phase::Closing)
                        || packet.body != protocol::VERSION.to_le_bytes()
                    {
                        return Err(Error::Protocol("invalid terminal helper readiness".into()));
                    }
                    if self.phase == Phase::Starting {
                        self.phase = Phase::Running;
                    }
                    dirty = true;
                }
                protocol::OUTPUT => {
                    if self.final_phase.is_some() {
                        return Err(Error::Protocol(
                            "terminal output followed final status".into(),
                        ));
                    }
                    let emission = self.vt.feed(&packet.body)?;
                    self.emission(emission)?;
                    dirty = true;
                }
                protocol::ACK => {
                    let (sequence, remaining) = protocol::sequence(&packet.body)?;
                    let expected = self.pending.front().ok_or_else(|| {
                        Error::Protocol("unexpected terminal acknowledgment".into())
                    })?;
                    if !remaining.is_empty() || sequence != expected.sequence {
                        return Err(Error::Protocol(
                            "out-of-order terminal acknowledgment".into(),
                        ));
                    }
                    if let Some(pending) = self.pending.pop_front() {
                        self.retained_input -= pending.bytes;
                        if let Some(geometry) = pending.resize {
                            self.vt.resize(geometry)?;
                            dirty = true;
                        }
                    }
                    self.acknowledged = sequence;
                }
                protocol::FAILED => {
                    let text = String::from_utf8(packet.body)
                        .map_err(|_| Error::Protocol("invalid terminal failure text".into()))?;
                    self.failure = Some(strop_core::layout::printable_text(text).into_owned());
                    self.phase = Phase::Closing;
                    self.writer = protocol::Writer::default();
                }
                protocol::EXITED => {
                    let phase = serde_json::from_slice(&packet.body)
                        .map_err(|error| Error::Protocol(error.to_string()))?;
                    if self.final_phase.is_some() || !matches!(phase, Phase::Exited { .. }) {
                        return Err(Error::Protocol("invalid terminal final status".into()));
                    }
                    self.final_phase = Some(phase);
                    // Final status closes the protocol. A short-lived helper may
                    // close with late input still unread; don't reinterpret that
                    // socket reset as failure after its validated final record.
                    self.phase = Phase::Closing;
                    self.writer = protocol::Writer::default();
                    break;
                }
                _ => {
                    return Err(Error::Protocol(
                        "unexpected terminal helper response".into(),
                    ))
                }
            }
        }
        if self.reader.eof() && self.final_phase.is_none() {
            return Err(Error::Protocol(
                "terminal helper closed without final status".into(),
            ));
        }
        if self.phase != Phase::Closing && self.final_phase.is_none() {
            if let Err(error) = self.writer.flush(&mut self.connection.channel) {
                self.warning = Some(format!(
                    "terminal input transport closed: {error}; draining final status"
                ));
                self.stop()?;
                changed = true;
            }
        }
        if self.final_phase.is_some()
            && self
                .connection
                .process
                .has_exited()
                .map_err(|failure| Error::Unavailable(failure.message))?
        {
            let status = self
                .connection
                .process
                .wait()
                .map_err(|failure| Error::Unavailable(failure.message))?;
            self.phase = if let Some(failure) = self.failure.take() {
                Phase::Failed(failure)
            } else if !status.success() {
                Phase::Failed(format!("terminal helper exited with {status}"))
            } else {
                self.final_phase
                    .take()
                    .ok_or_else(|| Error::Protocol("missing terminal final state".into()))?
            };
            self.final_phase = None;
            self.pending.clear();
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

    fn admit(&mut self, kind: u8, bytes: Vec<u8>, resize: Option<Geometry>) -> Result<u64, Error> {
        let charge = bytes.capacity().saturating_add(8);
        if charge > (MAX_INPUT_BYTES + 128).saturating_sub(self.retained_input)
            || self.pending.len() >= 128
        {
            return Err(Error::InputFull);
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(Error::Capacity("terminal input sequence exhausted"))?;
        self.writer.push_sequenced(kind, sequence, bytes)?;
        self.retained_input += charge;
        self.sequence = sequence;
        self.pending.push_back(Pending {
            sequence,
            bytes: charge,
            resize,
        });
        Ok(sequence)
    }

    fn emission(&mut self, emission: Emission) -> Result<(), Error> {
        self.effects(emission.effects);
        if !emission.reply.is_empty() && self.phase == Phase::Running && self.final_phase.is_none()
        {
            self.admit(protocol::INPUT, emission.reply, None)?;
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
fn io_error(operation: &'static str, error: io::Error) -> Error {
    Error::Io {
        operation,
        detail: error.to_string(),
    }
}
