//! The serve scheduler (0058 WK11): one session's budget registry and
//! two-lane outbound scheduler.
//!
//! ## Budgets
//!
//! Every admitted request is classified ([`RequestClass`]) and counted:
//! the total in-flight ceiling is `max_pending_requests`, of which
//! `control_reserve` slots are held for control-class requests so
//! health/quiesce/cancellation admission never queues behind bulk work;
//! bulk streaming reads additionally draw from `max_concurrent_reads`.
//! Exec leases and notify subscriptions keep their own registries
//! ([`Scheduler::acquire_exec`], `NotifyManager`); admission refusals are
//! typed (`Refusal::Busy`), never silent queueing.
//!
//! ## Outbound lanes
//!
//! One writer thread drains two queues behind a single lock: the
//! **control** lane (outcomes, errors, events, `bye`) always drains
//! before the **data** lane (stream chunks), so a control frame never
//! waits behind queued bulk data. Per-stream chunk order is preserved —
//! a stream's chunks all ride the data lane in FIFO order, and its
//! opening envelope was pushed to the control lane before its first
//! chunk could exist. A terminal (`last`) chunk bypasses a full data
//! lane: EOF is never queued behind data, and the bypass is bounded by
//! the number of live streams. Data producers block (cancel-aware) when
//! the lane is full — backpressure lands on the producers, never on the
//! input loop or the control path.
//!
//! ## Settlement
//!
//! Every admitted request settles exactly once: the owning thread sends
//! its outcome (or its stream's terminal chunk) and calls
//! [`Scheduler::settle`]. Cancellation never sends an outcome — it flips
//! the token and wakes data-lane waiters, so a cancelled bulk producer
//! blocked behind a full lane still terminates honestly (short stream,
//! never silent truncation). [`Scheduler::halt`] wakes every waiter;
//! afterwards pushes are dropped and the writer exits once the queues
//! drain — session death is the terminal disposition for anything still
//! in flight.
//!
//! ## Assurance seam
//!
//! The scheduler's own coordination types compile to loom's instrumented
//! `Mutex`/`Condvar` under `--cfg strop_loom` (with shrunken lane
//! capacities so exhaustive interleaving campaigns terminate); without
//! the cfg the identical source uses parking_lot and loom never ships
//! (0057 VF11/VF19 convention, riding the core-assurance lane).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};

use strop_core::worker::{CancelHandle, CancelReason, CancelToken};
use strop_worker_protocol::codec::StreamChunk;
#[cfg(test)]
use strop_worker_protocol::StreamId;
use strop_worker_protocol::{Limits, Refusal, RequestClass, RequestId, WorkerMessage};

/// The instrumented-sync seam (0057 VF11 convention): parking_lot's lock
/// returns the guard directly, loom's a LockResult — both produce it here.
#[cfg(not(strop_loom))]
type StateLock = parking_lot::Mutex<State>;
#[cfg(strop_loom)]
type StateLock = loom::sync::Mutex<State>;
#[cfg(not(strop_loom))]
type StateGuard<'a> = parking_lot::MutexGuard<'a, State>;
#[cfg(strop_loom)]
type StateGuard<'a> = loom::sync::MutexGuard<'a, State>;
#[cfg(not(strop_loom))]
type Signal = parking_lot::Condvar;
#[cfg(strop_loom)]
type Signal = loom::sync::Condvar;

#[cfg(not(strop_loom))]
fn lock(scheduler: &Scheduler) -> StateGuard<'_> {
    scheduler.state.lock()
}
#[cfg(strop_loom)]
fn lock(scheduler: &Scheduler) -> StateGuard<'_> {
    scheduler
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// parking_lot waits in place on a &mut guard; loom consumes and returns
/// it. Both produce the post-wait guard here.
#[cfg(not(strop_loom))]
fn wait<'a>(signal: &Signal, mut state: StateGuard<'a>) -> StateGuard<'a> {
    signal.wait(&mut state);
    state
}
#[cfg(strop_loom)]
fn wait<'a>(signal: &Signal, state: StateGuard<'a>) -> StateGuard<'a> {
    signal
        .wait(state)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The default outbound data lane capacity in chunks (16 MiB at the 64
/// KiB read chunk), restated on the wire as `max_queued_data_chunks`.
/// Calibration parameter: under `--cfg strop_loom` it shrinks so
/// exhaustive campaigns terminate; the admission logic is identical.
#[cfg(not(strop_loom))]
pub(crate) const DATA_LANE_CHUNKS: usize = 256;
#[cfg(strop_loom)]
pub(crate) const DATA_LANE_CHUNKS: usize = 2;

/// The registered per-class bounds for one session (WK11). Derived from
/// the negotiated [`Limits`] at session start; never guessed per call.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Budgets {
    /// Total admitted in-flight requests.
    max_pending: usize,
    /// Slots held for control-class requests.
    control_reserve: usize,
    /// Concurrent bulk streaming reads.
    max_reads: usize,
    /// Live exec leases.
    max_execs: usize,
    /// Outbound data lane capacity in chunks.
    data_chunks: usize,
    /// Outbound control lane capacity in frames.
    control_frames: usize,
}

