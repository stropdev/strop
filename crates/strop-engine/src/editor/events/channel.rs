//! Event transport, not another reducer. Both the priority input lane and native
//! job lane wake the owning driver; park/unpark's retained token prevents a lost
//! wake between observing an empty queue and parking.
use super::AppEvent;
use std::sync::mpsc::{self, Receiver, SendError};

#[derive(Clone)]
pub struct EventSender {
    sender: mpsc::Sender<AppEvent>,
    owner: std::thread::Thread,
}
impl EventSender {
    pub fn send(&self, event: AppEvent) -> Result<(), Box<SendError<AppEvent>>> {
        self.sender.send(event).map_err(Box::new)?;
        self.owner.unpark();
        Ok(())
    }
}
pub fn channel() -> (EventSender, Receiver<AppEvent>) {
    let (sender, receiver) = mpsc::channel();
    (
        EventSender {
            sender,
            owner: std::thread::current(),
        },
        receiver,
    )
}

/// Fairness limits apply between events; expensive work itself belongs on a worker.
pub const EVENTS_PER_TURN: usize = 32;
pub const TURN_BUDGET: std::time::Duration = std::time::Duration::from_millis(2);
