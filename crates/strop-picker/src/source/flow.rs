//! Source backlog is bounded by retained batch bytes. Producers wait on workers;
//! consuming/dropping a batch returns credit atomically, without locking the UI.
use super::PickerMsg;
use crate::{Item, Payload};
use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::Duration;
use strop_core::worker::CancelToken;

pub(super) const BACKLOG_BYTES: usize = 4 * 1024 * 1024;
const CATALOG_BYTES: usize = 64 * 1024 * 1024;
const CATALOG_ITEMS: usize = 100_000;
const CANCEL_POLL: Duration = Duration::from_millis(10);
#[derive(Default)]
pub(super) struct Flow {
    used: AtomicUsize,
    waiting: Mutex<()>,
    available: Condvar,
}
#[derive(Default)]
struct Budget {
    retained: AtomicUsize,
    items: AtomicUsize,
}
struct Permit {
    flow: Arc<Flow>,
    bytes: usize,
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.flow.used.fetch_sub(self.bytes, Ordering::Release);
        self.flow.available.notify_all();
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ItemBatch {
    items: Vec<Item>,
    #[serde(skip)]
    permit: Option<Arc<Permit>>,
}
impl std::fmt::Debug for ItemBatch {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.items.fmt(out)
    }
}
impl ItemBatch {
    pub fn into_items(self) -> Vec<Item> {
        let Self { items, permit } = self;
        drop(permit);
        items
    }
}
impl std::ops::Deref for ItemBatch {
    type Target = [Item];
    fn deref(&self) -> &[Item] {
        &self.items
    }
}
impl From<Vec<Item>> for ItemBatch {
    fn from(items: Vec<Item>) -> Self {
        Self {
            items,
            permit: None,
        }
    }
}

#[derive(Debug)]
pub(super) enum BatchError {
    Cancelled,
    Closed,
    Bound(&'static str),
}
impl BatchError {
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::Cancelled => "search cancelled",
            Self::Closed => "search consumer closed",
            Self::Bound(message) => message,
        }
    }
    pub(super) fn outcome(self) -> strop_core::worker::Outcome<()> {
        use strop_core::worker::{CancelReason, FailureKind, Outcome};
        match self {
            Self::Cancelled => Outcome::Cancelled(CancelReason::Superseded),
            Self::Closed => Outcome::failed(FailureKind::Disconnected, self.message()),
            Self::Bound(message) => Outcome::failed(FailureKind::Unavailable, message),
        }
    }
    /// A catalog bound is a truncation, not a failure (the workspace-
    /// symbols tier's warning precedent): the source stops walking, the
    /// results that already streamed stay ranked and openable, and the
    /// honest note — not a red error — says why the catalog is partial.
    /// Real source failures keep their error outcome.
    pub(super) fn finish(self, tx: &StreamSender) -> strop_core::worker::Outcome<()> {
        match self {
            Self::Bound(message) => {
                tx.mark_partial();
                let _ = tx.control(crate::source::PickerMsg::Warning(message.into()));
                strop_core::worker::Outcome::Success(())
            }
            other => other.outcome(),
        }
    }
}

/// Owned row allocation, excluding shared source text (charged once per run).
pub(super) fn row_bytes(item: &Item) -> usize {
    let path = match &item.payload {
        Payload::File(path) => path.capacity(),
        Payload::Grep { location, .. } | Payload::Symbol { location, .. } => {
            let namespace = match &location.filesystem {
                strop_workspace::Filesystem::Local => 0,
                strop_workspace::Filesystem::Remote(endpoint) => {
                    2 * (endpoint.host().len() + endpoint.user().map_or(0, str::len))
                }
            };
            location.path.capacity().saturating_add(namespace)
        }
        _ => 0,
    };
    std::mem::size_of::<Item>()
        .saturating_add(item.text.capacity())
        .saturating_add(match &item.badge {
            Some(std::borrow::Cow::Owned(badge)) => badge.capacity(),
            _ => 0,
        })
        .saturating_add(path)
}

/// One request's worker and event loop share this latch. It holds no rows,
/// channel or lock. Ordinary (including SSH) providers never bind an epoch.
#[derive(Clone, Default)]
pub struct InventoryGuard(Arc<Observation>);

