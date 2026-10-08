//! Subscription lifecycle: state, guarded local/remote subscribe jobs,
//! and the checked read a guarded reload performs.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use strop_core::id::DocumentId;
use strop_core::worker::{self, CancelReason, CancelToken, FailureKind, Outcome};
use strop_worker_protocol::message::NotifyCoverage;
use strop_worker_protocol::{Event, NotifyHint, Session, Subscription};
use strop_workspace::{Filesystem, ResourceLocation};

use super::super::directory::DirectoryTask;
use super::super::events::AppEvent;
use super::super::io::Opened;
use super::super::{Document, DocumentSource, Editor};
use super::{NotifyQueue, Record, ReloadKey, SubscribedScope};

/// The editor's notification state: subscription identity, coverage,
/// the in-flight guarded reloads and the honest coverage note.
pub(crate) struct NotifyState {
    pub(crate) queue: Arc<NotifyQueue>,
    pub(super) tx: Sender<Event>,
    pub(crate) rx: Option<Receiver<Event>>,
    /// The live subscription identity; events stamped otherwise are dead.
    pub(crate) subscription: Option<Subscription>,
    /// The worker session the subscription belongs to.
    pub(super) session: Option<Session>,
    pub(super) coverage: Option<NotifyCoverage>,
    pub(super) subscribing: bool,
    /// The subscribe job's cancel handle; retained until settle
    /// (dropping it cancels the job).
    pub(super) subscribe_handle: Option<worker::CancelHandle>,
    pub(super) reloads: HashMap<strop_core::worker::WorkerId, ReloadKey>,
    pub(super) reloading: HashMap<DocumentId, strop_core::worker::WorkerId>,
    /// Hints arrived while a reload was in flight: one re-observation
    /// owed at completion so a raced write is never stranded.
    pub(super) reload_again: HashSet<DocumentId>,
    /// Remote workspace scopes (WK07): one subscription per opened
    /// remote directory root on a worker-admitted endpoint.
    pub(super) remote: HashMap<ResourceLocation, RemoteScope>,
}

/// One remote workspace scope's subscription state. The subscription's
/// authority is its (session, scope, generation) identity exactly as
/// for the local scope; a dead worker incarnation's events are dropped.
pub(crate) struct RemoteScope {
    pub(super) endpoint: strop_workspace::RemoteEndpoint,
    pub(super) subscription: Option<Subscription>,
    pub(super) session: Option<Session>,
    pub(super) subscribing: bool,
    /// The subscribe job's cancel handle, retained until settle.
    pub(super) subscribe_handle: Option<worker::CancelHandle>,
}

impl Default for NotifyState {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            queue: Arc::new(NotifyQueue::new()),
            tx,
            rx: Some(rx),
            subscription: None,
            session: None,
            subscribe_handle: None,
            coverage: None,
            subscribing: false,
            reloads: HashMap::new(),
            reloading: HashMap::new(),
            reload_again: HashSet::new(),
            remote: HashMap::new(),
        }
    }
}

impl NotifyState {
    pub(crate) fn take_rx(&mut self) -> Option<Receiver<Event>> {
        self.rx.take()
    }

    /// Finite notify work in flight (guarded reloads). The subscribe
    /// attempt settles onto the notify queue, not the io channel, so it
    /// never parks an io drain.
    pub(crate) fn pending(&self) -> bool {
        !self.reloads.is_empty()
    }

    /// Push coverage over the scope, honestly reported (CoverageHonest):
    /// native/polling push, on-demand, or nothing.
    pub(crate) fn push_coverage(&self) -> bool {
        self.subscription.is_some()
            && matches!(
                self.coverage,
                Some(NotifyCoverage::Native) | Some(NotifyCoverage::Polling)
            )
    }
}

