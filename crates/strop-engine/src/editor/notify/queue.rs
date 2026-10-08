//! The bounded landing zone between the worker's reader thread and the
//! event loop: records, the coalescing queue, and the reload job key.
use std::collections::VecDeque;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::Outcome;
use strop_worker_protocol::message::NotifyCoverage;
use strop_worker_protocol::{Event, NotifyHint, Subscription};
use strop_workspace::ResourceLocation;

/// Queued records awaiting a drain turn (0056 AR06 semantic bound for
/// this lane). A full lane latches the conservative rescan obligation.
const MAX_RECORDS: usize = 64;
/// Hint paths coalesced per record before promotion to overflow.
const MAX_HINTS: usize = 512;

/// One semantic notify record. Subscription establishment rides the same
/// queue so application order never depends on a cross-channel race.
#[derive(Serialize, Deserialize)]
pub(crate) enum Record {
    /// The subscribe job settled (typed outcome, never a guessed state).
    Settled(Outcome<SubscribedScope>),
    /// Advisory freshness hints for one subscription generation.
    Hints {
        subscription: Subscription,
        hints: Vec<NotifyHint>,
    },
    /// Native overflow/partial coverage: rescan obligation.
    Overflow { subscription: Subscription },
    /// The initial scan completed (the boundary is positional: FIFO
    /// order against the subscription's own hints).
    Boundary { subscription: Subscription },
    /// A remote scope's subscribe job settled (WK07): the root it
    /// belongs to travels with the outcome — subscription identity
    /// alone never decides placement.
    RemoteSettled {
        root: ResourceLocation,
        outcome: Outcome<SubscribedScope>,
    },
}

/// A settled subscription, recorded only in consented full-content replay.
#[derive(Serialize, Deserialize)]
pub(crate) struct SubscribedScope {
    pub subscription: Subscription,
    pub coverage: NotifyCoverage,
    /// Relative native path of this process's own trace output, if it
    /// lives below this watched root. Never suppress unrelated files.
    pub owned_trace: Option<Vec<u8>>,
}

#[derive(Default)]
struct QueueState {
    records: VecDeque<Record>,
    /// The queue itself overflowed: conservative rescan obligation.
    rescan: bool,
    owned_trace: Option<(Subscription, Vec<u8>)>,
}

/// The bounded landing zone between the worker's reader thread (via the
/// forwarding sink) and the event loop. Producers record state here, so
/// the `AppEvent::Notify` wake hint may legally coalesce.
pub(crate) struct NotifyQueue {
    state: Mutex<QueueState>,
}

impl NotifyQueue {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
        }
    }

    /// Ignore only this capture's own writes: tracing a notification
    /// would otherwise write another notification, forming an
    /// unbounded event/trace loop. Other hints remain untouched.
    pub(crate) fn set_owned_trace(&self, owned: Option<(Subscription, Vec<u8>)>) {
        self.state.lock().owned_trace = owned;
    }

    /// True when a meaningful record was queued and needs an app wake.
    pub(crate) fn push_event(&self, event: Event) -> bool {
        match event {
            Event::Notify {
                subscription,
                hints,
                ..
            } => self.push_hints(subscription, hints),
            Event::NotifyOverflow { subscription, .. } => {
                self.push_record(Record::Overflow { subscription });
                true
            }
            Event::ReconcileBoundary { subscription, .. } => {
                self.push_record(Record::Boundary { subscription });
                true
            }
            Event::ExecExit { .. } | Event::ExecInput { .. } => false,
        }
    }

    pub(crate) fn push_hints(
        &self,
        subscription: Subscription,
        mut hints: Vec<NotifyHint>,
    ) -> bool {
        let mut state = self.state.lock();
        if strop_trace::enabled() {
            if let Some((owner, path)) = &state.owned_trace {
                if *owner == subscription {
                    hints.retain(|hint| hint.path.as_slice() != path.as_slice());
                }
            }
        }
        if hints.is_empty() {
            return false;
        }
        if let Some(Record::Hints {
            subscription: tail_subscription,
            hints: tail,
        }) = state.records.back_mut()
        {
            if *tail_subscription == subscription && tail.len() + hints.len() <= MAX_HINTS {
                tail.append(&mut hints);
                return true;
            }
        }
        if hints.len() > MAX_HINTS {
            state.records.push_back(Record::Overflow { subscription });
            return true;
        }
        if state.records.len() >= MAX_RECORDS {
            state.rescan = true;
            return true;
        }
        state.records.push_back(Record::Hints {
            subscription,
            hints,
        });
        true
    }

    pub(crate) fn push_record(&self, record: Record) {
        let mut state = self.state.lock();
        if state.records.len() >= MAX_RECORDS {
            state.rescan = true;
            return;
        }
        state.records.push_back(record);
    }

    /// Requeue deferred records at the front (a pending subscribe must
    /// see the hints that raced it, in order).
    pub(super) fn requeue_front(&self, records: Vec<Record>) {
        let mut state = self.state.lock();
        let room = MAX_RECORDS.saturating_sub(state.records.len());
        if records.len() > room {
            state.rescan = true;
        }
        for record in records.into_iter().take(room).rev() {
            state.records.push_front(record);
        }
    }

    /// Drain everything recorded so far plus the rescan latch.
    pub(crate) fn drain(&self) -> (Vec<Record>, bool) {
        let mut state = self.state.lock();
        (
            state.records.drain(..).collect(),
            std::mem::take(&mut state.rescan),
        )
    }
}

/// A guarded clean-buffer reload's identity (0058 §2 publication rule).

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReloadKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    #[serde(with = "strop_core::path_serde")]
    pub path: std::path::PathBuf,
}
