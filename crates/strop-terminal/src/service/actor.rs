use super::{
    empty_update, mailbox::Mailbox, Budget, Intent, Permit, Request, CLOSING, RUNNING, STARTING,
};
use crate::{
    client::Client,
    launch::Launch,
    model::{Geometry, Phase, SessionId, Update},
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
    pub target: crate::transport::Target,
    pub launch: Launch,
    pub geometry: Geometry,
    pub keyboard: u8,
    /// Embedder-owned default palette (0065 D3); `None` keeps the native
    /// engine's compiled-in defaults.
    pub palette: Option<crate::model::Palette>,
    pub receiver: Receiver<Request>,
    pub wake: UnixDatagram,
    /// Send-side wake clone the PTY's reader and waiter nudge on output
    /// and exit.
    pub nudge: UnixDatagram,
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
        let nudge = match self.nudge.try_clone() {
            Ok(nudge) => nudge,
            Err(error) => {
                return empty_update(
                    self.session,
                    Phase::Failed(
                        super::io_error("retain terminal output wake", error).to_string(),
                    ),
                    None,
                )
            }
        };
        let client = match Client::spawn(
            self.session,
            &self.target,
            &self.launch,
            self.geometry,
            crate::client::EmulationSettings {
                keyboard: self.keyboard,
                palette: self.palette.as_ref(),
            },
            nudge,
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
                // Actor destruction revokes the child before the effect
                // envelope publishes the failure.
                actor.status(Phase::Failed(message), None)
            }
        }
    }
}
struct Actor {
    client: Client,
    start: Start,
    pending: Option<Request>,
    inflight: VecDeque<(u64, Permit)>,
    acknowledged: u64,
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
            // Clear old nudges before checking the queues. Arrivals after
            // this drain retain their datagram until the next turn.
            drain_wake(&self.start.wake)?;
            let mut published = false;
            if let Some(update) = self.client.poll()? {
                published = true;
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
            let theme_request = self.start.budget.theme.load(Ordering::Acquire);
            if theme_request != 0 && self.client.phase() == &Phase::Running {
                let theme = strop_core::theme::ThemeId::ALL
                    .get(usize::from(theme_request - 1))
                    .copied()
                    .ok_or(Error::State("unregistered terminal theme request"))?;
                let update = self.client.set_theme(theme)?;
                self.start.mailbox.publish_with(update, || {
                    // A newer request racing publication remains pending.
                    let _ = self.start.budget.theme.compare_exchange(
                        theme_request,
                        0,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                });
                published = true;
            }
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
                let accepted = match &request.intent {
                    Intent::Input(value) => self.client.input(value),
                    Intent::Resize(geometry) => self.client.resize(*geometry),
                    Intent::Focus(focused) => self.client.focus(*focused),
                };
                match accepted {
                    Ok(sequence) => {
                        if let Some(request) = self.pending.take() {
                            if sequence <= self.acknowledged {
                                let update = self.status(self.client.phase().clone(), None);
                                self.start
                                    .mailbox
                                    .publish_with(update, || drop(request.permit));
                            } else {
                                self.inflight.push_back((sequence, request.permit));
                            }
                        }
                    }
                    Err(Error::InputFull) => {
                        filled_turn = false;
                        break;
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
            // A bounded output turn may have left chunks queued even if
            // all their arrival nudges were drained. Recheck before
            // parking; the 250 ms timeout is only wake-loss recovery.
            if !filled_turn && !published {
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
            Err(error) => return Err(super::io_error("read terminal wake", error)),
        }
    }
    Ok(())
}