#[derive(Default)]
struct Observation {
    epoch: OnceLock<Arc<AtomicBool>>,
    symbols: OnceLock<WorkspaceScope>,
}

struct WorkspaceScope {
    paths: Arc<Vec<std::path::PathBuf>>,
    order: Vec<usize>,
    catalog: Arc<super::catalog::ProjectCatalog>,
    index: Arc<super::symbols::SymbolIndex>,
}

impl InventoryGuard {
    pub fn is_current(&self) -> bool {
        self.0
            .epoch
            .get()
            .is_none_or(|epoch| epoch.load(Ordering::Acquire))
    }

    pub fn workspace_witness(&self, relative: &std::path::Path) -> Option<Arc<crate::DiskShard>> {
        self.0.symbols.get()?.index.disk_shard(relative).cloned()
    }

    /// A single action needs only the epoch, not a whole workspace index.
    pub fn epoch(&self) -> Option<Arc<AtomicBool>> {
        self.0.epoch.get().cloned()
    }

    /// A semantic producer must serve an admitted native path and its
    /// deepest observed project. A truncated catalog cannot grant ownership.
    pub fn symbol_owner(&self, relative: &std::path::Path) -> Option<&std::path::Path> {
        if !self.is_current() {
            return None;
        }
        let scope = self.0.symbols.get()?;
        scope
            .order
            .binary_search_by(|at| scope.paths[*at].as_path().cmp(relative))
            .ok()?;
        Some(&scope.catalog.project_for(relative)?.root)
    }
}

/// Delivery is supplied by the driver; live TUI sources post directly to its
/// event queue instead of allocating a forwarding thread for every request.
#[derive(Clone)]
pub struct SourceSink(Arc<dyn Fn(PickerMsg) -> bool + Send + Sync>, InventoryGuard);
impl SourceSink {
    pub fn new(emit: impl Fn(PickerMsg) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(emit), InventoryGuard::default())
    }
    pub fn inventory_guard(&self) -> InventoryGuard {
        self.1.clone()
    }
    pub fn send(&self, message: PickerMsg) -> bool {
        self.0(message)
    }
}
impl From<mpsc::Sender<PickerMsg>> for SourceSink {
    fn from(sender: mpsc::Sender<PickerMsg>) -> Self {
        Self::new(move |message| sender.send(message).is_ok())
    }
}

