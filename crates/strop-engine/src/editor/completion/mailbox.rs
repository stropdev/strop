//! Native producers retain the last owner of every delivered snapshot. Input
//! takes cheap Arc leases; supersession and final destruction stay native.
mod prepared;
#[cfg(test)]
mod tests;
use super::model::{CompletionDelivery, DeliveryScope};
use crate::editor::events::{AppEvent, EventSender};
use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use strop_core::worker::{Outcome, WorkerId};

const PROVIDER_SLOTS: usize = 5;
const MAX_RETAINED_DELIVERIES: usize = 16;
const MAX_RETAINED_BYTES: usize = 24 * 1024 * 1024;

struct Retained {
    delivery: Arc<CompletionDelivery>,
    bytes: usize,
}
struct Slot {
    retained: Retained,
    observed: bool,
}
struct State {
    slots: [Option<Slot>; PROVIDER_SLOTS],
    retired: Vec<Retained>,
    prepared: Option<prepared::Ready>,
    bytes: usize,
    high_water_bytes: usize,
    destroyed: u64,
    capacity_changed: bool,
    stopped: Option<Outcome<()>>,
    wake: Option<EventSender>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            retired: Vec::with_capacity(MAX_RETAINED_DELIVERIES),
            prepared: None,
            bytes: 0,
            high_water_bytes: 0,
            destroyed: 0,
            capacity_changed: false,
            stopped: None,
            wake: None,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct CompletionDrain {
    pub deliveries: Vec<Arc<CompletionDelivery>>,
    pub capacity_changed: bool,
    pub stopped: Option<Outcome<()>>,
    pub usage: PublicationUsage,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct PublicationUsage {
    pub retained_deliveries: usize,
    pub charged_bytes: usize,
    pub high_water_bytes: usize,
    pub destroyed: u64,
}

#[derive(Default)]
pub(super) struct CompletionMailbox {
    query: AtomicU64,
    selection: AtomicU64,
    preparation: AtomicU64,
    closed: AtomicBool,
    state: Mutex<State>,
    available: Condvar,
}
impl CompletionMailbox {
    pub fn connect(&self, sender: EventSender) {
        self.state.lock().wake = Some(sender);
    }

    pub fn query(&self, request: Option<WorkerId>) {
        self.query
            .store(request.map_or(0, WorkerId::get), Ordering::Release);
        self.selection.store(0, Ordering::Release);
        self.preparation.store(0, Ordering::Release);
        self.released();
    }
    pub fn select(&self, selection: Option<WorkerId>) {
        self.selection
            .store(selection.map_or(0, WorkerId::get), Ordering::Release);
        self.preparation.store(0, Ordering::Release);
        self.released();
    }
    pub fn prepare(&self, work: WorkerId) {
        self.preparation.store(work.get(), Ordering::Release);
        self.released();
    }
    pub fn owns(&self, query: WorkerId, scope: DeliveryScope) -> bool {
        !self.closed.load(Ordering::Acquire)
            && self.query.load(Ordering::Acquire) == query.get()
            && match scope {
                DeliveryScope::Query => true,
                DeliveryScope::Selection(selection) => {
                    self.selection.load(Ordering::Acquire) == selection.get()
                }
                DeliveryScope::Preparation { selection, work } => {
                    self.selection.load(Ordering::Acquire) == selection.get()
                        && self.preparation.load(Ordering::Acquire) == work.get()
                }
            }
    }

    /// Native producer only. Publication may backpressure here, never in input.
    /// The five slots are newest provider states, not a queue of old suggestions.
    pub fn publish(&self, delivery: CompletionDelivery) -> bool {
        if !self.owns(delivery.query.request, delivery.scope) {
            return false;
        }
        let bytes = delivery.payload.charged_bytes();
        debug_assert!(bytes <= MAX_RETAINED_BYTES);
        let slot = delivery.payload.slot();
        let retained = Retained {
            delivery: Arc::new(delivery),
            bytes,
        };
        loop {
            self.retire_native();
            let mut state = self.state.lock();
            if !self.owns(retained.delivery.query.request, retained.delivery.scope) {
                return false;
            }
            if let Some(previous) = state.slots[slot].take() {
                state.retired.push(previous.retained);
                drop(state);
                continue;
            }
            let count = state.retired.len() + state.slots.iter().flatten().count();
            if count < MAX_RETAINED_DELIVERIES && state.bytes + bytes <= MAX_RETAINED_BYTES {
                state.bytes += bytes;
                state.high_water_bytes = state.high_water_bytes.max(state.bytes);
                state.slots[slot] = Some(Slot {
                    retained,
                    observed: false,
                });
                let wake = state.wake.clone();
                drop(state);
                wake_owner(wake);
                return true;
            }
            if state
                .retired
                .iter()
                .any(|entry| Arc::strong_count(&entry.delivery) == 1)
            {
                drop(state);
                continue;
            }
            self.available.wait(&mut state);
        }
    }

    /// A physical LSP slot became available (or its connection closed). This
    /// is a coalescible hint; retry admission rechecks the current real client.
    pub fn capacity(&self) {
        let mut state = self.state.lock();
        state.capacity_changed = true;
        let wake = state.wake.clone();
        drop(state);
        wake_owner(wake);
    }

    pub fn take(&self) -> CompletionDrain {
        let mut state = self.state.lock();
        let mut deliveries = Vec::new();
        for slot in state.slots.iter_mut().flatten() {
            if !slot.observed {
                slot.observed = true;
                deliveries.push(slot.retained.delivery.clone());
            }
        }
        CompletionDrain {
            deliveries,
            capacity_changed: std::mem::take(&mut state.capacity_changed),
            stopped: state.stopped.take(),
            usage: PublicationUsage {
                retained_deliveries: state.retired.len()
                    + state.slots.iter().flatten().count()
                    + usize::from(state.prepared.is_some()),
                charged_bytes: state.bytes,
                high_water_bytes: state.high_water_bytes,
                destroyed: state.destroyed,
            },
        }
    }

    /// Pair with releasing UI leases. Taking the lock closes the lost-wake race
    /// with a producer checking strong counts before its condvar wait.
    pub fn released(&self) {
        let _state = self.state.lock();
        self.available.notify_all();
    }

    /// Native only; detach under the lock, destroy outside it. Once retired, an
    /// Arc cannot acquire a new UI lease: only current slots are drainable.
    pub fn retire_native(&self) {
        loop {
            let mut state = self.state.lock();
            if state
                .prepared
                .as_ref()
                .is_some_and(|ready| !self.owns(ready.query, ready.scope))
            {
                let prepared = state.prepared.take();
                state.bytes -= prepared::PLAN_BYTES;
                drop(state);
                drop(prepared);
                self.available.notify_all();
                continue;
            }
            for index in 0..PROVIDER_SLOTS {
                let stale = state.slots[index].as_ref().is_some_and(|slot| {
                    !self.owns(
                        slot.retained.delivery.query.request,
                        slot.retained.delivery.scope,
                    )
                });
                if stale {
                    if let Some(slot) = state.slots[index].take() {
                        state.retired.push(slot.retained);
                    }
                }
            }
            let obsolete = state
                .retired
                .iter()
                .position(|entry| Arc::strong_count(&entry.delivery) == 1)
                .map(|index| state.retired.swap_remove(index));
            let Some(obsolete) = obsolete else { return };
            state.bytes -= obsolete.bytes;
            state.destroyed = state.destroyed.saturating_add(1);
            drop(state);
            drop(obsolete);
            self.available.notify_all();
        }
    }

    /// Called after native work/captured inputs have been dropped. The final
    /// owner remains here until the UI acknowledges the stopped worker and
    /// releases its leases; no large snapshot destructor moves onto input.
    pub fn finish_native(&self, outcome: Outcome<()>) {
        self.closed.store(true, Ordering::Release);
        let mut state = self.state.lock();
        state.stopped = Some(outcome);
        self.available.notify_all();
        let wake = state.wake.clone();
        drop(state);
        wake_owner(wake);
        loop {
            self.retire_native();
            let mut state = self.state.lock();
            if state.bytes == 0 {
                break;
            }
            if state
                .retired
                .iter()
                .any(|entry| Arc::strong_count(&entry.delivery) == 1)
            {
                drop(state);
                continue;
            }
            self.available.wait(&mut state);
        }
    }
}

fn wake_owner(sender: Option<EventSender>) {
    if let Some(sender) = sender {
        let _ = sender.send(AppEvent::Completion);
    }
}
