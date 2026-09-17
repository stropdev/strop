//! Bounded event transport, not another reducer (0056 AR06). Fairness
//! (EVENTS_PER_TURN/TURN_BUDGET) limits the time spent between events;
//! this lane limits what admission may retain. Two stream classes:
//!
//! - Wake hints (terminal updates, resize, focus, resume-input) coalesce:
//!   per-session dedup and latest-wins slots. A stale hint carries no
//!   state — the consumer re-reads the model — so dropping it is legal.
//! - Semantic events (input, paste, job results, durable outcomes) are
//!   never dropped or coalesced. Admission is bounded by count and by
//!   retained paste bytes; refusal hands the event back to its producer
//!   and is recorded for the UI to surface (`take_refusals`).
//!
//! Producers that may never lose an event (the native input reader: VT
//! bytes arrive as input/paste) use `send_blocking`: backpressure parks
//! the reader thread against the OS buffer, never a blocked UI send.
//! Both lanes wake the owning driver; the retained unpark token plus the
//! condvar close the lost-wake race between observing an empty queue
//! and parking.
//!
//! 0057 VF11: the Mutex/Condvar/Arc/Thread seam below compiles to
//! loom's instrumented types under `--cfg strop_loom` (shrunken bounds make
//! exhaustive interleaving campaigns practical); without the cfg the
//! identical source uses std types and loom never ships.
use super::AppEvent;
#[cfg(strop_loom)]
use loom::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::SendError;
#[cfg(not(strop_loom))]
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The unpark token of the owning driver thread (0057 VF11 seam).
#[cfg(not(strop_loom))]
type OwnerThread = std::thread::Thread;
#[cfg(strop_loom)]
type OwnerThread = loom::thread::Thread;

#[cfg(not(strop_loom))]
fn current_owner() -> OwnerThread {
    std::thread::current()
}
#[cfg(strop_loom)]
fn current_owner() -> OwnerThread {
    loom::thread::current()
}

/// Semantic-lane count bound: admitted events awaiting a turn. One turn
/// drains at most EVENTS_PER_TURN, so this is dozens of turns of backlog
/// — generous for bursty job results, fatal to unbounded retention.
/// The value is a calibration parameter: under `--cfg strop_loom` the seam
/// shrinks it so exhaustive interleaving campaigns terminate; the
/// admission logic is identical either way.
#[cfg(not(strop_loom))]
pub const MAX_SEMANTIC_EVENTS: usize = 1024;
#[cfg(strop_loom)]
pub const MAX_SEMANTIC_EVENTS: usize = 3;
/// Retained payload bound for admitted-but-undrained pastes. Paste is
/// the one semantic event whose size the producer chooses.
#[cfg(not(strop_loom))]
pub const MAX_QUEUED_PASTE_BYTES: usize = 8 * 1024 * 1024;
#[cfg(strop_loom)]
pub const MAX_QUEUED_PASTE_BYTES: usize = 8;
/// Newest refusals retained for the UI to summarize.
const REFUSAL_LOG: usize = 8;

/// A refused admission. Refusal before admission is visible (AR06):
/// the producer gets its event back AND the lane records the class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionRefusal {
    /// Stream class whose admission was refused.
    pub class: &'static str,
    /// Payload bytes the refused event carried.
    pub bytes: usize,
}

/// Why `try_recv` has no event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    Empty,
    Disconnected,
}

/// Why `recv_timeout` returned no event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvTimeoutError {
    Timeout,
    Disconnected,
}

impl std::fmt::Display for RecvTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => f.write_str("timed out waiting on event channel"),
            Self::Disconnected => f.write_str("event channel disconnected"),
        }
    }
}
impl std::error::Error for RecvTimeoutError {}

/// What the transport may do with an event (the AR06 stream policy).
enum Class {
    /// Coalescible wake/repaint hint: dedup or latest-wins.
    Hint,
    /// Never dropped, never coalesced; bounded admission.
    Semantic,
}

