//! The ordered wire: one FIFO worker per connection serializes owned
//! rope snapshots and frames them in submission order (R6). Callers
//! cheaply clone a rope snapshot and enqueue; the String
//! materialization LSP full-text sync needs happens here, never on the
//! editor's input thread. `didOpen`/`didChange`/requests/`didClose`
//! therefore reach the server in the order the sync lock admitted them.
//!
//! Retention is bounded (0056 AR06): unsent jobs are bounded by count,
//! unsent document snapshots by bytes. A superseded unsent `didChange`
//! coalesces into the newest snapshot ONLY when no admitted barrier —
//! a request/workspace query, or an open/close for the same document —
//! sits between them: a barrier's server-side meaning depends on the
//! intermediate version. Refusal is visible to the caller (typed
//! `Admission`), never a silent drop; accepted jobs always reach the
//! wire or settle through the connection's terminal failure event.
//! Shutdown quiesces: a closed mainloop stops framing, the worker drops
//! retained snapshots off the input thread, and the queue refuses new
//! work instead of growing without a reader.
use parking_lot::Mutex;
#[cfg(not(strop_loom))]
use parking_lot::{Condvar, MutexGuard};
use std::collections::VecDeque;
use std::sync::Arc;

/// The instrumented-sync seam (0057 VF11): under `--cfg strop_loom` the
/// queue's OWN coordination types (the Shared mutex/condvar and the Arc
/// around it) compile to loom's instrumented types, so exhaustive
/// interleaving campaigns run over the real admission/drain control
/// flow. Types crossing module boundaries (WireEnv) keep the production
/// types; without the cfg the identical source uses parking_lot/std and
/// loom never ships.
#[cfg(not(strop_loom))]
type QueueLock = Mutex<QueueState>;
#[cfg(strop_loom)]
type QueueLock = loom::sync::Mutex<QueueState>;
#[cfg(not(strop_loom))]
type QueueCondvar = Condvar;
#[cfg(strop_loom)]
type QueueCondvar = loom::sync::Condvar;
#[cfg(not(strop_loom))]
type QueueGuard<'a> = MutexGuard<'a, QueueState>;
#[cfg(strop_loom)]
type QueueGuard<'a> = loom::sync::MutexGuard<'a, QueueState>;
#[cfg(not(strop_loom))]
type SharedArc = Arc<Shared>;
#[cfg(strop_loom)]
type SharedArc = loom::sync::Arc<Shared>;

use async_lsp::lsp_types as lt;
use async_lsp::lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument,
};
use async_lsp::ServerSocket;
use ropey::Rope;

use crate::caps::ServerCaps;
use crate::protocol::{LspEvent, PendingRequest, ServerId, WireVersion};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Bound on admitted-but-unsent wire jobs (AR06). One job per admitted
/// lifecycle step; coalescing keeps ordinary typing far below this.
/// The value is a calibration parameter: under `--cfg strop_loom` it shrinks
/// so exhaustive campaigns terminate; the admission logic is identical.
#[cfg(not(strop_loom))]
pub(crate) const MAX_QUEUED_JOBS: usize = 256;
#[cfg(strop_loom)]
pub(crate) const MAX_QUEUED_JOBS: usize = 3;
/// Bound on retained unsent document snapshots. Full-text sync retains
/// one rope clone per unsent open/change; this caps their total bytes.
#[cfg(not(strop_loom))]
pub(crate) const MAX_QUEUED_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
#[cfg(strop_loom)]
pub(crate) const MAX_QUEUED_SNAPSHOT_BYTES: usize = 64;
#[cfg(not(strop_loom))]
const MAX_RETIRED_SNAPSHOTS: usize = 8;
#[cfg(strop_loom)]
const MAX_RETIRED_SNAPSHOTS: usize = 3;

/// One admitted lifecycle step, carrying everything the wire needs.
pub(crate) enum WireJob {
    Open {
        uri: lt::Url,
        language_id: String,
        version: WireVersion,
        text: Rope,
    },
    Change {
        uri: lt::Url,
        version: WireVersion,
        text: Rope,
    },
    Close {
        uri: lt::Url,
    },
    Request(PendingRequest),
    /// A small barrier only; bounded completion tasks own their captured input.
    Completion(crate::RequestStamp),
    /// Workspace-wide symbol query (0063 §2): document-free, so the
    /// reply correlates on the caller's generation.
    WorkspaceSymbols {
        generation: u64,
        query: String,
    },
}

