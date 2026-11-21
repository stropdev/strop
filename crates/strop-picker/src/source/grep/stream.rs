//! Plain local text needs no global catalog or syntax inventory. Each worker
//! directly verifies the current walk entry; another may discover the next.
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use grep::searcher::SearcherBuilder;
use parking_lot::Mutex;
use strop_core::worker::{CancelReason, FailureKind, Outcome};
use strop_kernel::{
    admission::{admits_origin, Origin, Surface},
    coverage::{self, Evidence, Hint, PathWork, Schedule, Terminal},
};

use super::{disk, SourceSnapshot};
use crate::query::{Evidence as RowEvidence, FileSelectionPlan};
use crate::source::{
    cache::{Caches, InventoryError, SearchScan},
    flow,
    selection::{self, SelectionPolicy},
    snapshots::emit_rope,
};
use crate::{Item, Payload};

/// Coalesce short file tails so a common one-hit-per-file query doesn't post
/// thousands of individual ranking requests. The first row never waits for a
/// second file. Large individual deliveries still use the ordinary flow caps.
#[derive(Default)]
struct Group {
    sent_first: bool,
    pending: Vec<Item>,
    bytes: usize,
}

impl Group {
    fn push(&mut self, mut items: Vec<Item>) -> Option<Vec<Item>> {
        if !self.sent_first || items.len() >= 64 {
            self.sent_first = true;
            return Some(items);
        }
        let bytes: usize = items
            .iter()
            .map(|item| {
                flow::row_bytes(item)
                    + match &item.payload {
                        Payload::Grep { line_text, .. } => line_text.len(),
                        _ => 0,
                    }
            })
            .sum();
        // A large row is valid by itself. Never create an over-limit batch
        // by adding it to previously valid rows. Remaining slack covers Vec
        // capacity growth (the group holds at most 126 small rows).
        if bytes > 128 * 1024 {
            return Some(items);
        }
        self.bytes += bytes;
        self.pending.append(&mut items);
        if self.pending.len() >= 64 || self.bytes >= 128 * 1024 {
            self.bytes = 0;
            return Some(std::mem::take(&mut self.pending));
        }
        None
    }
}

/// One small status per eligible path (including zero-hit dirty ropes),
/// accumulated without taking a shared lock per file. No path list survives.
struct Ledger<'a> {
    paths: Vec<PathWork>,
    all: &'a Mutex<Vec<PathWork>>,
}
impl Drop for Ledger<'_> {
    fn drop(&mut self) {
        self.all.lock().append(&mut self.paths);
    }
}

pub(super) fn search(
    scope: &disk::Scope<'_>,
    cache: &Mutex<Caches>,
    plan: &FileSelectionPlan,
    policy: SelectionPolicy,
    snapshots: &[SourceSnapshot],
    threads: usize,
) -> Outcome<()> {
    let scan = match SearchScan::begin(cache, scope.cwd) {
        Ok(scan) => scan,
        Err(error) => return error.outcome("search"),
    };
    if !scope.tx.bind_inventory(scan.epoch.clone()) {
        return Outcome::Cancelled(CancelReason::Superseded);
    }
    let matcher = match disk::matcher(scope.content) {
        Ok(matcher) => matcher,
        Err(outcome) => return outcome,
    };
    let mut dirty = HashMap::new();
    for snapshot in snapshots {
        if let Ok(path) = snapshot.path.strip_prefix(scope.cwd) {
            dirty.entry(path).or_insert(&snapshot.text);
        }
    }
    let group = Mutex::new(Group::default());
    let deliver = |items| {
        scope
            .tx
            .batch(items, scope.cancel)
            .map_err(|e| e.finish(scope.tx))
    };
    let send = |items| {
        let batch = group.lock().push(items);
        batch.map_or(Ok(()), &deliver)
    };
    let failure = OnceLock::new();
    let stopped = || {
        scope.tx.heartbeat();
        scope.cancel.is_cancelled() || !scope.tx.inventory_current() || failure.get().is_some()
    };
    let all = Mutex::new(Vec::new());
    let eager = std::sync::atomic::AtomicBool::new(true);
    let lane = || {
        let mut searcher = SearcherBuilder::new().line_number(true).build();
        let mut ledger = Ledger {
            paths: Vec::new(),
            all: &all,
        };
        let (dirty, matcher, send, failure, scan) = (&dirty, &matcher, &send, &failure, &scan);
        let eager = &eager;
        move |path: &Path| {
            let buffer = dirty.get(path);
            let surface = if buffer.is_some() {
                Surface::BufferPending
            } else {
                Surface::Disk
            };
            let verified = if admits_origin(surface, Origin::Disk) {
                disk::search_file(scope, matcher, &mut searcher, path, eager, send)
            } else if let Some(rope) = buffer {
                emit_rope(
                    rope,
                    path,
                    &strop_workspace::ResourceLocation::local(scope.cwd.join(path)),
                    scope.content,
                    RowEvidence::default(),
                    scope.tx,
                    scope.cancel,
                )
                .map_err(|error| error.finish(scope.tx))
            } else {
                Err(Outcome::failed(
                    FailureKind::Protocol,
                    "search source owner is unavailable",
                ))
            };
            match verified {
                Ok(hit) => {
                    ledger.paths.push(PathWork::Verified);
                    if hit {
                        scan.note_hit(path);
                    }
                    true
                }
                Err(outcome) => {
                    ledger.paths.push(PathWork::Verifying);
                    let _ = failure.set(outcome);
                    false
                }
            }
        }
    };
    let hint = if scan.hint.is_some() {
        Hint::LikelyHit
    } else {
        Hint::None
    };
    let priority = match coverage::schedule(Evidence::Unknown, hint) {
        Schedule::ExamineFirst => scan
            .hint
            .as_deref()
            .filter(|p| selection::admits_hint(scope.cwd, plan, policy, p, &stopped)),
        _ => None,
    };
    let checked = priority.filter(|path| lane()(path));
    let walk = selection::stream(scope.cwd, plan, policy, &stopped, threads, checked, lane);
    if scope.cancel.is_cancelled() || !scan.current(cache) {
        return Outcome::Cancelled(CancelReason::Superseded);
    }
    let exhausted = matches!(walk, Ok(true));
    let result = match (failure.into_inner(), walk) {
        (Some(outcome), _) => Err(outcome),
        (_, Err(error)) => Err(InventoryError::from(error).outcome("search")),
        _ => deliver(group.into_inner().pending),
    };
    let terminal = match &result {
        Ok(()) => Terminal::Success,
        // The established row-cap contract is warning + successful terminal,
        // but a stopped verifier cannot certify a complete (or empty) answer.
        Err(Outcome::Success(())) => Terminal::Truncated,
        Err(_) => Terminal::Failed,
    };
    let id = scope.tx.request_id();
    let query = coverage::Query {
        scope: Arc::as_ptr(&scan.epoch) as usize as u64,
        policy: id,
        generation: id,
    };
    // All children and loose files use this single native traversal. Text
    // search has no independent semantic-child obligations.
    let settlement = coverage::settle(query, None, exhausted, terminal, &all.into_inner());
    if !coverage::workspace_complete(&query, &settlement, &[]) {
        if let Err(outcome) = result {
            return outcome;
        }
        scope.tx.mark_partial();
        return Outcome::failed(
            FailureKind::Unavailable,
            "search enumeration was interrupted",
        );
    }
    if !scan.finish(cache) {
        return Outcome::Cancelled(CancelReason::Superseded);
    }
    Outcome::Success(())
}
