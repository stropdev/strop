//! Subscription lifecycle: state, the in-process watcher thread, and the
//! checked read a guarded reload performs.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use strop_core::id::DocumentId;
use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};
use strop_fs::notify::{NotifyConfig, NotifyManager, Subscription};
use strop_workspace::ResourceLocation;

use super::super::events::AppEvent;
use super::super::io::Opened;
use super::super::{Document, Editor};
use super::{Record, ReloadKey, SubscribedScope};

/// How long one watcher poll waits; it also bounds how late the thread
/// notices it was stopped.
const WATCH_TICK: Duration = Duration::from_millis(25);

/// The editor's notification state: subscription identity, the watcher
/// thread's stop latch and the in-flight guarded reloads.
pub(crate) struct NotifyState {
    pub(crate) queue: Arc<super::NotifyQueue>,
    /// The live subscription identity; events stamped otherwise are dead.
    pub(crate) subscription: Option<Subscription>,
    pub(super) subscribing: bool,
    /// Set to stop the watcher thread (it owns the inotify instances).
    pub(super) stop: Arc<AtomicBool>,
    pub(super) reloads: HashMap<strop_core::worker::WorkerId, ReloadKey>,
    pub(super) reloading: HashMap<DocumentId, strop_core::worker::WorkerId>,
    /// Hints arrived while a reload was in flight: one re-observation
    /// owed at completion so a raced write is never stranded.
    pub(super) reload_again: HashSet<DocumentId>,
}

impl Default for NotifyState {
    fn default() -> Self {
        Self {
            queue: Arc::new(super::NotifyQueue::new()),
            subscription: None,
            subscribing: false,
            stop: Arc::new(AtomicBool::new(false)),
            reloads: HashMap::new(),
            reloading: HashMap::new(),
            reload_again: HashSet::new(),
        }
    }
}

impl Drop for NotifyState {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl NotifyState {
    /// Finite notify work in flight (guarded reloads). The subscription
    /// settles onto the notify queue, not the io channel, so it never
    /// parks an io drain.
    pub(crate) fn pending(&self) -> bool {
        !self.reloads.is_empty()
    }

    /// Whether native watching covers the scope (search baselines may
    /// trust hints instead of rescanning).
    pub(crate) fn push_coverage(&self) -> bool {
        self.subscription.is_some()
    }

    /// A subscription is being set up: its settle is not stamped, so the
    /// scope must not change under it (`:cd` waits).
    pub(crate) fn subscribing(&self) -> bool {
        self.subscribing
    }

    /// Stop the watcher thread; its inotify instances close with it.
    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.subscription = None;
    }
}

/// Observe and read one local file for a guarded reload: the same
/// checked observation/read family as open.
/// `None` means the resource vanished or stopped being a regular file —
/// an outcome, never a clobber.
pub(super) fn reload_run(
    local: &crate::editor::namespace::LocalFs,
    path: &std::path::Path,
    cancel: &CancelToken,
) -> Outcome<Option<Opened>> {
    if cancel.is_cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    let location = ResourceLocation::local(path.to_path_buf());
    let observed = match local.observe(cancel, std::slice::from_ref(&location)) {
        Ok(mut observations) => observations.pop().and_then(|located| located.value),
        Err(error) if error.kind == strop_workspace::operation::FsFailureKind::Cancelled => {
            return Outcome::Cancelled(CancelReason::OwnerClosed)
        }
        Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
    };
    let Some(observation) = observed else {
        return Outcome::Success(None);
    };
    if observation.kind != strop_workspace::EntryKind::File {
        return Outcome::Success(None);
    }
    let payload = match local.read(cancel, &location) {
        Ok(payload) => payload,
        Err(error) if error.kind == strop_workspace::operation::FsFailureKind::Cancelled => {
            return Outcome::Cancelled(CancelReason::OwnerClosed)
        }
        Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
    };
    let rope = match ropey::Rope::from_reader(payload.as_slice()) {
        Ok(rope) => rope,
        Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
    };
    let writable = observation
        .permissions
        .is_none_or(|permissions| permissions.bits() & 0o222 != 0);
    let stamp = observation
        .modified
        .map(super::super::io::open::filetime_to_systemtime);
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Outcome::Success(Some(Opened {
        document: Document::new(strop_core::Buffer::from_read(
            path.to_path_buf(),
            rope,
            stamp,
            canonical.clone(),
            writable,
        )),
        canonical: crate::files::FileTarget::Local(canonical),
    }))
}

impl Editor {
    /// Watch the editor's directory recursively on one thread that owns
    /// the inotify manager: it subscribes, settles onto the notify queue,
    /// then pumps hints there until stopped. No-op under replay, while
    /// subscribed/subscribing, or when finishing. Never blocks input:
    /// the registration walk runs on that thread.
    pub(crate) fn start_notifications(&mut self) {
        if self.finishing
            || self.notify.subscribing
            || self.notify.subscription.is_some()
            || self.tape.is_replay()
        {
            return;
        }
        self.notify.subscribing = true;
        let stop = Arc::new(AtomicBool::new(false));
        self.notify.stop = Arc::clone(&stop);
        let scope = ResourceLocation::local(self.cwd.clone());
        let queue = Arc::clone(&self.notify.queue);
        let wake = self.app_tx.clone();
        let notify = move || {
            if let Some(wake) = &wake {
                let _ = wake.send(AppEvent::Notify);
            }
        };
        let spawned = std::thread::Builder::new()
            .name("fs-notify".into())
            .spawn(move || {
                let owned_trace = strop_trace::active_path().and_then(|path| {
                    std::fs::canonicalize(&scope.path).ok().and_then(|root| {
                        path.strip_prefix(root).ok().map(|relative| {
                            strop_workspace::addr::uri::path_bytes(relative).to_vec()
                        })
                    })
                });
                let mut manager = NotifyManager::new(NotifyConfig::default());
                let settled = match manager.subscribe(&scope, true) {
                    Ok(subscription) => Outcome::Success(SubscribedScope {
                        subscription,
                        owned_trace,
                    }),
                    Err(error) => Outcome::failed(FailureKind::Unavailable, error.to_string()),
                };
                let watching = matches!(settled, Outcome::Success(_));
                queue.push_record(Record::Settled(settled));
                notify();
                while watching && !stop.load(Ordering::Acquire) {
                    let events = manager.poll(Some(WATCH_TICK)).and_then(|_| manager.drain());
                    match events {
                        Ok(events) => {
                            let mut woke = false;
                            for event in events {
                                woke |= queue.push_event(event);
                            }
                            if woke {
                                notify();
                            }
                        }
                        Err(error) => {
                            queue.push_record(Record::Settled(Outcome::failed(
                                FailureKind::Io,
                                error.to_string(),
                            )));
                            notify();
                            return;
                        }
                    }
                }
            });
        if let Err(error) = spawned {
            self.notify.subscribing = false;
            self.message = format!("filesystem notifications unavailable: {error}");
        }
    }
}