/// The outcome of offering one job to the wire (AR06: visible refusal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Queued behind everything admitted earlier.
    Admitted,
    /// Replaced a superseded unsent snapshot for the same document in
    /// place; no admitted barrier needed the intermediate version.
    Coalesced,
    /// The queue is full — the connection is not draining. The caller
    /// makes the refusal visible; the job was never admitted.
    Refused,
    /// The wire worker is gone with the connection; admitted-looking
    /// state settles through the connection's terminal failure event.
    Closed,
}

/// Bounded shared queue state. All mutation happens under the mutex;
/// the worker sleeps on `available` instead of polling.
#[derive(Default)]
struct QueueState {
    jobs: VecDeque<WireJob>,
    /// Retained bytes of unsent Open/Change snapshots.
    snapshot_bytes: usize,
    /// Superseded snapshots await destruction on the existing wire worker.
    /// Count/bytes are independent of the live queue, with the same lone-large
    /// snapshot allowance as initial document synchronization.
    retired: Vec<Rope>,
    retired_bytes: usize,
    /// Live WireTx clones; the worker exits once drained with none left.
    senders: usize,
    /// False once the worker has ended: no reader, no further admission.
    accepting: bool,
}

struct Shared {
    state: QueueLock,
    available: QueueCondvar,
}

/// The instrumented-sync seam (0057 VF11): parking_lot's lock returns
/// the guard directly, loom's a LockResult — both produce it here.
#[cfg(not(strop_loom))]
fn lock_state(shared: &Shared) -> QueueGuard<'_> {
    shared.state.lock()
}
#[cfg(strop_loom)]
fn lock_state(shared: &Shared) -> QueueGuard<'_> {
    shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// parking_lot waits in place on a &mut guard; loom consumes and
/// returns it. Both produce the post-wait guard here.
#[cfg(not(strop_loom))]
fn wait_available<'a>(shared: &Shared, mut state: QueueGuard<'a>) -> QueueGuard<'a> {
    shared.available.wait(&mut state);
    state
}
#[cfg(strop_loom)]
fn wait_available<'a>(shared: &Shared, state: QueueGuard<'a>) -> QueueGuard<'a> {
    shared
        .available
        .wait(state)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Open/change snapshots obey the count/byte bound. A lone oversized snapshot
/// still admits; lifecycle/control messages with no snapshot keep flowing.
fn admit_append(state: &mut QueueState, job: WireJob) -> Admission {
    if state.jobs.len() >= MAX_QUEUED_JOBS {
        return Admission::Refused;
    }
    let bytes = match &job {
        WireJob::Open { text, .. } | WireJob::Change { text, .. } => text.len_bytes(),
        _ => 0,
    };
    if bytes > 0
        && state.snapshot_bytes > 0
        && state.snapshot_bytes.saturating_add(bytes) > MAX_QUEUED_SNAPSHOT_BYTES
    {
        return Admission::Refused;
    }
    state.snapshot_bytes = state.snapshot_bytes.saturating_add(bytes);
    state.jobs.push_back(job);
    Admission::Admitted
}

/// Admit a `Change`, coalescing a superseded unsent change for the same
/// document when legal: scanning newest-first, the nearest job for this
/// URI either is an unsent `Change` (merge target) or a lifecycle
/// barrier (`Open`/`Close` for this URI — stop). ANY queued request or
/// workspace query is a barrier too: its server-side answer is computed
/// against the intermediate version, so that version must reach the
/// wire. Coalescing retains the superseded rope for bounded worker retirement;
/// it cannot hide growth in either the live snapshot or retirement budget.
fn admit_change(
    state: &mut QueueState,
    uri: lt::Url,
    version: WireVersion,
    text: Rope,
) -> Admission {
    let mut merge_at = None;
    for (index, job) in state.jobs.iter().enumerate().rev() {
        match job {
            WireJob::Change { uri: queued, .. } if *queued == uri => {
                merge_at = Some(index);
                break;
            }
            WireJob::Request(_) | WireJob::Completion(_) | WireJob::WorkspaceSymbols { .. } => {
                break
            }
            WireJob::Open { uri: queued, .. } | WireJob::Close { uri: queued }
                if *queued == uri =>
            {
                break;
            }
            _ => {}
        }
    }
    let bytes = text.len_bytes();
    match merge_at {
        Some(index) => {
            let Some(WireJob::Change {
                version: queued_version,
                text: queued_text,
                ..
            }) = state.jobs.get_mut(index)
            else {
                debug_assert!(false, "merge target changed class under the lock");
                return Admission::Refused;
            };
            let previous_bytes = queued_text.len_bytes();
            let other_bytes = state.snapshot_bytes.saturating_sub(previous_bytes);
            if (other_bytes > 0 && other_bytes.saturating_add(bytes) > MAX_QUEUED_SNAPSHOT_BYTES)
                || state.retired.len() >= MAX_RETIRED_SNAPSHOTS
                || (!state.retired.is_empty()
                    && state.retired_bytes.saturating_add(previous_bytes)
                        > MAX_QUEUED_SNAPSHOT_BYTES)
            {
                return Admission::Refused;
            }
            state.snapshot_bytes = other_bytes.saturating_add(bytes);
            state.retired_bytes = state.retired_bytes.saturating_add(previous_bytes);
            *queued_version = version;
            state.retired.push(std::mem::replace(queued_text, text));
            Admission::Coalesced
        }
        None => admit_append(state, WireJob::Change { uri, version, text }),
    }
}

