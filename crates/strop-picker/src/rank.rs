//! Pure ranking over an immutable catalog snapshot, driven by one owned actor per
//! picker. Query changes cancel CPU work cooperatively; no thread per keystroke.
use crate::Catalog;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use std::io;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};

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
}

/// Shared by the actor, hermetic oracles and the scoring benchmark. Pattern
/// parsing and Unicode scratch allocation are per pass, not per candidate.
pub fn rank(request: &FilterRequest, cancelled: impl Fn() -> bool) -> Option<Ranking> {
    let pattern = Pattern::parse(&request.query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT);
    let mut unicode = Vec::new();
    let mut matched = Vec::new();
    let mut ranking = Ranking {
        item_count: request.catalog.len(),
        ..Ranking::default()
    };
    let pinned_from = request.catalog.len().saturating_sub(request.pinned_tail);
    for (index, item) in request.catalog.iter().enumerate() {
        if index % 128 == 0 && cancelled() {
            return None;
        }
        let start = ranking.columns.len();
        matched.clear();
        let score = if index >= pinned_from || request.upstream_filtered || request.query.is_empty()
        {
            Some(0)
        } else {
            let text = if item.text.is_ascii() {
                nucleo_matcher::Utf32Str::Ascii(item.text.as_bytes())
            } else {
                unicode.clear();
                unicode.extend(item.text.chars());
                nucleo_matcher::Utf32Str::Unicode(&unicode)
            };
            pattern.indices(text, &mut matcher, &mut matched)
        };
        if let Some(score) = score {
            ranking.columns.extend_from_slice(&matched);
            ranking.rows.push(Row {
                item: index,
                score,
                matches: start..ranking.columns.len(),
            });
        } else {
            ranking.columns.truncate(start);
        }
    }
    if cancelled() {
        return None;
    }
    if !request.upstream_filtered && !request.query.is_empty() {
        ranking
            .rows
            .sort_unstable_by(|a, b| b.score.cmp(&a.score).then(a.item.cmp(&b.item)));
    }
    (!cancelled()).then_some(ranking)
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
                            Ok(Some(ranking)) => Outcome::Success(ranking),
                            Ok(None) => Outcome::Cancelled(CancelReason::Superseded),
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
