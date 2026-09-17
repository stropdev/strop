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
use parking_lot::{Condvar, Mutex, MutexGuard};
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

/// Append with the count/byte bounds. `Change` checks both bounds;
/// lifecycle frames check the count bound only (an open's snapshot must
/// be admissible regardless of size, or the document can never sync).
/// A lone oversize job into an EMPTY queue still admits — the bound
/// limits retention, not single-message size.
fn admit_append(state: &mut QueueState, job: WireJob) -> Admission {
    if state.jobs.len() >= MAX_QUEUED_JOBS {
        return Admission::Refused;
    }
    let bytes = match &job {
        WireJob::Open { text, .. } | WireJob::Change { text, .. } => text.len_bytes(),
        _ => 0,
    };
    if matches!(job, WireJob::Change { .. })
        && !state.jobs.is_empty()
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
/// wire. Replacement keeps one snapshot per document per queue, so the
/// byte bound does not gate it.
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
            WireJob::Request(_) | WireJob::WorkspaceSymbols { .. } => break,
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
            state.snapshot_bytes = state.snapshot_bytes.saturating_sub(queued_text.len_bytes());
            state.snapshot_bytes = state.snapshot_bytes.saturating_add(bytes);
            *queued_version = version;
            *queued_text = text;
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
fn next_job(shared: &Shared) -> Option<WireJob> {
    let mut state = lock_state(shared);
    loop {
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
}

impl WireEnv {
    /// A failed frame write means the connection is gone; surface it as
    /// a terminal failure unless we asked to quit.
    fn report_dead(&self) {
        if !self.quitting.load(Ordering::Relaxed) {
            let _ = self.tx.send(LspEvent::Failed {
                server: self.id,
                name: self.name.clone(),
                hint: self.hint.clone(),
            });
        }
    }
}

pub(crate) struct WireTx {
    shared: SharedArc,
}

impl WireTx {
    /// Enqueue in admission order. The admission outcome is the caller's
    /// to act on: refused work was never admitted and must be made
    /// visible; `Closed` work settles through the connection's terminal
    /// failure event, as before.
    pub(crate) fn send(&self, job: WireJob) -> Admission {
        admit(&self.shared, job)
    }
}

impl Clone for WireTx {
    fn clone(&self) -> Self {
        lock_state(&self.shared).senders += 1;
        Self {
            shared: self.shared.clone(),
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
            ..QueueState::default()
        }),
        available: QueueCondvar::new(),
    });
    let worker_shared = shared.clone();
    let spawned = std::thread::Builder::new()
        .name("strop-lsp-wire".into())
        .spawn(move || worker(env, worker_shared));
    spawned.ok().map(|_| WireTx { shared })
}

fn worker(env: WireEnv, shared: SharedArc) {
    while let Some(job) = next_job(&shared) {
        // The mainloop is gone: anything still queued can never be
        // framed, and its requests settle through the failure event.
        if env.closed.load(Ordering::Relaxed) {
            break;
        }
        frame(&env, job);
    }
    // Quiesce (AR06): stop accepting, drop retained snapshots here on
    // the worker thread — never as a blocking Drop on an input caller.
    let mut state = lock_state(&shared);
    state.jobs.clear();
    state.snapshot_bytes = 0;
    state.accepting = false;
    drop(state);
    shared.available.notify_all();
}