fn admit(shared: &Shared, job: WireJob) -> Admission {
    let mut state = lock_state(shared);
    if !state.accepting {
        return Admission::Closed;
    }
    let admission = match job {
        WireJob::Change { uri, version, text } => admit_change(&mut state, uri, version, text),
        job => admit_append(&mut state, job),
    };
    if matches!(admission, Admission::Admitted | Admission::Coalesced) {
        drop(state);
        shared.available.notify_one();
    }
    admission
}

/// Pop the next job, sleeping until one arrives or every sender is
/// gone. Drains admitted work before observing disconnect, exactly like
/// the mpsc predecessor.
fn next_job(shared: &Shared, retired: &mut Vec<Rope>) -> Option<WireJob> {
    let mut state = lock_state(shared);
    loop {
        if !state.retired.is_empty() {
            std::mem::swap(&mut state.retired, retired);
            state.retired_bytes = 0;
            drop(state);
            retired.clear();
            state = lock_state(shared);
        }
        if !state.accepting {
            return None;
        }
        if let Some(job) = state.jobs.pop_front() {
            if let WireJob::Open { text, .. } | WireJob::Change { text, .. } = &job {
                state.snapshot_bytes = state.snapshot_bytes.saturating_sub(text.len_bytes());
            }
            return Some(job);
        }
        if state.senders == 0 {
            return None;
        }
        state = wait_available(shared, state);
    }
}

fn retire_snapshots(shared: &Shared, retired: &mut Vec<Rope>) {
    debug_assert!(
        retired.is_empty(),
        "worker retirement scratch is empty between drains"
    );
    let mut state = lock_state(shared);
    std::mem::swap(&mut state.retired, retired);
    state.retired_bytes = 0;
    drop(state);
    retired.clear();
}

/// Everything the worker and the request launcher need — deliberately
/// NOT a `Client` clone, so the worker never keeps its own queue sender
/// alive and shutdown-by-drop still terminates it.
#[derive(Clone)]
pub(crate) struct WireEnv {
    pub(crate) id: ServerId,
    pub(crate) name: String,
    pub(crate) hint: String,
    pub(crate) socket: ServerSocket,
    pub(crate) handle: tokio::runtime::Handle,
    pub(crate) tx: std::sync::mpsc::Sender<LspEvent>,
    pub(crate) caps: ServerCaps,
    /// Where the server runs: URI mapping and failure labels are
    /// target-aware (local root or remote endpoint+root).
    pub(crate) workspace: crate::target::Workspace,
    pub(crate) sync: Arc<Mutex<super::sync::SyncState>>,
    /// Set by `Client::shutdown`: an exit after this is not a crash.
    pub(crate) quitting: Arc<AtomicBool>,
    /// Set once the server mainloop has ended: stop framing new jobs.
    pub(crate) closed: Arc<AtomicBool>,
    pub(crate) outbound: Arc<super::outbound::Outbound>,
    pub(crate) completion: Arc<super::completion::Registry>,
}

impl WireEnv {
    /// A failed frame write means the connection is gone; surface it as
    /// a terminal failure unless we asked to quit.
    fn report_dead(&self, detail: impl std::fmt::Display) {
        if !self.quitting.load(Ordering::Relaxed) {
            let _ = self.tx.send(LspEvent::Failed {
                server: self.id,
                name: self.name.clone(),
                hint: format!("language-service write failed: {detail} — {}", self.hint),
            });
        }
    }