impl Budgets {
    pub(crate) fn from_limits(limits: &Limits) -> Self {
        Budgets {
            max_pending: limits.max_pending_requests,
            control_reserve: limits.control_reserve,
            max_reads: limits.max_concurrent_reads,
            max_execs: limits.max_exec_processes,
            data_chunks: limits.max_queued_data_chunks,
            // Results are bounded by admitted requests; event batches are
            // bounded per subscription by the notify manager's own queues.
            control_frames: limits.max_pending_requests + 4 * limits.max_subscriptions + 8,
        }
    }
}

/// One frame the writer thread drains.
#[cfg_attr(test, derive(Debug))]
pub(crate) enum Outbound {
    /// Outcomes, errors, events, `bye`: always drained before data.
    Control(WorkerMessage),
    /// One stream chunk, per-stream FIFO.
    Chunk(StreamChunk),
}

/// How a chunk push resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PushChunk {
    Enqueued,
    /// The request's token was cancelled while waiting for lane room.
    Cancelled,
    /// The session is halted; the frame was dropped.
    Halted,
}

/// One admitted, not-yet-settled request.
struct Slot {
    class: RequestClass,
    handle: Option<CancelHandle>,
}

struct State {
    /// The admission registry: admitted and not yet settled.
    requests: HashMap<RequestId, Slot>,
    /// In-flight bulk reads (also counted in `requests`).
    bulk: usize,
    /// Live exec leases.
    execs: usize,
    control: VecDeque<WorkerMessage>,
    data: VecDeque<StreamChunk>,
    /// Written, not merely popped: an exec exit must follow its final
    /// output bytes on the actual transport despite control priority.
    queued_data: u64,
    written_data: u64,
    halted: bool,
}

/// The session scheduler: admission registry, per-class budgets and the
/// two outbound lanes. See the module docs for the contract.
pub(crate) struct Scheduler {
    budgets: Budgets,
    state: StateLock,
    /// Signalled when a frame lands for the writer (or on halt).
    control_ready: Signal,
    /// Signalled when the writer frees data-lane room (or on cancel/halt).
    data_room: Signal,
    /// Signalled when the writer frees control-lane room (or on halt).
    control_room: Signal,
    /// Wakes an exec whose output has reached the actual writer.
    data_written: Signal,
}

impl Scheduler {
    pub(crate) fn new(budgets: Budgets) -> Self {
        Scheduler {
            budgets,
            state: StateLock::new(State {
                requests: HashMap::new(),
                bulk: 0,
                execs: 0,
                control: VecDeque::new(),
                data: VecDeque::new(),
                queued_data: 0,
                written_data: 0,
                halted: false,
            }),
            control_ready: Signal::new(),
            data_room: Signal::new(),
            control_room: Signal::new(),
            data_written: Signal::new(),
        }
    }

    /// Admit one request into the registry. Control draws from the
    /// reserved slots; standard/bulk classes stop `control_reserve` short
    /// of the ceiling so control admission is never starved; bulk reads
    /// additionally draw from the read budget. Overflow is a typed
    /// refusal, never silent queueing.
    pub(crate) fn admit(
        &self,
        id: RequestId,
        class: RequestClass,
        handle: CancelHandle,
    ) -> Result<(), Refusal> {
        let mut state = lock(self);
        let total = state.requests.len();
        let admitted = match class {
            RequestClass::Control => total < self.budgets.max_pending,
            RequestClass::Standard => {
                total
                    < self
                        .budgets
                        .max_pending
                        .saturating_sub(self.budgets.control_reserve)
            }
            RequestClass::Bulk => {
                total
                    < self
                        .budgets
                        .max_pending
                        .saturating_sub(self.budgets.control_reserve)
                    && state.bulk < self.budgets.max_reads
            }
        };
        if !admitted {
            return Err(Refusal::Busy {
                message: format!("{class:?} admission bound reached"),
            });
        }
        if class == RequestClass::Bulk {
            state.bulk += 1;
        }
        state.requests.insert(
            id,
            Slot {
                class,
                handle: Some(handle),
            },
        );
        Ok(())
    }

    /// Cancel one admitted request: flips its token and wakes data-lane
    /// waiters so a bulk producer blocked behind a full lane terminates.
    /// Never sends an outcome — settlement stays with the owning thread.
    /// Returns false when the id already settled (a no-op, not an error).
    pub(crate) fn cancel(&self, id: RequestId) -> bool {
        let handle = {
            let mut state = lock(self);
            state
                .requests
                .get_mut(&id)
                .and_then(|slot| slot.handle.take())
        };
        let Some(handle) = handle else { return false };
        handle.cancel(CancelReason::Dismissed);
        self.data_room.notify_all();
        true
    }

    /// Settle one admitted request exactly once, at thread end. Settling
    /// an unknown id is a scheduler bug, not a race outcome.
    pub(crate) fn settle(&self, id: RequestId) {
        let mut state = lock(self);
        let slot = state
            .requests
            .remove(&id)
            .unwrap_or_else(|| panic!("double settlement of request {}", id.0));
        if slot.class == RequestClass::Bulk {
            state.bulk -= 1;
        }
    }

