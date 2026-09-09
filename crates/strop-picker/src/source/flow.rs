//! Source backlog is bounded by retained batch bytes. Producers wait on workers;
//! consuming/dropping a batch returns credit atomically, without locking the UI.
use super::PickerMsg;
use crate::{Item, Payload};
use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use strop_core::worker::CancelToken;

const BACKLOG_BYTES: usize = 4 * 1024 * 1024;
const CANCEL_POLL: Duration = Duration::from_millis(10);
#[derive(Default)]
struct Flow {
    used: AtomicUsize,
    waiting: Mutex<()>,
    available: Condvar,
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

#[derive(Clone)]
pub struct StreamSender {
    sender: mpsc::Sender<PickerMsg>,
    flow: Arc<Flow>,
}
impl From<mpsc::Sender<PickerMsg>> for StreamSender {
    fn from(sender: mpsc::Sender<PickerMsg>) -> Self {
        Self {
            sender,
            flow: Arc::default(),
        }
    }
}
impl StreamSender {
    pub fn batch(&self, items: Vec<Item>, cancel: &CancelToken) -> bool {
        if items.is_empty() {
            return true;
        }
        let bytes = items
            .iter()
            .fold(0usize, |size, item| {
                size.saturating_add(item.text.len())
                    .saturating_add(match &item.payload {
                        Payload::Grep { line_text, .. } => line_text.len(),
                        _ => 0,
                    })
                    .saturating_add(std::mem::size_of::<Item>())
            })
            .clamp(1, BACKLOG_BYTES);
        loop {
            if cancel.is_cancelled() {
                return false;
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
                        .is_ok();
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
    pub fn control(&self, message: PickerMsg) -> Result<(), mpsc::SendError<PickerMsg>> {
        debug_assert!(!matches!(message, PickerMsg::Items(_)));
        self.sender.send(message)
    }
}