    fn notify<N: lt::notification::Notification>(
        &self,
        shared: &Shared,
        retired: &mut Vec<Rope>,
        method: super::outbound::Method,
        params: N::Params,
    ) {
        let ticket = match self.outbound.begin_notification(method) {
            Ok(ticket) => ticket,
            Err(error) => {
                if method != super::outbound::Method::Close {
                    self.report_dead(error);
                }
                return;
            }
        };
        if let Err(error) = self.socket.notify::<N>(params) {
            self.outbound.abort(ticket);
            if method != super::outbound::Method::Close {
                self.report_dead(error);
            }
        } else if !self.outbound.wait(ticket, || {
            retire_snapshots(shared, retired);
            self.completion.wake_available();
        }) && method != super::outbound::Method::Close
        {
            self.report_dead("connection closed before the notification flushed");
        }
        self.completion.wake_available();
    }
}

pub(crate) struct WireTx {
    shared: SharedArc,
    outbound: Arc<super::outbound::Outbound>,
}

/// Cancellation capability without a sender lease: request tasks cannot keep
/// their own wire worker alive after the owning clients have gone away.
#[derive(Clone)]
pub(super) struct WireControl {
    shared: SharedArc,
}
impl WireControl {
    pub fn cancel_completion(&self, stamp: crate::RequestStamp) {
        let mut state = lock_state(&self.shared);
        if let Some(index) = state
            .jobs
            .iter()
            .position(|job| matches!(job, WireJob::Completion(owner) if *owner == stamp))
        {
            // This variant holds no rope/candidate payload, so removing an
            // obsolete barrier cannot run a large destructor on input.
            state.jobs.remove(index);
        }
        drop(state);
        self.shared.available.notify_one();
    }
}

impl WireTx {
    pub(super) fn control(&self) -> WireControl {
        WireControl {
            shared: self.shared.clone(),
        }
    }

    /// Register physical ownership and the ordered barrier atomically. No task
    /// or retained request is created when this admission is refused.
    pub(super) fn admit_completion<T>(
        &self,
        stamp: crate::RequestStamp,
        register: impl FnOnce() -> Result<T, crate::RequestRefusal>,
    ) -> Result<T, crate::RequestRefusal> {
        let mut state = lock_state(&self.shared);
        if !state.accepting {
            return Err(crate::RequestRefusal::NotReady);
        }
        if state.jobs.len() >= MAX_QUEUED_JOBS {
            return Err(crate::RequestRefusal::Overloaded);
        }
        let registered = register()?;
        state.jobs.push_back(WireJob::Completion(stamp));
        drop(state);
        self.shared.available.notify_one();
        Ok(registered)
    }

    /// Enqueue in admission order. The admission outcome is the caller's
    /// to act on: refused work was never admitted and must be made
    /// visible; `Closed` work settles through the connection's terminal
    /// failure event, as before.
    pub(crate) fn send(&self, job: WireJob) -> Admission {
        let admission = admit(&self.shared, job);
        if admission == Admission::Coalesced {
            self.outbound.wake_retirement();
        }
        admission
    }
}

impl Clone for WireTx {
    fn clone(&self) -> Self {
        lock_state(&self.shared).senders += 1;
        Self {
            shared: self.shared.clone(),
            outbound: self.outbound.clone(),
        }
    }
}

impl Drop for WireTx {
    fn drop(&mut self) {
        let mut state = lock_state(&self.shared);
        state.senders = state.senders.saturating_sub(1);
        if state.senders == 0 {
            // Wake the worker so it drains and exits.
            drop(state);
            self.shared.available.notify_all();
        }
    }
}

/// Start the per-connection wire worker. `None` when the thread cannot
/// be created — callers treat that as a spawn failure.
pub(crate) fn start(env: WireEnv) -> Option<WireTx> {
    let shared = SharedArc::new(Shared {
        state: QueueLock::new(QueueState {
            senders: 1,
            accepting: true,
            retired: Vec::with_capacity(MAX_RETIRED_SNAPSHOTS),
            ..QueueState::default()
        }),
        available: QueueCondvar::new(),
    });
    let closing = shared.clone();
    let completion = env.completion.clone();
    env.outbound
        .set_close_wake(move || {
            let mut state = lock_state(&closing);
            state.accepting = false;
            drop(state);
            closing.available.notify_all();
            completion.close();
        })
        .ok()?;
    let worker_shared = shared.clone();
    let outbound = env.outbound.clone();
    let spawned = std::thread::Builder::new()
        .name("strop-lsp-wire".into())
        .spawn(move || worker(env, worker_shared));
    spawned.ok().map(|_| WireTx { shared, outbound })
}

