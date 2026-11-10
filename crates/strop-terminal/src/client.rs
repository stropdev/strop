//! One PTY session: the in-process [`LocalPty`] a [`Target`] resolves to
//! (0090 §4), its VT, bounded history and immutable publications. Editor
//! state receives only model::Update, never PTY handles.
//!
//! Ordering contract (the 0055 lifecycle):
//!
//! - Inputs and resizes are accepted into one client sequence space.
//!   Each one records the PTY's input mark at that point and is
//!   acknowledged once the master has taken every byte up to it, which
//!   releases the retained-input budget. A child that stops draining
//!   leaves input retained here; acceptance then fails truthfully on the
//!   budget, never by a dropped byte.
//! - Resize is `TIOCSWINSZ` on the master, applied at once, and the VT
//!   parser's geometry flips at once: output read after the resize is
//!   parsed under the new geometry.
//! - Exit is truthful: the supervisor's recorded status maps to
//!   [`Phase::Exited`]; a revocation carries no code, and settlement waits
//!   for the output stream's end so no output is lost behind the exit.
use crate::local::{LocalPty, Output};
pub use crate::transport::Target;
use crate::{
    launch::Launch,
    model::*,
    vt::{Emission, Vt},
    Error,
};
use std::{collections::VecDeque, os::unix::net::UnixDatagram, sync::mpsc::TryRecvError};
use strop_core::worker::CancelToken;

/// One accepted input or resize awaiting delivery to the master.
struct Pending {
    sequence: u64,
    bytes: usize,
    /// The PTY input mark this entry is delivered at.
    mark: u64,
}

pub struct Client {
    pty: LocalPty,
    token: CancelToken,
    vt: Vt,
    session: SessionId,
    phase: Phase,
    exit: Option<Phase>,
    output_closed: bool,
    failure: Option<String>,
    warning: Option<String>,
    pending: VecDeque<Pending>,
    retained_input: usize,
    sequence: u64,
    acknowledged: u64,
    geometry_revision: u64,
    effects: Vec<Effect>,
    /// A resize flipped the VT; the next poll publishes the reflowed frame.
    resized: bool,
}

/// Emulator settings owned by the embedder rather than the PTY.
pub(crate) struct EmulationSettings<'a> {
    pub keyboard: u8,
    pub palette: Option<&'a Palette>,
}

