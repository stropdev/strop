//! Pure ranking over an immutable catalog snapshot, driven by one owned actor per
//! picker. Query changes cancel CPU work cooperatively; no thread per keystroke.
use crate::Catalog;
use nucleo_matcher::pattern::CaseMatching;
use std::io;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};

mod order;
mod terms;
pub use order::{tree_order, Priors};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Row {
    pub item: usize,
    pub score: u32,
    pub(crate) matches: Range<usize>,
}

/// Rows contain only indices and scores. Discarding a stale result never frees
/// thousands of per-row strings or highlight allocations on the editor thread.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Ranking {
    pub rows: Vec<Row>,
    pub(crate) columns: Vec<u32>,
    item_count: usize,
}
impl Ranking {
    pub fn columns(&self, row: &Row) -> &[u32] {
        &self.columns[row.matches.clone()]
    }
    pub fn item_count(&self) -> usize {
        self.item_count
    }
}
impl<'de> serde::Deserialize<'de> for Ranking {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Wire {
            rows: Vec<Row>,
            columns: Vec<u32>,
            item_count: usize,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.rows.iter().any(|row| {
            row.item >= wire.item_count
                || row.matches.start > row.matches.end
                || row.matches.end > wire.columns.len()
        }) {
            return Err(serde::de::Error::custom(
                "ranking contains an invalid item or match range",
            ));
        }
        Ok(Self {
            rows: wire.rows,
            columns: wire.columns,
            item_count: wire.item_count,
        })
    }
}

#[derive(Debug, Clone)]
pub enum MatchMode {
    Fuzzy(crate::query::CaseMode),
    Exact(Arc<crate::query::ContentExpr>, crate::query::CaseMode),
}

impl Default for MatchMode {
    fn default() -> Self {
        Self::Fuzzy(crate::query::CaseMode::default())
    }
}

#[derive(Clone)]
pub struct FilterRequest {
    pub catalog: Catalog,
    pub query: String,
    pub upstream_filtered: bool,
    /// Count of trailing catalog items that survive any query — pinned
    /// affordances like the remote picker's "Add a host…" (0.21.0 field
    /// report: filtering hid the only way to enter a new destination).
    /// Pinned rows score 0: real matches always rank above them.
    pub pinned_tail: usize,
    pub mode: MatchMode,
    /// The catalog's texts are paths: terms match the file name first and
    /// ties go to the shorter path (0093 §3.2).
    pub paths: bool,
    pub priors: Priors,
}