/// Observe and read one local file for a guarded reload: the same
/// checked observation/read family as open (WK09), riding the lease.
/// `None` means the resource vanished or stopped being a regular file —
/// an outcome, never a clobber.
pub(super) fn reload_run(
    worker: &strop_worker_client::Worker,
    path: &std::path::Path,
    cancel: &CancelToken,
) -> Outcome<Option<Opened>> {
    if cancel.is_cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    let location = ResourceLocation::local(path.to_path_buf());
    let observed = match worker.observe(cancel, vec![location.clone()]) {
        Ok(mut observations) => observations.pop().and_then(|located| located.value),
        Err(error) if error.is_cancellation() => {
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
    let payload = match worker.read(cancel, location, 0, None) {
        Ok(payload) => payload,
        Err(error) if error.is_cancellation() => {
            return Outcome::Cancelled(CancelReason::OwnerClosed)
        }
        Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
    };
    let rope = match ropey::Rope::from_reader(payload) {
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
    /// Subscribe the opened scope root (recursive; the worker adds the
    /// parent/name guards of the root itself) through the local worker
    /// lease. No-op under replay, while subscribed/subscribing, or when
    /// finishing. Never blocks input: the subscribe round trip rides a
    /// job, and the settle lands on the notify queue.
    pub(crate) fn start_notifications(&mut self) {
        if self.finishing
            || self.notify.subscribing
            || self.notify.subscription.is_some()
            || self.tape.is_replay()
        {
            return;
        }
        self.notify.subscribing = true;
        let worker = self.filesystem.worker().clone();
        worker.set_event_sink(self.notify.tx.clone());
        let scope = ResourceLocation::local(self.cwd.clone());
        let queue = Arc::clone(&self.notify.queue);
        let wake = self.app_tx.clone();
        let handle = worker::spawn(
            "fs-notify-subscribe",
            move |outcome| {
                queue.push_record(Record::Settled(outcome));
                if let Some(wake) = &wake {
                    let _ = wake.send(AppEvent::Notify);
                }
            },
            move |cancel| {
                let owned_trace = strop_trace::active_path().and_then(|path| {
                    std::fs::canonicalize(&scope.path).ok().and_then(|root| {
                        path.strip_prefix(root).ok().map(|relative| {
                            strop_workspace::addr::uri::path_bytes(relative).to_vec()
                        })
                    })
                });
                match worker.subscribe(&cancel, scope, true) {
                    Ok((subscription, coverage)) => Outcome::Success(SubscribedScope {
                        subscription,
                        coverage,
                        owned_trace,
                    }),
                    Err(error) if error.is_cancellation() => {
                        Outcome::Cancelled(CancelReason::OwnerClosed)
                    }
                    Err(error) => Outcome::failed(FailureKind::Unavailable, error.to_string()),
                }
            },
        );
        self.notify.subscribe_handle = Some(handle);
    }

    pub(crate) fn start_remote_notifications(&mut self) {
        // Hint timing is inherently racy, so the bit-exact full-content
        // tape lane (recording or replay) keeps remote freshness on
        // demand — hints are advisory and the feature is untraced by
        // design. Default sessions (no content capture) subscribe.
        if self.finishing || self.tape.observes() {
            return;
        }
        let roots: Vec<(ResourceLocation, strop_workspace::RemoteEndpoint)> = self
            .docs
            .iter()
            .filter_map(|(_, document)| {
                let directory = document.directory_metadata_ref()?;
                match &directory.location.filesystem {
                    Filesystem::Remote(endpoint) => {
                        Some((directory.location.clone(), endpoint.clone()))
                    }
                    _ => None,
                }
            })
            .collect();
        for (root, endpoint) in roots {
            if self
                .notify
                .remote
                .get(&root)
                .is_some_and(|scope| scope.subscribing || scope.subscription.is_some())
            {
                continue;
            }
            let Some(worker) = self.remote.workers.get(&endpoint) else {
                continue;
            };
            let tx = self.notify.tx.clone();
            let queue = Arc::clone(&self.notify.queue);
            let wake = self.app_tx.clone();
            let scope = self
                .notify
                .remote
                .entry(root.clone())
                .or_insert_with(|| RemoteScope {
                    endpoint,
                    subscription: None,
                    session: None,
                    subscribing: false,
                    subscribe_handle: None,
                });
            scope.subscribing = true;
            worker.worker().set_event_sink(tx);
            let scope_root = root.clone();
            let work_root = root.clone();
            let handle = worker::spawn(
                "remote-fs-notify-subscribe",
                move |outcome| {
                    queue.push_record(Record::RemoteSettled {
                        root: scope_root,
                        outcome,
                    });
                    if let Some(wake) = &wake {
                        let _ = wake.send(AppEvent::Notify);
                    }
                },
                move |cancel| match worker.subscribe(&cancel, work_root, true) {
                    Ok((subscription, coverage)) => Outcome::Success(SubscribedScope {
                        subscription,
                        coverage,
                        owned_trace: None,
                    }),
                    Err(error) if error.is_cancellation() => {
                        Outcome::Cancelled(CancelReason::OwnerClosed)
                    }
                    Err(error) => Outcome::failed(FailureKind::Unavailable, error.to_string()),
                },
            );
            if let Some(scope) = self.notify.remote.get_mut(&root) {
                scope.subscribe_handle = Some(handle);
            }
        }
    }

    /// Hints against one remote scope (WK07): remote Directory buffers
    /// reobserve through the same checked listing path (which routes
    /// through the endpoint's worker lease); a dirty remote document
    /// gains external-change state and is never clobbered; clean remote
    /// documents keep their follow-poll freshness until WK09's guarded
    /// remote reload.
    pub(super) fn apply_remote_hints(&mut self, root: &std::path::Path, hints: Vec<NotifyHint>) {
        use std::os::unix::ffi::OsStrExt;
        let mut directories = Vec::new();
        let mut dirty = Vec::new();
        for hint in hints {
            let relative = PathBuf::from(std::ffi::OsStr::from_bytes(&hint.path));
            if relative.as_os_str().is_empty() {
                // The scope root itself moved/was replaced: the whole
                // remote baseline is suspect.
                self.remote_notify_rescan(root);
                return;
            }
            let absolute = root.join(&relative);
            for (id, document) in self.docs.iter() {
                match &document.source {
                    DocumentSource::Directory(directory) => {
                        if !matches!(directory.location.filesystem, Filesystem::Remote(_)) {
                            continue;
                        }
                        let affects_listing = absolute == directory.location.path
                            || absolute.parent() == Some(directory.location.path.as_path());
                        if affects_listing && !directories.contains(&id) {
                            directories.push(id);
                        }
                    }
                    DocumentSource::Remote(source)
                        if source.file.path() == absolute
                            && document.buf.dirty
                            && !dirty.contains(&id) =>
                    {
                        dirty.push(id);
                    }
                    DocumentSource::Remote(_) => {}
                    _ => {}
                }
            }
        }
        for id in dirty {
            self.notify_mark_external(id);
        }
        for id in directories {
            let _ = self.start_directory_task(id, DirectoryTask::Reload, None);
        }
    }

    /// A conservative remote rescan obligation (overflow, queue bound,
    /// root replacement, lease loss): invalidate the remote baseline
    /// and reobserve before freshness is claimed again.
    pub(super) fn remote_notify_rescan(&mut self, root: &std::path::Path) {
        let mut directories = Vec::new();
        let mut dirty = Vec::new();
        for (id, document) in self.docs.iter() {
            match &document.source {
                DocumentSource::Directory(directory)
                    if matches!(directory.location.filesystem, Filesystem::Remote(_))
                        && directory.location.path.starts_with(root) =>
                {
                    directories.push(id);
                }
                DocumentSource::Remote(source)
                    if source.file.path().starts_with(root) && document.buf.dirty =>
                {
                    dirty.push(id);
                }
                DocumentSource::Remote(_) => {}
                _ => {}
            }
        }
        for id in dirty {
            self.notify_mark_external(id);
        }
        for id in directories {
            let _ = self.start_directory_task(id, DirectoryTask::Reload, None);
        }
    }

    /// Lease observation, per app event (cheap mutex reads, no I/O): a
    /// changed/absent session incarnation means the subscriptions died
    /// with it. Invalidate conservatively, surface, and reestablish
    /// coverage before freshness is claimed again (LOSS).
    /// Remote lease observation (WK07): a changed or dead incarnation
    /// kills its scopes' subscriptions; the scope baseline is
    /// invalidated, rescanned and reestablished before freshness is
    /// claimed again — identical to the local LOSS rule.
    pub(super) fn observe_remote_leases(&mut self) {
        let lost: Vec<ResourceLocation> = self
            .notify
            .remote
            .iter()
            .filter_map(|(root, scope)| {
                let session = scope.session?;
                let alive = self
                    .remote
                    .workers
                    .get(&scope.endpoint)
                    .and_then(|worker| worker.worker().session());
                (alive != Some(session)).then(|| root.clone())
            })
            .collect();
        let reestablish = !lost.is_empty();
        for root in lost {
            let path = root.path.clone();
            if let Some(scope) = self.notify.remote.get_mut(&root) {
                scope.subscription = None;
                scope.session = None;
            }
            self.remote_notify_rescan(&path);
            self.message =
                "remote filesystem notifications lost with the worker; reestablishing coverage"
                    .into();
        }
        if reestablish {
            self.start_remote_notifications();
        }
    }
}