/// (class, refusal label, payload bytes) for one event.
fn classify(event: &AppEvent) -> (Class, &'static str, usize) {
    let semantic = |name: &'static str| (Class::Semantic, name, 0);
    match event {
        AppEvent::TerminalUpdate(_) => (Class::Hint, "terminal wake", 0),
        AppEvent::Resize { .. } => (Class::Hint, "resize", 0),
        AppEvent::Focus(_) => (Class::Hint, "focus", 0),
        AppEvent::ResumeInput => (Class::Hint, "resume input", 0),
        AppEvent::Paste(text) => (Class::Semantic, "paste", text.len()),
        AppEvent::Input(_) => semantic("input"),
        AppEvent::EditorKey(_) => semantic("editor key"),
        AppEvent::QuitIntent => semantic("quit intent"),
        AppEvent::Lsp(_) => semantic("lsp"),
        AppEvent::LspAttach(_) => semantic("lsp attach"),
        AppEvent::Shell(_) => semantic("shell"),
        AppEvent::Io(_) => semantic("io"),
        AppEvent::RemoteCompletion(_) => semantic("remote completion"),
        AppEvent::Container(_) => semantic("container"),
        AppEvent::Git(_) => semantic("git"),
        AppEvent::Picker(_) => semantic("picker"),
        AppEvent::PickerRanking(_) => semantic("picker ranking"),
        AppEvent::Analysis(_) => semantic("analysis"),
        AppEvent::Resolution(_) => semantic("resolution"),
        AppEvent::Preview(_) => semantic("preview"),
        AppEvent::Clipboard(_) => semantic("clipboard"),
    }
}

#[derive(Default)]
struct State {
    semantic: VecDeque<AppEvent>,
    paste_bytes: usize,
    terminal_wakes: VecDeque<strop_terminal::model::SessionId>,
    terminal_wake_set: HashSet<strop_terminal::model::SessionId>,
    resize: Option<(u16, u16)>,
    focus: Option<bool>,
    resume_input: bool,
    /// Lane alternation: under sustained load of both classes, pops
    /// alternate so neither input nor terminal output starves.
    semantic_turn: bool,
    senders: usize,
    receiver_live: bool,
    refusals: VecDeque<AdmissionRefusal>,
}

impl State {
    fn has_hint(&self) -> bool {
        !self.terminal_wakes.is_empty()
            || self.resize.is_some()
            || self.focus.is_some()
            || self.resume_input
    }

    fn pop_hint(&mut self) -> Option<AppEvent> {
        if let Some(session) = self.terminal_wakes.pop_front() {
            self.terminal_wake_set.remove(&session);
            return Some(AppEvent::TerminalUpdate(session));
        }
        if let Some((columns, rows)) = self.resize.take() {
            return Some(AppEvent::Resize { columns, rows });
        }
        if let Some(focused) = self.focus.take() {
            return Some(AppEvent::Focus(focused));
        }
        if self.resume_input {
            self.resume_input = false;
            return Some(AppEvent::ResumeInput);
        }
        None
    }

    fn pop_semantic(&mut self) -> Option<AppEvent> {
        let event = self.semantic.pop_front()?;
        if let AppEvent::Paste(text) = &event {
            self.paste_bytes = self.paste_bytes.saturating_sub(text.len());
        }
        Some(event)
    }

    /// Strict lane alternation when both classes are pending: neither
    /// input/cancellation nor terminal output can starve the other.
    fn pop(&mut self) -> Option<AppEvent> {
        match (self.semantic.is_empty(), self.has_hint()) {
            (false, true) => {
                self.semantic_turn = !self.semantic_turn;
                if self.semantic_turn {
                    self.pop_hint().or_else(|| self.pop_semantic())
                } else {
                    self.pop_semantic()
                }
            }
            (false, false) => self.pop_semantic(),
            (true, true) => self.pop_hint(),
            (true, false) => None,
        }
    }

    fn record_refusal(&mut self, class: &'static str, bytes: usize) {
        if self.refusals.len() == REFUSAL_LOG {
            self.refusals.pop_front();
        }
        self.refusals.push_back(AdmissionRefusal { class, bytes });
    }
}

struct Shared {
    state: Mutex<State>,
    /// Signalled when an event lands (receiver side).
    available: Condvar,
    /// Signalled when a pop frees semantic-lane room (blocking senders).
    space: Condvar,
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether a semantic event of `bytes` may be admitted. An empty lane
/// always admits: a single oversize paste must not strand input behind
/// its own bound.
fn has_room(state: &State, bytes: usize) -> bool {
    if state.semantic.is_empty() {
        return true;
    }
    state.semantic.len() < MAX_SEMANTIC_EVENTS
        && state.paste_bytes.saturating_add(bytes) <= MAX_QUEUED_PASTE_BYTES
}

/// Insert an admitted event, applying its class's coalescing policy.
fn insert(state: &mut State, event: AppEvent) {
    match event {
        AppEvent::TerminalUpdate(session) => {
            if state.terminal_wake_set.insert(session) {
                state.terminal_wakes.push_back(session);
            }
        }
        AppEvent::Resize { columns, rows } => state.resize = Some((columns, rows)),
        AppEvent::Focus(focused) => state.focus = Some(focused),
        AppEvent::ResumeInput => state.resume_input = true,
        event => {
            if let AppEvent::Paste(text) = &event {
                state.paste_bytes = state.paste_bytes.saturating_add(text.len());
            }
            state.semantic.push_back(event);
        }
    }
}

/// Clones share one bounded lane; the sender count tracks liveness so
/// the receiver observes disconnect exactly like the mpsc predecessor.
pub struct EventSender {
    shared: Arc<Shared>,
    owner: OwnerThread,
}

impl EventSender {
    /// Non-blocking admission (the default for every UI-side producer).
    /// A full semantic lane refuses: the producer gets its event back
    /// and the refusal is recorded for the UI to surface.
    pub fn send(&self, event: AppEvent) -> Result<(), Box<SendError<AppEvent>>> {
        let mut state = lock(&self.shared);
        if !state.receiver_live {
            return Err(Box::new(SendError(event)));
        }
        let (class, name, bytes) = classify(&event);
        if matches!(class, Class::Semantic) && !has_room(&state, bytes) {
            state.record_refusal(name, bytes);
            return Err(Box::new(SendError(event)));
        }
        insert(&mut state, event);
        drop(state);
        self.shared.available.notify_one();
        self.owner.unpark();
        Ok(())
    }

