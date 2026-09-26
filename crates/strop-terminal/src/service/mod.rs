//! Bounded frontend handle. The PTY lives on the namespace's worker (0058
//! WK12); the VT and session state stay on this crate's service thread.
//! Callers only enqueue intents or take immutable updates.
mod mailbox;
mod worker;
use crate::{
    launch::Launch,
    model::{Geometry, Phase, SessionId, Update, MAX_INPUT_BYTES},
    Error,
};
use mailbox::Mailbox;
use std::{
    io,
    os::unix::net::UnixDatagram,
    sync::{
        atomic::{AtomicU8, AtomicUsize, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc,
    },
};
use strop_core::{
    frontend_input::Input,
    worker::{CancelHandle, CancelReason, Outcome},
};

const MAX_COMMANDS: usize = 64;
const STARTING: u8 = 0;
const RUNNING: u8 = 1;
const CLOSING: u8 = 2;
const FINISHED: u8 = 3;
#[derive(Default)]
struct Budget {
    bytes: AtomicUsize,
    count: AtomicUsize,
    lifecycle: AtomicU8,
}
struct Permit {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.budget.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
        self.budget.count.fetch_sub(1, Ordering::AcqRel);
    }
}
impl Budget {
    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Permit, Error> {
        if self
            .count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_COMMANDS).then_some(count + 1)
            })
            .is_err()
        {
            return Err(Error::InputFull);
        }
        if self
            .bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                held.checked_add(bytes)
                    .filter(|sum| *sum <= MAX_INPUT_BYTES)
            })
            .is_err()
        {
            self.count.fetch_sub(1, Ordering::AcqRel);
            return Err(Error::InputFull);
        }
        Ok(Permit {
            budget: self.clone(),
            bytes,
        })
    }
}
enum Intent {
    Input { value: Input, confirmed: bool },
    Resize(Geometry),
    PasteDecision { ticket: u64, accept: bool },
    Focus(bool),
}
struct Request {
    intent: Intent,
    permit: Permit,
}

pub struct Service {
    sender: SyncSender<Request>,
    wake: UnixDatagram,
    budget: Arc<Budget>,
    mailbox: Arc<Mailbox>,
    cancel: Option<CancelHandle>,
    stopped: bool,
}
impl Service {
    /// Start one session behind the namespace's worker lease (0058
    /// WK12): the PTY spawns on that worker, and the lease's stream
    /// hook nudges this service's wake on output. The caller (the
    /// engine) selected the namespace; the lease's admitted
    /// capabilities decide whether a PTY is possible — a refusal is
    /// typed, never a guessed local launch.
    pub fn start(
        session: SessionId,
        worker: strop_worker_client::Worker,
        launch: Launch,
        geometry: Geometry,
        keyboard: u8,
        palette: Option<crate::model::Palette>,
        notify: impl Fn(SessionId) + Send + Sync + 'static,
    ) -> Result<Self, Error> {
        launch.validate()?;
        if !geometry.valid() {
            return Err(Error::Capacity("terminal geometry"));
        }
        if keyboard & !crate::model::SUPPORTED_KEYBOARD_FLAGS != 0 {
            return Err(Error::Unavailable(
                "unsupported frontend keyboard capabilities".into(),
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(MAX_COMMANDS);
        let (wake, listening) =
            UnixDatagram::pair().map_err(|error| io_error("create terminal worker wake", error))?;
        wake.set_nonblocking(true)
            .map_err(|error| io_error("configure terminal worker wake", error))?;
        listening
            .set_nonblocking(true)
            .map_err(|error| io_error("configure terminal worker wake", error))?;
        crate::client::install_wake_hook(&worker);
        let nudge = wake
            .try_clone()
            .map_err(|error| io_error("retain terminal stream wake", error))?;
        let mailbox = Arc::new(Mailbox::new(session, notify));
        let budget = Arc::new(Budget::default());
        let outcome_mailbox = mailbox.clone();
        let outcome_budget = budget.clone();
        let work = worker::Start {
            session,
            worker,
            launch,
            geometry,
            keyboard,
            palette,
            receiver,
            wake: listening,
            nudge,
            mailbox: mailbox.clone(),
            budget: budget.clone(),
        };
        let cancel = strop_core::worker::spawn_effect(
            "terminal",
            move |outcome: Outcome<Update>| {
                let update = match outcome {
                    Outcome::Success(update) => update,
                    Outcome::Failed { failure, partial } => {
                        let mut update = partial.unwrap_or_else(|| {
                            empty_update(session, Phase::Failed(failure.message.clone()), None)
                        });
                        update.phase = Phase::Failed(failure.message);
                        update
                    }
                    Outcome::Cancelled(_) => empty_update(
                        session,
                        Phase::Exited {
                            code: None,
                            signal: None,
                        },
                        None,
                    ),
                };
                outcome_mailbox.publish_with(update, || {
                    outcome_budget.lifecycle.store(FINISHED, Ordering::Release)
                });
            },
            move |token| Outcome::Success(work.run(token)),
        );
        Ok(Self {
            sender,
            wake,
            budget,
            mailbox,
            cancel: Some(cancel),
            stopped: false,
        })
    }

    pub fn input(&self, value: Input, confirmed: bool) -> Result<(), Error> {
        let bytes = match &value {
            Input::Key(_) => 0,
            Input::Text(text) | Input::Paste(text) => text.capacity(),
        };
        if bytes > MAX_INPUT_BYTES {
            return Err(Error::Capacity("terminal input"));
        }
        self.send(Intent::Input { value, confirmed }, bytes)
    }
    pub fn resize(&self, geometry: Geometry) -> Result<(), Error> {
        if !geometry.valid() {
            return Err(Error::Capacity("terminal geometry"));
        }
        self.send(Intent::Resize(geometry), 0)
    }
    pub fn decide_paste(&self, ticket: u64, accept: bool) -> Result<(), Error> {
        self.send(Intent::PasteDecision { ticket, accept }, 0)
    }
    pub fn focus(&self, focused: bool) -> Result<(), Error> {
        self.send(Intent::Focus(focused), 0)
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        self.budget.lifecycle.fetch_max(CLOSING, Ordering::AcqRel);
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel(CancelReason::OwnerClosed);
        }
    }
    pub fn pending(&self) -> bool {
        matches!(
            self.budget.lifecycle.load(Ordering::Acquire),
            STARTING | CLOSING
        ) || self.budget.count.load(Ordering::Acquire) != 0
            || self.mailbox.pending()
    }
    /// Consume only after the corresponding notification; a contended slot
    /// requeues that notification so the frontend never waits on publication.
    pub fn take_update(&self) -> Option<Update> {
        self.mailbox.take()
    }

    fn send(&self, intent: Intent, bytes: usize) -> Result<(), Error> {
        if self.stopped {
            return Err(Error::Closed);
        }
        let permit = self.budget.reserve(bytes)?;
        match self.sender.try_send(Request { intent, permit }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(Error::InputFull),
            Err(TrySendError::Disconnected(_)) => return Err(Error::Closed),
        }
        match self.wake.send(&[1]) {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(io_error("wake terminal after input admission", error)),
        }
    }
}
fn empty_update(session: SessionId, phase: Phase, warning: Option<String>) -> Update {
    Update {
        session,
        phase,
        frame: None,
        effects: Vec::new(),
        acknowledged_input: 0,
        warning,
    }
}
fn io_error(operation: &'static str, error: io::Error) -> Error {
    Error::Io {
        operation,
        detail: error.to_string(),
    }
}