/// Shared by the actor, hermetic oracles and the scoring benchmark. Pattern
/// parsing and Unicode scratch allocation are per pass, not per candidate.
pub fn rank(
    request: &FilterRequest,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Ranking>, crate::query::QueryDiagnostic> {
    if cancelled() {
        return Ok(None);
    }
    let exact = match &request.mode {
        MatchMode::Exact(expression, case) => Some(crate::query::ContentPlan::compile_expression(
            expression, *case,
        )?),
        _ => None,
    };
    let case = match request.mode {
        MatchMode::Fuzzy(crate::query::CaseMode::Sensitive) => CaseMatching::Respect,
        MatchMode::Fuzzy(crate::query::CaseMode::Ignore) => CaseMatching::Ignore,
        MatchMode::Fuzzy(crate::query::CaseMode::Smart) => CaseMatching::Smart,
        MatchMode::Exact(_, _) => CaseMatching::Ignore,
    };
    let mut fuzzy = None;
    let mut matched = Vec::new();
    // Path lengths of the admitted rows, for the shorter-path tie-break.
    let mut lengths: Vec<u32> = Vec::new();
    let mut ranking = Ranking {
        item_count: request.catalog.len(),
        ..Ranking::default()
    };
    let pinned_from = request.catalog.len().saturating_sub(request.pinned_tail);
    let listing = request.upstream_filtered || request.query.is_empty();
    let reached = request.paths && !request.priors.is_empty();
    for (index, item) in request.catalog.iter().enumerate() {
        if index % 128 == 0 && cancelled() {
            return Ok(None);
        }
        let start = ranking.columns.len();
        matched.clear();
        let score = if index >= pinned_from || request.upstream_filtered {
            Some(0)
        } else if request.query.is_empty() {
            Some(if reached {
                request.priors.standing(&item.text)
            } else {
                0
            })
        } else if let Some(regex) = &exact {
            let mut found = false;
            for hit in regex.find_iter(&item.text) {
                found = true;
                let first = item.text[..hit.start()].chars().count();
                let length = item.text[hit.start()..hit.end()].chars().count();
                matched.extend((first..first + length).map(|column| column as u32));
            }
            found.then_some(1)
        } else {
            let (terms, matcher) = fuzzy.get_or_insert_with(|| {
                (
                    terms::Terms::new(&request.query, case),
                    nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT),
                )
            });
            if request.paths {
                terms
                    .score_path(&item.text, matcher, &mut matched)
                    .map(|score| score + request.priors.bonus(&item.text))
            } else {
                terms.score_text(&item.text, matcher, &mut matched)
            }
        };
        if let Some(score) = score {
            // Terms report their columns one after another; the renderer
            // wants each matched cell once, left to right.
            matched.sort_unstable();
            matched.dedup();
            ranking.columns.extend_from_slice(&matched);
            if request.paths {
                lengths.push(item.text.len() as u32);
            }
            ranking.rows.push(Row {
                item: index,
                score,
                matches: start..ranking.columns.len(),
            });
        }
    }
    if cancelled() {
        return Ok(None);
    }
    if request.upstream_filtered {
        // Hits arrive as workers finish files; the list reads in tree
        // order whatever order that was (0093 §4.2). One flat borrow of
        // the snapshot: the comparator indexes it many times.
        let catalog: Vec<&crate::Item> = request.catalog.iter().collect();
        let in_order = |a: &Row, b: &Row| order::hit_order(catalog[a.item], catalog[b.item]);
        if !ranking.rows.is_sorted_by(|a, b| in_order(a, b).is_le()) {
            ranking.rows.sort_by(in_order);
        }
    } else if !listing && request.paths {
        // Best score, then the shorter path, then tree order. The keys
        // are sorted on their own — no row or item is touched until the
        // order is known — because the first keystrokes match everything.
        let mut keys: Vec<(std::cmp::Reverse<u32>, u32, u32)> = ranking
            .rows
            .iter()
            .zip(&lengths)
            .enumerate()
            .map(|(at, (row, length))| (std::cmp::Reverse(row.score), *length, at as u32))
            .collect();
        keys.sort_unstable();
        ranking.rows = keys
            .into_iter()
            .map(|(_, _, at)| ranking.rows[at as usize].clone())
            .collect();
    } else if !listing || reached {
        ranking
            .rows
            .sort_unstable_by(|a, b| b.score.cmp(&a.score).then(a.item.cmp(&b.item)));
    }
    Ok((!cancelled()).then_some(ranking))
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum RankingEvent<K> {
    Completed(Completion<K, Ranking>),
    Stopped,
}
struct Work<K> {
    ticket: Ticket<K>,
    request: FilterRequest,
}
enum Message<K> {
    Filter(Work<K>),
    Retire(Box<crate::Picker>),
}

pub struct RankingWorker<K> {
    requests: mpsc::Sender<Message<K>>,
    latest: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
}
impl<K: Send + 'static> RankingWorker<K> {
    pub fn start(emit: impl Fn(RankingEvent<K>) -> bool + Send + 'static) -> io::Result<Self> {
        let (requests, receiver) = mpsc::channel::<Message<K>>();
        let latest = Arc::new(AtomicU64::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let current = latest.clone();
        let stopping = closed.clone();
        std::thread::Builder::new()
            .name("picker-rank".into())
            .spawn(move || {
                // Keep the current catalog owned here between requests: dropping the
                // visible picker must not destroy a workspace's strings on the UI.
                let mut retained = Catalog::default();
                'actor: while let Ok(message) = receiver.recv() {
                    let mut work = match message {
                        Message::Filter(work) => work,
                        Message::Retire(picker) => {
                            drop(picker);
                            break;
                        }
                    };
                    while let Ok(message) = receiver.try_recv() {
                        if !emit(RankingEvent::Completed(Completion {
                            ticket: work.ticket,
                            outcome: Outcome::Cancelled(CancelReason::Superseded),
                        })) {
                            return;
                        }
                        work = match message {
                            Message::Filter(newer) => newer,
                            Message::Retire(picker) => {
                                drop(picker);
                                break 'actor;
                            }
                        };
                    }
                    retained = work.request.catalog.clone();
                    let request_id = work.ticket.request.get();
                    let cancelled = || {
                        stopping.load(Ordering::Acquire)
                            || current.load(Ordering::Acquire) != request_id
                    };
                    let outcome =
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            rank(&work.request, cancelled)
                        })) {
                            Ok(Ok(Some(ranking))) => Outcome::Success(ranking),
                            Ok(Ok(None)) => Outcome::Cancelled(CancelReason::Superseded),
                            Ok(Err(diagnostic)) => {
                                Outcome::failed(FailureKind::Protocol, diagnostic.message)
                            }
                            Err(_) => Outcome::failed(FailureKind::Panic, "picker ranking failed"),
                        };
                    if !emit(RankingEvent::Completed(Completion {
                        ticket: work.ticket,
                        outcome,
                    })) {
                        return;
                    }
                }
                drop(retained);
                emit(RankingEvent::Stopped);
            })?;
        Ok(Self {
            requests,
            latest,
            closed,
        })
    }
    /// Revoke the current CPU pass without enqueueing another catalog snapshot.
    pub fn cancel_pending(&self) {
        self.latest.fetch_add(1, Ordering::AcqRel);
    }
    pub fn submit(&self, ticket: Ticket<K>, request: FilterRequest) -> io::Result<()> {
        self.latest.store(ticket.request.get(), Ordering::Release);
        self.requests
            .send(Message::Filter(Work { ticket, request }))
            .map_err(|_| io::Error::other("picker ranking worker stopped"))
    }
    pub fn retire(self, picker: crate::Picker) -> io::Result<()> {
        if let Err(mpsc::SendError(Message::Retire(picker))) =
            self.requests.send(Message::Retire(Box::new(picker)))
        {
            std::thread::Builder::new()
                .name("picker-reclaim".into())
                .spawn(move || drop(picker))?;
        }
        Ok(())
    }
}
impl<K> Drop for RankingWorker<K> {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        // Dropping the sole sender wakes recv; the actor drains/cancels accepted
        // requests and owns reclamation. No join or wait occurs here.
    }
}

#[cfg(test)]
mod corpus_tests;