    /// Admission for native readers whose events may never be dropped
    /// (input/paste/quit). Backpressure parks the reader thread against
    /// the OS buffer — never a blocked UI send (AR06).
    pub fn send_blocking(&self, event: AppEvent) -> Result<(), Box<SendError<AppEvent>>> {
        let mut state = lock(&self.shared);
        let (class, _, bytes) = classify(&event);
        if matches!(class, Class::Hint) {
            insert(&mut state, event);
            drop(state);
            self.shared.available.notify_one();
            self.owner.unpark();
            return Ok(());
        }
        loop {
            if !state.receiver_live {
                return Err(Box::new(SendError(event)));
            }
            if has_room(&state, bytes) {
                insert(&mut state, event);
                drop(state);
                self.shared.available.notify_one();
                self.owner.unpark();
                return Ok(());
            }
            state = self
                .shared
                .space
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Refusals recorded since the last take (the visible-refusal
    /// ledger; the editor summarizes it into the status line).
    pub fn take_refusals(&self) -> Vec<AdmissionRefusal> {
        lock(&self.shared).refusals.drain(..).collect()
    }
}
impl Clone for EventSender {
    fn clone(&self) -> Self {
        lock(&self.shared).senders += 1;
        Self {
            shared: self.shared.clone(),
            owner: self.owner.clone(),
        }
    }
}

impl Drop for EventSender {
    fn drop(&mut self) {
        let mut state = lock(&self.shared);
        state.senders = state.senders.saturating_sub(1);
        if state.senders == 0 {
            // Wake the receiver so it observes the disconnect.
            drop(state);
            self.shared.available.notify_all();
        }
    }
}

pub struct EventReceiver {
    shared: Arc<Shared>,
}

impl EventReceiver {
    pub fn try_recv(&self) -> Result<AppEvent, TryRecvError> {
        let mut state = lock(&self.shared);
        if let Some(event) = state.pop() {
            drop(state);
            self.shared.space.notify_one();
            return Ok(event);
        }
        if state.senders == 0 {
            return Err(TryRecvError::Disconnected);
        }
        Err(TryRecvError::Empty)
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<AppEvent, RecvTimeoutError> {
        let deadline = Instant::now().checked_add(timeout);
        let mut state = lock(&self.shared);
        loop {
            if let Some(event) = state.pop() {
                drop(state);
                self.shared.space.notify_one();
                return Ok(event);
            }
            if state.senders == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let Some(deadline) = deadline else {
                return Err(RecvTimeoutError::Timeout);
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            let (guard, _) = self
                .shared
                .available
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = guard;
        }
    }

    /// Refusals recorded since the last take (receiver-side access for
    /// drivers that do not retain a sender).
    pub fn take_refusals(&self) -> Vec<AdmissionRefusal> {
        lock(&self.shared).refusals.drain(..).collect()
    }
}

impl Drop for EventReceiver {
    fn drop(&mut self) {
        let mut state = lock(&self.shared);
        state.receiver_live = false;
        drop(state);
        // Wake blocking native readers so they observe the close.
        self.shared.space.notify_all();
        self.shared.available.notify_all();
    }
}

pub fn channel() -> (EventSender, EventReceiver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            senders: 1,
            receiver_live: true,
            ..State::default()
        }),
        available: Condvar::new(),
        space: Condvar::new(),
    });
    (
        EventSender {
            shared: shared.clone(),
            owner: current_owner(),
        },
        EventReceiver { shared },
    )
}

/// Fairness limits apply between events; expensive work itself belongs on a worker.
pub const EVENTS_PER_TURN: usize = 32;
pub const TURN_BUDGET: std::time::Duration = std::time::Duration::from_millis(2);

/// 0057 VF11: Loom campaigns over the REAL bounded lane — the same
/// admission/drain control flow with loom's instrumented Mutex/Condvar/
/// Arc/Thread, at the shrunken cfg(strop_loom) bounds. Properties: no lost
/// wakeup, no lost or reordered semantic event, legal hint coalescing,
/// drain-before-disconnect, and close waking a parked blocking sender.
/// 0057 VF11: Loom campaigns over the REAL bounded lane. Note: the
/// receiver-side condvar park (recv_timeout) is NOT exercised here —
/// loom's wait_timeout never times out, so a parking receiver would
/// spin real time per branch; the identical park/notify race is proven
/// on the LSP wire queue's next_job drain (strop-lsp loom campaign),
/// and these campaigns target admission, coalescing, backpressure
/// wakeup and drain-before-disconnect.
#[cfg(all(test, strop_loom))]
mod loom_tests {
    use super::*;
    use strop_terminal::model::SessionId;

