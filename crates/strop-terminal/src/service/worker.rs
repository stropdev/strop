use super::{
    empty_update, mailbox::Mailbox, Budget, Intent, Permit, Request, CLOSING, RUNNING, STARTING,
};
use crate::{
    client::Client,
    launch::Launch,
    model::{Effect, Geometry, Phase, SessionId, Update},
    Error,
};
use std::{
    collections::VecDeque,
    io,
    os::unix::net::UnixDatagram,
    sync::{
        atomic::Ordering,
        mpsc::{Receiver, TryRecvError},
        Arc,
    },
};
use strop_core::worker::CancelToken;

pub(super) struct Start {
    pub session: SessionId,
    pub launch: Launch,
    pub geometry: Geometry,
    pub keyboard: u8,
    /// Embedder-owned default palette (0065 D3); `None` keeps the native
    /// engine's compiled-in defaults.
    pub palette: Option<crate::model::Palette>,
    pub receiver: Receiver<Request>,
    pub wake: UnixDatagram,
    pub mailbox: Arc<Mailbox>,
    pub budget: Arc<Budget>,
}
impl Start {
    pub fn run(self, token: CancelToken) -> Update {
        if token.is_cancelled() {
            return empty_update(
                self.session,
                Phase::Exited {
                    code: None,
                    signal: None,
                },
                None,
            );
        }
        self.mailbox
            .publish(empty_update(self.session, Phase::Starting, None));
        let client = match Client::spawn(
            self.session,
            &self.launch,
            self.geometry,
            self.keyboard,
            self.palette.as_ref(),
            &token,
        ) {
            Ok(client) => client,
            Err(_) if token.is_cancelled() => {
                return empty_update(
                    self.session,
                    Phase::Exited {
                        code: None,
                        signal: None,
                    },
                    None,
                )
            }
            Err(error) => {
                return empty_update(self.session, Phase::Failed(error.to_string()), None)
            }
        };
        let mut actor = Actor {
            client,
            start: self,
            pending: None,
            inflight: VecDeque::new(),
            acknowledged: 0,
            held_paste: None,
            confirmation: 0,
        };
        match actor.drive() {
            Ok(update) => update,
            Err(error) => {
                actor
                    .start
                    .budget
                    .lifecycle
                    .fetch_max(CLOSING, Ordering::AcqRel);
                let message = error.to_string();
                actor
                    .start
                    .mailbox
                    .publish(actor.status(Phase::Closing, Some(message.clone())));
                // Actor destruction closes both socket halves before joining
                // the helper. The effect envelope publishes only after that.
                actor.status(Phase::Failed(message), None)
            }
        }
    }
}
struct HeldPaste {
    ticket: u64,
    value: strop_core::frontend_input::Input,
}
struct Actor {
    client: Client,
    start: Start,
    pending: Option<Request>,
    inflight: VecDeque<(u64, Permit)>,
    acknowledged: u64,
    held_paste: Option<HeldPaste>,
    confirmation: u64,
}
impl Drop for Actor {
    fn drop(&mut self) {
        // Runs before native destruction and before queued permits drop, also
        // during panic. Quiescence cannot overtake final outcome publication.
        self.start
            .budget
            .lifecycle
            .fetch_max(CLOSING, Ordering::AcqRel);
    }
}
impl Actor {
    fn drive(&mut self) -> Result<Update, Error> {
        loop {
            if let Some(update) = self.client.poll()? {
                self.acknowledged = update.acknowledged_input;
                if !update.phase.live() {
                    return Ok(update);
                }
                if update.phase == Phase::Closing {
                    self.start
                        .budget
                        .lifecycle
                        .fetch_max(CLOSING, Ordering::AcqRel);
                }
                let running = update.phase == Phase::Running;
                self.start.mailbox.publish_with(update, || {
                    while self
                        .inflight
                        .front()
                        .is_some_and(|(sequence, _)| *sequence <= self.acknowledged)
                    {
                        self.inflight.pop_front();
                    }
                    if running {
                        let _ = self.start.budget.lifecycle.compare_exchange(
                            STARTING,
                            RUNNING,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                    }
                });
            }
            drain_wake(&self.start.wake)?;
            let mut filled_turn = true;
            for _ in 0..32 {
                if self.pending.is_none() {
                    match self.start.receiver.try_recv() {
                        Ok(request) => self.pending = Some(request),
                        Err(TryRecvError::Empty) => {
                            filled_turn = false;
                            break;
                        }
                        Err(TryRecvError::Disconnected) => {
                            self.client.stop()?;
                            filled_turn = false;
                            break;
                        }
                    }
                }
                if self.client.phase() == &Phase::Closing {
                    self.pending.take();
                    continue;
                }
                if self.client.phase() != &Phase::Running {
                    filled_turn = false;
                    break;
                }
                let Some(request) = self.pending.as_ref() else {
                    break;
                };
                let admitted = match &request.intent {
                    Intent::Input { value, confirmed } => {
                        self.client.input(value, *confirmed).map(Some)
                    }
                    Intent::Resize(geometry) => self.client.resize(*geometry).map(Some),
                    Intent::Focus(focused) => self.client.focus(*focused).map(Some),
                    Intent::PasteDecision { ticket, accept } => {
                        match self
                            .held_paste
                            .as_ref()
                            .filter(|held| held.ticket == *ticket)
                        {
                            Some(held) if *accept => self.client.input(&held.value, true).map(Some),
                            Some(_) => Ok(None),
                            None => Err(Error::Unavailable(
                                "paste confirmation is stale or was cleared".into(),
                            )),
                        }
                    }
                };
                match admitted {
                    Ok(sequence) => {
                        if let Some(request) = self.pending.take() {
                            let clears = matches!(
                                request.intent,
                                Intent::PasteDecision { .. }
                                    | Intent::Input {
                                        value: strop_core::frontend_input::Input::Paste(_),
                                        ..
                                    }
                            );
                            let cleared = if clears {
                                self.held_paste.take().map(|held| held.ticket)
                            } else {
                                None
                            };
                            let mut update = self.status(self.client.phase().clone(), None);
                            if let Some(ticket) = cleared {
                                update
                                    .effects
                                    .push(Effect::PasteConfirmationCleared { ticket });
                            }
                            if let Some(sequence) = sequence {
                                self.inflight.push_back((sequence, request.permit));
                                if cleared.is_some() {
                                    self.start.mailbox.publish(update);
                                }
                            } else {
                                update.warning = Some("held terminal paste discarded".into());
                                self.start
                                    .mailbox
                                    .publish_with(update, || drop(request.permit));
                            }
                        }
                    }
                    Err(Error::InputFull) => {
                        filled_turn = false;
                        break;
                    }
                    Err(Error::PasteNeedsConfirmation) => {
                        self.confirmation = self
                            .confirmation
                            .checked_add(1)
                            .ok_or(Error::Capacity("paste confirmation IDs exhausted"))?;
                        let Some(request) = self.pending.take() else {
                            return Err(Error::State("missing paste admission"));
                        };
                        let Intent::Input { value, .. } = request.intent else {
                            return Err(Error::State("confirmed paste was rejected"));
                        };
                        self.held_paste = Some(HeldPaste {
                            ticket: self.confirmation,
                            value,
                        });
                        let mut update = self.status(self.client.phase().clone(), Some("paste held: Ctrl-\\ Ctrl-N, then :terminal-paste to confirm or :terminal-paste-cancel to discard".into()));
                        update.effects.push(Effect::PasteConfirmationRequested {
                            ticket: self.confirmation,
                        });
                        self.start
                            .mailbox
                            .publish_with(update, || drop(request.permit));
                    }
                    Err(error @ (Error::Native { .. } | Error::State(_) | Error::Protocol(_))) => {
                        return Err(error)
                    }
                    Err(error) => {
                        let update =
                            self.status(self.client.phase().clone(), Some(error.to_string()));
                        self.start.mailbox.publish_with(update, || {
                            self.pending.take();
                        });
                    }
                }
            }
            if !filled_turn {
                self.client.wait(&self.start.wake)?;
            }
        }
    }
    fn status(&self, phase: Phase, warning: Option<String>) -> Update {
        let mut update = empty_update(self.start.session, phase, warning);
        update.acknowledged_input = self.acknowledged;
        update
    }
}
fn drain_wake(wake: &UnixDatagram) -> Result<(), Error> {
    let mut bytes = [0; 64];
    for _ in 0..64 {
        match wake.recv(&mut bytes) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(super::io_error("read terminal worker wake", error)),
        }
    }
    Ok(())
}