    /// Admitted and not yet settled; the teardown drain waits on this.
    pub(crate) fn outstanding(&self) -> usize {
        lock(self).requests.len()
    }

    /// Acquire one exec lease slot atomically (the check and the count
    /// share one lock — no check-then-insert race). Released exactly
    /// where the exec registry entry is removed.
    pub(crate) fn acquire_exec(&self) -> Result<(), Refusal> {
        let mut state = lock(self);
        if state.execs >= self.budgets.max_execs {
            return Err(Refusal::Busy {
                message: "exec process bound reached".into(),
            });
        }
        state.execs += 1;
        Ok(())
    }

    pub(crate) fn release_exec(&self) {
        let mut state = lock(self);
        debug_assert!(state.execs > 0, "exec budget release without acquire");
        state.execs = state.execs.saturating_sub(1);
    }

    /// Push one control frame. Control frames are small and bounded by
    /// admission; a full lane back-pressures the pusher (which the writer
    /// drains first), never the reader loop. Returns false once halted.
    pub(crate) fn push_control(&self, frame: WorkerMessage) -> bool {
        let mut state = lock(self);
        loop {
            if state.halted {
                return false;
            }
            if state.control.len() < self.budgets.control_frames {
                state.control.push_back(frame);
                self.control_ready.notify_one();
                return true;
            }
            state = wait(&self.control_room, state);
        }
    }

    /// Push one stream chunk, blocking (cancel-aware) while the data lane
    /// is full. Terminal (`last`) chunks bypass the bound: EOF is never
    /// queued behind data. The bypass is bounded by the live stream
    /// count. A cancelled token or a halted session abandons the push.
    pub(crate) fn push_chunk(&self, chunk: StreamChunk, token: Option<&CancelToken>) -> PushChunk {
        let mut state = lock(self);
        loop {
            if state.halted {
                return PushChunk::Halted;
            }
            if token.is_some_and(CancelToken::is_cancelled) {
                return PushChunk::Cancelled;
            }
            if chunk.last || state.data.len() < self.budgets.data_chunks {
                state.data.push_back(chunk);
                state.queued_data += 1;
                self.control_ready.notify_one();
                return PushChunk::Enqueued;
            }
            state = wait(&self.data_room, state);
        }
    }

    /// The writer's next frame: control before data. In particular, an
    /// opening `ReadOpened`/`ExecStarted` envelope must reach the wire
    /// before its first chunk, even after many other control frames.
    /// Per-stream data remains FIFO.
    pub(crate) fn pop(&self) -> Option<Outbound> {
        let mut state = lock(self);
        loop {
            if let Some(frame) = state.control.pop_front() {
                self.control_room.notify_all();
                return Some(Outbound::Control(frame));
            }
            if let Some(chunk) = state.data.pop_front() {
                self.data_room.notify_all();
                return Some(Outbound::Chunk(chunk));
            }
            if state.halted {
                return None;
            }
            state = wait(&self.control_ready, state);
        }
    }

    /// The writer reports completion after the chunk's frame actually
    /// reaches the transport, not when it leaves the in-memory lane.
    pub(crate) fn chunk_written(&self) {
        let mut state = lock(self);
        state.written_data += 1;
        self.data_written.notify_all();
    }

    /// A child exit cannot overtake its stdout/stderr terminal chunks.
    /// Called only after its output pumps have joined: all their chunks
    /// are in the data lane. Wait off the reader/input path for the
    /// writer to flush that finite prefix, or abandon on session death.
    pub(crate) fn flush_data(&self, stop: &AtomicBool) -> bool {
        let mut state = lock(self);
        let target = state.queued_data;
        while !state.halted && !stop.load(Ordering::Acquire) && state.written_data < target {
            state = wait(&self.data_written, state);
        }
        !state.halted && !stop.load(Ordering::Acquire)
    }

    /// End the session: wake every waiter. Producers abandon, the writer
    /// drains what is queued and exits; anything still in flight settles
    /// by session death.
    pub(crate) fn halt(&self) {
        let mut state = lock(self);
        state.halted = true;
        self.control_ready.notify_all();
        self.data_room.notify_all();
        self.control_room.notify_all();
        self.data_written.notify_all();
    }

    /// Wake data-lane waiters without cancelling a specific request
    /// (session stop): blocked producers re-check their token/halt.
    pub(crate) fn wake_producers(&self) {
        self.data_room.notify_all();
        self.data_written.notify_all();
    }

    /// Test/assurance access: queued lane depths.
    #[cfg(all(test, not(strop_loom)))]
    fn lanes(&self) -> (usize, usize) {
        let state = lock(self);
        (state.control.len(), state.data.len())
    }

    /// Test/assurance access: one stream's queued chunk order.
    #[cfg(test)]
    fn queued_data_sequences(&self, stream: StreamId) -> Vec<u64> {
        lock(self)
            .data
            .iter()
            .filter(|chunk| chunk.stream == stream)
            .map(|chunk| chunk.sequence)
            .collect()
    }
}

#[cfg(all(test, strop_loom))]
mod loom_tests;
#[cfg(all(test, not(strop_loom)))]
mod tests;