    #[test]
    fn loom_semantic_never_lost_hints_coalesce() {
        loom::model::Builder::new().preemption_bound(3).check(|| {
            let (tx, rx) = channel();
            let tx2 = tx.clone();
            let writer = loom::thread::spawn(move || {
                tx.send(AppEvent::QuitIntent).expect("lane has room");
                tx.send(AppEvent::Resize {
                    columns: 3,
                    rows: 3,
                })
                .expect("hint");
                tx.send(AppEvent::TerminalUpdate(SessionId::from_request(
                    strop_core::worker::WorkerId::new(1),
                )))
                .expect("hint");
                tx.send(AppEvent::Resize {
                    columns: 5,
                    rows: 5,
                })
                .expect("hint");
                1usize // one semantic event admitted
            });
            let paster = loom::thread::spawn(move || {
                tx2.send(AppEvent::Paste("abc".into()))
                    .expect("lane has room");
                1usize
            });
            let mut semantic = 0usize;
            let mut resizes = 0usize;
            let mut wakes = 0usize;
            loop {
                match rx.try_recv() {
                    Ok(AppEvent::Resize { .. }) => resizes += 1,
                    Ok(AppEvent::TerminalUpdate(_)) => wakes += 1,
                    Ok(_) => semantic += 1,
                    Err(TryRecvError::Empty) => loom::thread::yield_now(),
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            let admitted = writer.join().unwrap() + paster.join().unwrap();
            assert_eq!(
                semantic, admitted,
                "every admitted semantic event delivered exactly once"
            );
            assert!(
                (1..=2).contains(&resizes),
                "latest-wins coalescing: {resizes}"
            );
            assert_eq!(wakes, 1, "per-session wake dedup");
        });
    }

    #[test]
    fn loom_receiver_close_wakes_blocking_sender() {
        loom::model::Builder::new().preemption_bound(3).check(|| {
            let (tx, rx) = channel();
            for _ in 0..MAX_SEMANTIC_EVENTS {
                tx.send(AppEvent::QuitIntent).expect("filling the lane");
            }
            let blocker = loom::thread::spawn(move || {
                // The lane is full: this parks until a pop or the close.
                tx.send_blocking(AppEvent::Paste("xy".into()))
            });
            drop(rx);
            let result = blocker.join().unwrap();
            assert!(result.is_err(), "the close hands the event back");
            assert!(matches!(result.err().unwrap().0, AppEvent::Paste(_)));
        });
    }

    #[test]
    fn loom_pop_wakes_blocking_sender() {
        loom::model::Builder::new().preemption_bound(3).check(|| {
            let (tx, rx) = channel();
            for _ in 0..MAX_SEMANTIC_EVENTS {
                tx.send(AppEvent::QuitIntent).expect("prefill");
            }
            let blocker = loom::thread::spawn(move || {
                tx.send_blocking(AppEvent::Paste("xy".into()))
                    .expect("a pop frees room");
                // The sender drops HERE, inside its loom thread — loom
                // objects must not outlive their execution context.
            });
            let mut seen = 0usize;
            let mut paste_seen = false;
            loop {
                match rx.try_recv() {
                    Ok(AppEvent::Paste(_)) => {
                        paste_seen = true;
                        seen += 1;
                    }
                    Ok(_) => seen += 1,
                    Err(TryRecvError::Empty) => loom::thread::yield_now(),
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            blocker.join().unwrap();
            assert!(paste_seen, "the blocked admission landed after a pop");
            assert_eq!(seen, MAX_SEMANTIC_EVENTS + 1, "nothing lost");
        });
    }
}