#[derive(Default)]
pub(super) struct BatchProgress {
    ticks: AtomicUsize,
    /// Producers currently blocked on delivery credit. A count, not a
    /// flag: parallel search threads wait and resume independently.
    delivery_waiting: AtomicUsize,
}
impl BatchProgress {
    pub(super) fn get(&self) -> usize {
        self.ticks.load(Ordering::Acquire)
    }
    fn advance(&self) {
        self.ticks.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn delivery_waiting(&self) -> bool {
        self.delivery_waiting.load(Ordering::Acquire) > 0
    }
    fn wait_for_delivery(&self) -> DeliveryWait<'_> {
        self.delivery_waiting.fetch_add(1, Ordering::AcqRel);
        DeliveryWait { progress: self }
    }
}
struct DeliveryWait<'a> {
    progress: &'a BatchProgress,
}
impl Drop for DeliveryWait<'_> {
    fn drop(&mut self) {
        self.progress
            .delivery_waiting
            .fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub struct StreamSender {
    sender: SourceSink,
    flow: Arc<Flow>,
    budget: Arc<Budget>,
    progress: Arc<BatchProgress>,
}
impl StreamSender {
    pub(super) fn new(sender: SourceSink, flow: Arc<Flow>, progress: Arc<BatchProgress>) -> Self {
        Self {
            sender,
            flow,
            budget: Arc::default(),
            progress,
        }
    }
}
impl StreamSender {
    /// Process-local identity, valid only while this request or one of its
    /// worker clones owns the progress allocation. Never persist this token.
    pub(super) fn request_id(&self) -> u64 {
        Arc::as_ptr(&self.progress) as usize as u64
    }

    pub(super) fn mark_partial(&self) {
        let _ = self.control(crate::source::PickerMsg::PartialCoverage);
    }

    pub(super) fn bind_inventory(&self, epoch: Arc<AtomicBool>) -> bool {
        self.sender.1 .0.epoch.set(epoch).is_ok() && self.inventory_current()
    }

    pub(super) fn workspace_scope(
        &self,
        paths: Arc<Vec<std::path::PathBuf>>,
        catalog: Arc<super::catalog::ProjectCatalog>,
        index: Arc<super::symbols::SymbolIndex>,
    ) {
        let mut order: Vec<_> = (0..paths.len()).collect();
        order.sort_unstable_by(|a, b| paths[*a].cmp(&paths[*b]));
        let _ = self.sender.1 .0.symbols.set(WorkspaceScope {
            paths,
            order,
            catalog,
            index,
        });
    }

    pub(super) fn inventory_current(&self) -> bool {
        self.sender.1.is_current()
    }

    pub(super) fn batch(&self, items: Vec<Item>, cancel: &CancelToken) -> Result<(), BatchError> {
        if !self.inventory_current() {
            return Err(BatchError::Cancelled);
        }
        if items.is_empty() {
            return Ok(());
        }
        let mut bytes = items
            .capacity()
            .saturating_sub(items.len())
            .saturating_mul(std::mem::size_of::<Item>());
        let mut previous: Option<&Arc<str>> = None;
        for item in &items {
            bytes = bytes.saturating_add(row_bytes(item));
            if let Payload::Grep { line_text, .. } = &item.payload {
                if previous.is_none_or(|text| !Arc::ptr_eq(text, line_text)) {
                    bytes = bytes.saturating_add(line_text.len());
                }
                previous = Some(line_text);
            }
        }
        if bytes > BACKLOG_BYTES {
            return Err(BatchError::Bound(
                "one result batch exceeds 4 MiB; kept what streamed — narrow the query",
            ));
        }
        if self
            .budget
            .items
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count
                    .checked_add(items.len())
                    .filter(|total| *total <= CATALOG_ITEMS)
            })
            .is_err()
            || self
                .budget
                .retained
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    used.checked_add(bytes)
                        .filter(|total| *total <= CATALOG_BYTES)
                })
                .is_err()
        {
            return Err(BatchError::Bound(
                "results exceed 100,000 rows or 64 MiB; kept what streamed — narrow the query",
            ));
        }

        let _delivery_wait = self.progress.wait_for_delivery();
        loop {
            if cancel.is_cancelled() || !self.inventory_current() {
                return Err(BatchError::Cancelled);
            }
            let used = self.flow.used.load(Ordering::Acquire);
            if used + bytes <= BACKLOG_BYTES {
                if self
                    .flow
                    .used
                    .compare_exchange_weak(used, used + bytes, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
                {
                    let permit = Arc::new(Permit {
                        flow: self.flow.clone(),
                        bytes,
                    });
                    let delivered = self.sender.send(PickerMsg::Items(ItemBatch {
                        items,
                        permit: Some(permit),
                    }));
                    if delivered {
                        self.progress.advance();
                    }
                    return delivered.then_some(()).ok_or(BatchError::Closed);
                }
                continue;
            }
            let mut guard = self.flow.waiting.lock();
            if self.flow.used.load(Ordering::Acquire) + bytes > BACKLOG_BYTES
                && !cancel.is_cancelled()
            {
                self.flow.available.wait_for(&mut guard, CANCEL_POLL);
            }
        }
    }
    /// Filesystem walks and matchless file scans are live even when they
    /// produce no batches. Delivery backpressure is tracked separately.
    pub(super) fn heartbeat(&self) {
        self.progress.advance();
    }
    /// Remote admission and execution have worker/network timeout ownership.
    /// Their silence is not evidence of a local filesystem stall.
    pub(super) fn remote_work(&self) -> impl Drop + use<'_> {
        self.progress.wait_for_delivery()
    }
    /// Control/terminal publication never waits for data credit, including when
    /// invoked by the cancellation handle on the editor thread.
    pub fn control(&self, message: PickerMsg) -> bool {
        debug_assert!(!matches!(message, PickerMsg::Items(_)));
        if !self.inventory_current() {
            return matches!(message, PickerMsg::Finished(_))
                && self
                    .sender
                    .send(PickerMsg::Finished(strop_core::worker::Outcome::Cancelled(
                        strop_core::worker::CancelReason::Superseded,
                    )));
        }
        self.sender.send(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: String) -> Item {
        Item {
            badge: None,
            text: name.clone(),
            payload: Payload::File(std::path::PathBuf::from(name)),
            facts: Default::default(),
        }
    }

    #[test]
    fn retired_inventory_revokes_queued_rows_warm_candidates_and_success() {
        let (tx, rx) = mpsc::channel();
        let sink: SourceSink = tx.into();
        let queued_guard = sink.inventory_guard();
        let stream = StreamSender::new(sink, Arc::default(), Arc::default());
        let epoch = Arc::new(AtomicBool::new(true));
        assert!(stream.bind_inventory(epoch.clone()));
        let (token, _handle) = CancelToken::standalone();
        stream.batch(vec![item("before".into())], &token).unwrap();
        epoch.store(false, Ordering::Release);
        assert!(matches!(rx.recv().unwrap(), PickerMsg::Items(_)));
        assert!(
            !queued_guard.is_current(),
            "already queued rows are ineligible on the event loop"
        );
        assert!(matches!(
            stream.batch(vec![item("late".into())], &token),
            Err(BatchError::Cancelled)
        ));
        assert!(!stream.control(PickerMsg::ScopeProjects(Vec::new())));
        assert!(stream.control(PickerMsg::Finished(
            strop_core::worker::Outcome::Success(())
        )));
        assert!(matches!(
            rx.recv().unwrap(),
            PickerMsg::Finished(strop_core::worker::Outcome::Cancelled(_))
        ));
    }

    /// The catalog bound is a truncation, not a failure: the source
    /// stops walking, the already-streamed rows stay (their permits
    /// release credit), the note is a warning — never a red error —
    /// and the finished outcome is success so Enter keeps working.
    #[test]
    fn the_catalog_bound_finishes_with_a_truncation_warning() {
        let controls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&controls);
        let sink = SourceSink::new(move |message| {
            // keep control traffic only: dropping item batches releases
            // their backlog credit, exactly like the TUI consumer
            if !matches!(message, PickerMsg::Items(_)) {
                seen.lock().push(message);
            }
            true
        });
        let sender = StreamSender::new(
            sink,
            Arc::new(Flow::default()),
            Arc::new(BatchProgress::default()),
        );
        let (cancel, _handle) = CancelToken::standalone();
        let mut bound = None;
        for index in 0..400usize {
            let batch = (0..512)
                .map(|n| item(format!("f{}", index * 512 + n)))
                .collect();
            if let Err(error) = sender.batch(batch, &cancel) {
                bound = Some(error);
                break;
            }
        }
        let error = bound.expect("the catalog budget stops delivery");
        let outcome = error.finish(&sender);
        assert!(
            matches!(outcome, strop_core::worker::Outcome::Success(())),
            "a bound is a truncation, not a source failure"
        );
        let controls = controls.lock();
        assert!(controls
            .iter()
            .any(|m| matches!(m, PickerMsg::PartialCoverage)));
        let warning = controls.iter().find_map(|message| match message {
            PickerMsg::Warning(message) => Some(message),
            _ => None,
        });
        assert!(
            warning.is_some_and(|message| message.contains("100,000")),
            "the honest truncation note: {controls:?}"
        );
    }

    #[test]
    fn heartbeats_and_delivery_backpressure_are_distinct_liveness_signals() {
        let progress = Arc::new(BatchProgress::default());
        let sender = StreamSender::new(
            SourceSink::new(|_| true),
            Arc::new(Flow::default()),
            progress.clone(),
        );
        sender.heartbeat();
        assert_eq!(progress.get(), 1);
        assert!(!progress.delivery_waiting());
        let waiting = progress.wait_for_delivery();
        assert!(progress.delivery_waiting());
        drop(waiting);
        assert!(!progress.delivery_waiting());
    }

    /// Real delivery failures keep their failure outcome — only the
    /// catalog bound downgrades to a warning.
    #[test]
    fn a_closed_consumer_still_fails_the_source() {
        let sender = StreamSender::new(
            SourceSink::new(|_| false),
            Arc::new(Flow::default()),
            Arc::new(BatchProgress::default()),
        );
        let (cancel, _handle) = CancelToken::standalone();
        let error = sender
            .batch(vec![item("f1".into())], &cancel)
            .expect_err("a closed sink fails delivery");
        assert!(matches!(
            error.finish(&sender),
            strop_core::worker::Outcome::Failed { .. }
        ));
    }
}