fn worker(env: WireEnv, shared: SharedArc) {
    let mut retired = Vec::with_capacity(MAX_RETIRED_SNAPSHOTS);
    while let Some(job) = next_job(&shared, &mut retired) {
        // The mainloop is gone: anything still queued can never be
        // framed, and its requests settle through the failure event.
        if env.closed.load(Ordering::Relaxed) {
            break;
        }
        env.completion.wake_available();
        frame(&env, &shared, &mut retired, job);
    }
    // Quiesce (AR06): stop accepting, drop retained snapshots here on
    // the worker thread — never as a blocking Drop on an input caller.
    let mut state = lock_state(&shared);
    let abandoned = std::mem::take(&mut state.jobs);
    let abandoned_retirement = std::mem::take(&mut state.retired);
    state.retired_bytes = 0;
    state.snapshot_bytes = 0;
    state.accepting = false;
    drop(state);
    drop(abandoned);
    drop(abandoned_retirement);
    shared.available.notify_all();
}

/// Frame one admitted job. A frame write is synchronous: an emitting
/// frame is never cancelled halfway (AR06).
fn frame(env: &WireEnv, shared: &Shared, retired: &mut Vec<Rope>, job: WireJob) {
    match job {
        WireJob::Open {
            uri,
            language_id,
            version,
            text,
        } => {
            env.notify::<DidOpenTextDocument>(
                shared,
                retired,
                super::outbound::Method::Open,
                lt::DidOpenTextDocumentParams {
                    text_document: lt::TextDocumentItem {
                        uri,
                        language_id,
                        version: version.get(),
                        text: text.to_string(),
                    },
                },
            );
        }
        WireJob::Change { uri, version, text } => {
            env.notify::<DidChangeTextDocument>(
                shared,
                retired,
                super::outbound::Method::Change,
                lt::DidChangeTextDocumentParams {
                    text_document: lt::VersionedTextDocumentIdentifier {
                        uri,
                        version: version.get(),
                    },
                    content_changes: vec![lt::TextDocumentContentChangeEvent {
                        range: None,
                        range_length: None,
                        text: text.to_string(),
                    }],
                },
            );
        }
        WireJob::Close { uri } => {
            env.notify::<DidCloseTextDocument>(
                shared,
                retired,
                super::outbound::Method::Close,
                lt::DidCloseTextDocumentParams {
                    text_document: lt::TextDocumentIdentifier { uri },
                },
            );
        }
        WireJob::Completion(stamp) => env.completion.start(stamp),
        WireJob::WorkspaceSymbols { generation, query } => {
            // Same ordered-lane rule as requests: earlier frames
            // are on the wire before the query leaves.
            let launch = std::panic::AssertUnwindSafe(|| {
                env.handle.spawn(super::api::workspace_symbols(
                    env.clone(),
                    generation,
                    query,
                ));
            });
            let _ = std::panic::catch_unwind(launch);
        }
        WireJob::Request(request) => {
            // The job order already put every earlier frame on the
            // wire; the request task itself rides the client runtime.
            // Racing runtime teardown cannot abort the process.
            let launch = std::panic::AssertUnwindSafe(|| super::api::launch(env, request));
            let _ = std::panic::catch_unwind(launch);
        }
    }
}

/// How long the content-modified retry policy waits before re-sending.
pub(crate) const RETRY_DELAY: Duration = Duration::from_millis(800);

#[cfg(test)]
mod tests;

/// 0057 VF11: a Loom campaign over the REAL admission/drain control
/// flow — the same QueueState under loom's instrumented Mutex/Condvar,
/// at the shrunken cfg(strop_loom) bounds. Properties under racing producers:
/// admitted jobs all drain (drain-before-disconnect), drained versions
/// for a document strictly increase, a request barrier is never
/// overtaken by a later change for the same document, and the worker
/// side of `next_job` parks/wakes without loss.
#[cfg(all(test, strop_loom))]
mod loom_tests;
