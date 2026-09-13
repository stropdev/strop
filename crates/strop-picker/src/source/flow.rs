//! Source backlog is bounded by retained batch bytes. Producers wait on workers;
//! consuming/dropping a batch returns credit atomically, without locking the UI.
use super::PickerMsg;
use crate::{Item, Payload};
use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
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
}

/// Owned row allocation, excluding shared source text (charged once per run).
pub(super) fn row_bytes(item: &Item) -> usize {
    let path = match &item.payload {
        Payload::File(path) => path.capacity(),
        Payload::Grep { location, .. } => {
            let namespace = match &location.filesystem {
                strop_workspace::Filesystem::Local => 0,
                strop_workspace::Filesystem::Remote(endpoint) => {
                    2 * (endpoint.host().len() + endpoint.user().map_or(0, str::len))
                }
                strop_workspace::Filesystem::Container(id) => id.as_str().len(),
            };
            location.path.capacity().saturating_add(namespace)
        }
        _ => 0,
    };
    std::mem::size_of::<Item>()
        .saturating_add(item.text.capacity())
        .saturating_add(item.badge.as_ref().map_or(0, String::capacity))
        .saturating_add(path)
}

/// Delivery is supplied by the driver; live TUI sources post directly to its
/// event queue instead of allocating a forwarding thread for every request.
#[derive(Clone)]
pub struct SourceSink(Arc<dyn Fn(PickerMsg) -> bool + Send + Sync>);
impl SourceSink {
    pub fn new(emit: impl Fn(PickerMsg) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(emit))
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

#[derive(Clone)]
pub struct StreamSender {
    sender: SourceSink,
    flow: Arc<Flow>,
    budget: Arc<Budget>,
}
impl StreamSender {
    pub(super) fn new(sender: SourceSink, flow: Arc<Flow>) -> Self {
        Self {
            sender,
            flow,
            budget: Arc::default(),
        }
    }
}
impl StreamSender {
    pub(super) fn batch(&self, items: Vec<Item>, cancel: &CancelToken) -> Result<(), BatchError> {
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
                "search batch exceeds 4 MiB; narrow the query",
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
                "search results exceed 100000 rows or 64 MiB; narrow the query",
            ));
        }
        loop {
            if cancel.is_cancelled() {
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
                    return self
                        .sender
                        .send(PickerMsg::Items(ItemBatch {
                            items,
                            permit: Some(permit),
                        }))
                        .then_some(())
                        .ok_or(BatchError::Closed);
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
    /// Control/terminal publication never waits for data credit, including when
    /// invoked by the cancellation handle on the editor thread.
    pub fn control(&self, message: PickerMsg) -> bool {
        debug_assert!(!matches!(message, PickerMsg::Items(_)));
        self.sender.send(message)
    }
}
