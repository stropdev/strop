use crate::model::{Effect, SessionId, Update};
use parking_lot::Mutex;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(super) struct Mailbox {
    value: Mutex<Option<Update>>,
    notified: AtomicBool,
    session: SessionId,
    notify: Arc<dyn Fn(SessionId) + Send + Sync>,
}
impl Mailbox {
    pub fn new(session: SessionId, notify: impl Fn(SessionId) + Send + Sync + 'static) -> Self {
        Self {
            value: Mutex::new(None),
            notified: AtomicBool::new(false),
            session,
            notify: Arc::new(notify),
        }
    }
    pub fn pending(&self) -> bool {
        self.notified.load(Ordering::Acquire)
    }
    pub fn publish(&self, update: Update) {
        self.publish_with(update, || {});
    }
    pub fn publish_with(&self, mut update: Update, complete: impl FnOnce()) {
        let mut slot = self.value.lock();
        if let Some(mut previous) = slot.take() {
            if update.frame.is_none() {
                update.frame = previous.frame.take();
            }
            if update.warning.is_none() {
                update.warning = previous.warning.take();
            }
            for effect in update.effects.drain(..) {
                merge_effect(&mut previous.effects, effect);
            }
            update.effects = previous.effects;
        }
        *slot = Some(update);
        let notify = !self.notified.swap(true, Ordering::AcqRel);
        complete();
        drop(slot);
        if notify {
            (self.notify)(self.session);
        }
    }
    pub fn take(&self) -> Option<Update> {
        let Some(mut slot) = self.value.try_lock() else {
            // The current notification was consumed but publication owns the
            // slot briefly. Requeue its one wake rather than block the UI.
            (self.notify)(self.session);
            return None;
        };
        let update = slot.take();
        self.notified.store(false, Ordering::Release);
        update
    }
}
fn merge_effect(effects: &mut Vec<Effect>, effect: Effect) {
    if let Some(old) = effects
        .iter_mut()
        .find(|old| std::mem::discriminant(*old) == std::mem::discriminant(&effect))
    {
        *old = effect;
    } else {
        effects.push(effect);
    }
}