/// Frame one admitted job. A frame write is synchronous: an emitting
/// frame is never cancelled halfway (AR06).
fn frame(env: &WireEnv, job: WireJob) {
    match job {
        WireJob::Open {
            uri,
            language_id,
            version,
            text,
        } => {
            let notified =
                env.socket
                    .notify::<DidOpenTextDocument>(lt::DidOpenTextDocumentParams {
                        text_document: lt::TextDocumentItem {
                            uri,
                            language_id,
                            version: version.get(),
                            text: text.to_string(),
                        },
                    });
            if notified.is_err() {
                env.report_dead();
            }
        }
        WireJob::Change { uri, version, text } => {
            let notified =
                env.socket
                    .notify::<DidChangeTextDocument>(lt::DidChangeTextDocumentParams {
                        text_document: lt::VersionedTextDocumentIdentifier {
                            uri,
                            version: version.get(),
                        },
                        content_changes: vec![lt::TextDocumentContentChangeEvent {
                            range: None,
                            range_length: None,
                            text: text.to_string(),
                        }],
                    });
            if notified.is_err() {
                env.report_dead();
            }
        }
        WireJob::Close { uri } => {
            // A close for a dying connection needs no failure event.
            let _ = env
                .socket
                .notify::<DidCloseTextDocument>(lt::DidCloseTextDocumentParams {
                    text_document: lt::TextDocumentIdentifier { uri },
                });
        }
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
mod tests {
    //! Admission-policy unit tests: coalescing legality, barrier
    //! preservation and the count/byte bounds. These exercise the queue
    //! state directly; the worker is a plain FIFO drain over it.
    use super::*;

    fn uri(name: &str) -> lt::Url {
        lt::Url::parse(&format!("file:///workspace/{name}")).unwrap()
    }

    fn change(name: &str, version: i32, text: &str) -> WireJob {
        WireJob::Change {
            uri: uri(name),
            version: WireVersion::new(version),
            text: Rope::from_str(text),
        }
    }

    fn document_id() -> strop_core::id::DocumentId {
        let mut arena: strop_core::id::Arena<strop_core::id::DocumentKind, ()> =
            strop_core::id::Arena::default();
        arena.try_insert(()).unwrap()
    }

    fn request() -> WireJob {
        WireJob::Request(PendingRequest {
            stamp: crate::protocol::RequestStamp {
                request: crate::protocol::RequestId::new(0),
                server: ServerId::new(1),
                document: document_id(),
                revision: strop_core::id::BufferRevision::new(0),
            },
            input: crate::protocol::RequestInput {
                document: document_id(),
                revision: strop_core::id::BufferRevision::new(0),
                path: std::path::PathBuf::from("/workspace/a.rs"),
                line: strop_core::id::LineIndex::new(0),
                byte_col: strop_core::id::ByteColumn::new(0),
                line_text: crate::FrozenLine::from(""),
                kind: crate::protocol::RequestKind::Hover,
                rename_to: None,
            },
            tab_width: None,
        })
    }

    fn queue() -> Shared {
        Shared {
            state: QueueLock::new(QueueState {
                senders: 1,
                accepting: true,
                ..QueueState::default()
            }),
            available: QueueCondvar::new(),
        }
    }

    fn queued_versions(shared: &Shared, name: &str) -> Vec<i32> {
        lock_state(shared)
            .jobs
            .iter()
            .filter_map(|job| match job {
                WireJob::Change {
                    uri: u, version, ..
                } if *u == uri(name) => Some(version.get()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn superseded_unsent_changes_coalesce_without_a_barrier() {
        let shared = queue();
        assert_eq!(
            admit(&shared, change("a.rs", 1, "one")),
            Admission::Admitted
        );
        assert_eq!(
            admit(&shared, change("b.rs", 1, "other")),
            Admission::Admitted
        );
        // No barrier between: the newest snapshot replaces the oldest
        // unsent one for the same document, in place.
        assert_eq!(
            admit(&shared, change("a.rs", 2, "two")),
            Admission::Coalesced
        );
        assert_eq!(queued_versions(&shared, "a.rs"), vec![2]);
        assert_eq!(queued_versions(&shared, "b.rs"), vec![1]);
        let state = lock_state(&shared);
        assert_eq!(state.jobs.len(), 2);
        assert_eq!(
            state.snapshot_bytes,
            Rope::from_str("two").len_bytes() + Rope::from_str("other").len_bytes()
        );
    }

    #[test]
    fn an_admitted_request_bars_coalescing_the_version_it_observes() {
        let shared = queue();
        admit(&shared, change("a.rs", 1, "one"));
        admit(&shared, request());
        // The request is answered against version 1: version 1 must
        // reach the wire, so version 2 queues behind it.
        assert_eq!(
            admit(&shared, change("a.rs", 2, "two")),
            Admission::Admitted
        );
        assert_eq!(queued_versions(&shared, "a.rs"), vec![1, 2]);
        // Versions 1 and 2 are BOTH pre-request... no: only version 2
        // trails the request. A third change may coalesce into 2.
        assert_eq!(
            admit(&shared, change("a.rs", 3, "three")),
            Admission::Coalesced
        );
        assert_eq!(queued_versions(&shared, "a.rs"), vec![1, 3]);
    }

    #[test]
    fn a_close_for_the_document_bars_coalescing_across_it() {
        let shared = queue();
        admit(&shared, change("a.rs", 1, "one"));
        admit(&shared, WireJob::Close { uri: uri("a.rs") });
        // A change after the close (reopen churn) must not merge past
        // the lifecycle barrier.
        assert_eq!(
            admit(&shared, change("a.rs", 2, "two")),
            Admission::Admitted
        );
        assert_eq!(queued_versions(&shared, "a.rs"), vec![1, 2]);
        assert_eq!(lock_state(&shared).jobs.len(), 3);
    }

    #[test]
    fn the_job_count_bound_refuses_visibly() {
        let shared = queue();
        for i in 0..MAX_QUEUED_JOBS {
            let admitted = admit(
                &shared,
                WireJob::Close {
                    uri: uri(&format!("{i}.rs")),
                },
            );
            assert_eq!(admitted, Admission::Admitted);
        }
        assert_eq!(
            admit(
                &shared,
                WireJob::Close {
                    uri: uri("overflow.rs"),
                },
            ),
            Admission::Refused
        );
    }

    #[test]
    fn the_snapshot_byte_bound_refuses_changes_but_not_lifecycle() {
        let shared = queue();
        // A lone oversize snapshot into an empty queue admits (the bound
        // limits retention, not single-message size).
        let big = "x".repeat(MAX_QUEUED_SNAPSHOT_BYTES + 1);
        assert_eq!(
            admit(&shared, change("big.rs", 1, &big)),
            Admission::Admitted
        );
        // With a snapshot retained, any further change exceeds the byte
        // bound and is refused...
        assert_eq!(admit(&shared, change("b.rs", 1, "b")), Admission::Refused);
        // ...but coalescing the retained snapshot itself is never
        // byte-gated: replacement cannot grow retention past one
        // snapshot per document.
        assert_eq!(
            admit(&shared, change("big.rs", 2, "small")),
            Admission::Coalesced
        );
        // Lifecycle frames and requests still flow with the lane at
        // the byte bound (they carry no snapshots).
        assert_eq!(
            admit(&shared, WireJob::Close { uri: uri("big.rs") },),
            Admission::Admitted
        );
        assert_eq!(admit(&shared, request()), Admission::Admitted);
        // The close is a barrier for its document: a later change must
        // not merge past it — both versions stay queued in order.
        assert_eq!(
            admit(&shared, change("big.rs", 3, "later")),
            Admission::Admitted
        );
        assert_eq!(queued_versions(&shared, "big.rs"), vec![2, 3]);
    }

    #[test]
    fn coalescing_tracks_bytes_so_the_bound_reflects_what_is_retained() {
        let shared = queue();
        let first = "a".repeat(MAX_QUEUED_SNAPSHOT_BYTES - 4);
        admit(&shared, change("a.rs", 1, &first));
        // Replace the big snapshot with a tiny one: the freed bytes are
        // available to the next document.
        assert_eq!(
            admit(&shared, change("a.rs", 2, "tiny")),
            Admission::Coalesced
        );
        assert_eq!(
            admit(&shared, change("b.rs", 1, "fits now")),
            Admission::Admitted
        );
    }

    #[test]
    fn a_stopped_queue_reports_closed_and_drops_retained_snapshots() {
        let shared = queue();
        admit(&shared, change("a.rs", 1, "retained"));
        {
            let mut state = lock_state(&shared);
            state.jobs.clear();
            state.snapshot_bytes = 0;
            state.accepting = false;
        }
        assert_eq!(admit(&shared, change("a.rs", 2, "late")), Admission::Closed);
        assert_eq!(lock_state(&shared).snapshot_bytes, 0);
    }
}

/// 0057 VF11: a Loom campaign over the REAL admission/drain control
/// flow — the same QueueState under loom's instrumented Mutex/Condvar,
/// at the shrunken cfg(strop_loom) bounds. Properties under racing producers:
/// admitted jobs all drain (drain-before-disconnect), drained versions
/// for a document strictly increase, a request barrier is never
/// overtaken by a later change for the same document, and the worker
/// side of `next_job` parks/wakes without loss.
#[cfg(all(test, strop_loom))]
mod loom_tests {
    use super::*;

    fn uri(name: &str) -> lt::Url {
        lt::Url::parse(&format!("file:///workspace/{name}")).unwrap()
    }

    fn open(name: &str, version: i32, text: &str) -> WireJob {
        WireJob::Open {
            uri: uri(name),
            language_id: "rust".to_owned(),
            version: WireVersion::new(version),
            text: Rope::from_str(text),
        }
    }

    fn change(name: &str, version: i32, text: &str) -> WireJob {
        WireJob::Change {
            uri: uri(name),
            version: WireVersion::new(version),
            text: Rope::from_str(text),
        }
    }

    fn request() -> WireJob {
        let mut arena: strop_core::id::Arena<strop_core::id::DocumentKind, ()> =
            strop_core::id::Arena::default();
        let document = arena.try_insert(()).unwrap();
        WireJob::Request(PendingRequest {
            stamp: crate::protocol::RequestStamp {
                request: crate::protocol::RequestId::new(0),
                server: ServerId::new(1),
                document,
                revision: strop_core::id::BufferRevision::new(0),
            },
            input: crate::protocol::RequestInput {
                document,
                revision: strop_core::id::BufferRevision::new(0),
                path: std::path::PathBuf::from("/workspace/a.rs"),
                line: strop_core::id::LineIndex::new(0),
                byte_col: strop_core::id::ByteColumn::new(0),
                line_text: crate::FrozenLine::from(""),
                kind: crate::protocol::RequestKind::Hover,
                rename_to: None,
            },
            tab_width: None,
        })
    }

    /// One loom thread's share of the sender count: drop discipline is
    /// what WireTx::drop does.
    fn release(shared: &Shared) {
        let mut state = lock_state(shared);
        state.senders = state.senders.saturating_sub(1);
        if state.senders == 0 {
            drop(state);
            shared.available.notify_all();
        }
    }

    /// Versions allocate in admission order (production: next_version
    /// under the sync lock that also admits), so the campaign pulls the
    /// version and admits under one instrumented lock.
    fn admit_versioned(
        alloc: &loom::sync::Mutex<i32>,
        shared: &Shared,
        job: impl FnOnce(i32) -> WireJob,
    ) -> (Admission, i32) {
        let mut next = alloc.lock().unwrap_or_else(|p| p.into_inner());
        *next += 1;
        let version = *next;
        let outcome = admit(shared, job(version));
        drop(next);
        (outcome, version)
    }

    #[test]
    fn loom_fifo_barrier_drain_disconnect() {
        loom::model(|| {
            let shared = SharedArc::new(Shared {
                state: QueueLock::new(QueueState {
                    senders: 2,
                    accepting: true,
                    ..QueueState::default()
                }),
                available: QueueCondvar::new(),
            });
            let alloc = loom::sync::Arc::new(loom::sync::Mutex::new(0));
            let producer_shared = shared.clone();
            let producer_alloc = alloc.clone();
            let producer = loom::thread::spawn(move || {
                let mut admissions = Vec::new();
                for text in ["one", "two", "three"] {
                    admissions.push(admit_versioned(&producer_alloc, &producer_shared, |v| {
                        change("a.rs", v, text)
                    }));
                }
                release(&producer_shared);
                admissions
            });
            let request_admission = admit(&shared, request());
            let (change_admission, after_request) =
                admit_versioned(&alloc, &shared, |v| change("a.rs", v, "four"));
            release(&shared);
            let mut drained = Vec::new();
            while let Some(job) = next_job(&shared) {
                drained.push(job);
            }
            let theirs = producer.join().unwrap();
            let admissions: Vec<Admission> = theirs
                .iter()
                .map(|(a, _)| *a)
                .chain([request_admission, change_admission])
                .collect();
            assert!(
                admissions
                    .iter()
                    .all(|a| *a == Admission::Admitted || *a == Admission::Coalesced),
                "nothing refuses within the shrunken bound: {admissions:?}"
            );
            let admitted = admissions
                .iter()
                .filter(|a| **a == Admission::Admitted)
                .count();
            assert_eq!(
                drained.len(),
                admitted,
                "every admitted job drained before disconnect; \
                 coalesced ones replaced their slot"
            );
            let mut last_version = 0;
            let mut request_seen = false;
            for job in &drained {
                match job {
                    WireJob::Open { version, .. } | WireJob::Change { version, .. } => {
                        let v = version.get();
                        assert!(v > last_version, "versions strictly increase");
                        if request_seen {
                            assert!(
                                v >= after_request,
                                "a pre-request version never drains after \
                                 the barrier"
                            );
                        } else {
                            assert!(
                                v < after_request,
                                "a post-request version never drains before \
                                 the barrier"
                            );
                        }
                        last_version = v;
                    }
                    WireJob::Request(_) => request_seen = true,
                    WireJob::Close { .. } | WireJob::WorkspaceSymbols { .. } => {}
                }
            }
            assert!(request_seen, "the request drained");
            assert_eq!(last_version, 4, "the newest snapshot reached the wire");
        });
    }
}