impl Client {
    pub(crate) fn spawn(
        session: SessionId,
        target: &Target,
        launch: &Launch,
        geometry: Geometry,
        emulation: EmulationSettings<'_>,
        wake: UnixDatagram,
        token: &CancelToken,
    ) -> Result<Self, Error> {
        let mut vt = Vt::new(session, geometry, emulation.palette)?;
        vt.keyboard_capabilities(emulation.keyboard)?;
        let (local, hold) = crate::transport::local_launch(target, launch)?;
        let pty = LocalPty::spawn(&local, geometry, hold, wake)?;
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
            retained_input: 0,
            sequence: 0,
            acknowledged: 0,
            geometry_revision: geometry.revision,
            effects: Vec::new(),
            resized: false,
        })
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Wait for service wakes (intents and output nudges share the one
    /// datagram), or for the master to take retained input.
    pub(crate) fn wait(&self, wake: &UnixDatagram) -> Result<(), Error> {
        self.pty.wait(wake)
    }

    pub fn input(&mut self, input: &strop_core::frontend_input::Input) -> Result<u64, Error> {
        if self.phase != Phase::Running {
            return Err(Error::Closed);
        }
        let emission = self.vt.input(input)?;
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

    /// Service-owned appearance change: no PTY input or stream sequence is
    /// consumed, and publication does not depend on fresh child output.
    pub(crate) fn set_theme(&mut self, theme: strop_core::theme::ThemeId) -> Result<Update, Error> {
        if self.phase != Phase::Running {
            return Err(Error::Closed);
        }
        self.vt.configure_palette(&Palette::for_theme(theme))?;
        let (frame, emission) = self.vt.snapshot()?;
        self.emission(emission)?;
        Ok(Update {
            session: self.session,
            phase: self.phase.clone(),
            frame: Some(frame),
            effects: std::mem::take(&mut self.effects),
            acknowledged_input: self.acknowledged,
            warning: self.warning.take(),
        })
    }

    pub fn resize(&mut self, geometry: Geometry) -> Result<u64, Error> {
        if self.phase != Phase::Running {
            return Err(Error::Closed);
        }
        if !geometry.valid() || geometry.revision <= self.geometry_revision {
            return Err(Error::Protocol("stale or invalid terminal geometry".into()));
        }
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
        self.pty.resize(geometry)?;
        self.vt.resize(geometry)?;
        self.retained_input += charge;
        self.sequence = sequence;
        self.pending.push_back(Pending {
            sequence,
            bytes: charge,
            mark: self.pty.queued(),
        });
        self.geometry_revision = geometry.revision;
        self.resized = true;
        self.retire();
        Ok(sequence)
    }

    /// One input emission, acknowledged once the master took all of it.
    fn feed(&mut self, bytes: &[u8]) -> Result<u64, Error> {
        if bytes.is_empty() {
            // No bytes means nothing to deliver. Reuse the preceding
            // sequence so the service can settle immediately, or behind
            // that preceding input if it is still in flight.
            return Ok(self.sequence);
        }
        let charge = bytes.len() + 8;
        if charge > (MAX_INPUT_BYTES + 128).saturating_sub(self.retained_input)
            || self.pending.len() >= 128
        {
            return Err(Error::InputFull);
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(Error::Capacity("terminal input sequence exhausted"))?;
        let mark = self.pty.feed(bytes)?;
        self.retained_input += charge;
        self.sequence = sequence;
        self.pending.push_back(Pending {
            sequence,
            bytes: charge,
            mark,
        });
        self.retire();
        Ok(sequence)
    }

    pub fn stop(&mut self) -> Result<(), Error> {
        if !self.phase.live() {
            return Ok(());
        }
        self.phase = Phase::Closing;
        if !self.pending.is_empty() {
            self.warning = Some("terminal stopped; undelivered input was revoked".into());
        }
        self.pty.terminate();
        Ok(())
    }

    /// Consume a bounded turn: retained input, output chunks and input
    /// acknowledgments, then settlement.
    pub fn poll(&mut self) -> Result<Option<Update>, Error> {
        if !self.phase.live() {
            return Ok(None);
        }
        if self.token.is_cancelled() && self.phase != Phase::Closing {
            self.stop()?;
        }
        let mut changed = std::mem::take(&mut self.resized);
        let mut dirty = changed;
        // Retained input gets its chance on every turn.
        self.pty.flush()?;
        for _ in 0..32 {
            if self.output_closed {
                break;
            }
            let output = match self.pty.try_output() {
                Ok(output) => output,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
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
            match output {
                Output::Bytes(bytes) => {
                    let emission = self.vt.feed(&bytes)?;
                    self.emission(emission)?;
                    dirty = true;
                }
                // The exit arrives on its own channel and may be seen
                // first while older chunks remain queued: only the end
                // of output permits settlement below.
                Output::Closed => self.output_closed = true,
            }
        }
        // The first output may have released held typeahead.
        self.pty.flush()?;
        changed |= self.retire();
        match self.pty.try_exit() {
            Ok(phase) => {
                self.exit = Some(phase);
                changed = true;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                // The waiter is gone without a status: no code is guessed.
                if self.exit.is_none() {
                    self.exit = Some(Phase::Exited {
                        code: None,
                        signal: None,
                    });
                }
            }
        }
        if self.output_closed {
            if let Some(exit) = self.exit.take() {
                self.phase = match self.failure.take() {
                    Some(failure) => Phase::Failed(failure),
                    None => exit,
                };
                self.pending.clear();
                self.retained_input = 0;
                changed = true;
            }
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

    /// Acknowledge every entry the master has fully taken. Marks rise
    /// with the sequence, so delivery retires a prefix. `true` when the
    /// acknowledgment advanced.
    fn retire(&mut self) -> bool {
        let delivered = self.pty.delivered();
        let before = self.acknowledged;
        while let Some(pending) = self.pending.front() {
            if pending.mark > delivered {
                break;
            }
            self.retained_input -= pending.bytes;
            self.acknowledged = pending.sequence;
            self.pending.pop_front();
        }
        self.acknowledged != before
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
        if self.phase.live() {
            self.pty.terminate();
        }
    }
}
