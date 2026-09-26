//! Filesystem notifications, editor half (0058 §2 amendments 2026-09-14/16;
//! specs/Notify.tla correspondence). The worker beside the files owns
//! native watching; the editor owns application of observations to
//! documents, Directory buffers, Git signs and the search baseline.
//!
//! - **Events are hints.** A hint triggers observation/reconciliation —
//!   never an authoritative edit, save receipt or relocation. An
//!   `Ambiguous` rename invalidates; only an observation-confirmed
//!   outcome may change a binding.
//! - **Identity.** A subscription's authority is its (session, scope,
//!   generation) triple; events stamped by a superseded identity are
//!   dropped, never acted on (StaleIdentityNeverActs).
//! - **Bounded flow.** Worker events land in [`NotifyQueue`], a bounded
//!   coalescing buffer; the event-loop wake is a pure hint (AR06-legal:
//!   the state lives here). Queue/hint-set bounds and native overflows
//!   coalesce into a conservative rescan obligation — the only sign of
//!   staleness is never silently dropped (CoalesceNeverLosesStaleness).
//! - **Publication.** A clean buffer reloads only through a guarded
//!   job: document/binding/revision re-checked at completion, published
//!   only when the fresh observation differs from the buffer's baseline
//!   (a confirmed own-save receipt is never erased by a late hint). A
//!   dirty buffer is preserved and gains external-change state
//!   (DirtyNeverClobbered); a stale in-flight reload never clears newer
//!   edits (StaleReloadNeverClears). Loss/overflow/exclusions invalidate
//!   the baseline and reobserve before freshness is claimed again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use parking_lot::Mutex;

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, CancelReason, CancelToken, Completion, FailureKind, Outcome};
use strop_worker_protocol::message::NotifyCoverage;
use strop_worker_protocol::{Event, NotifyHint, Session, Subscription};
use strop_workspace::{Filesystem, ResourceLocation};

use super::directory::DirectoryTask;
use super::io::{IoEvent, Opened};
use super::DocumentSource;
use super::Editor;

/// Queued records awaiting a drain turn (0056 AR06 semantic bound for
/// this lane). A full lane latches the conservative rescan obligation.
const MAX_RECORDS: usize = 64;
/// Hint paths coalesced per record before promotion to overflow.
const MAX_HINTS: usize = 512;

/// One semantic notify record. Subscription establishment rides the same
/// queue so application order never depends on a cross-channel race.
pub(crate) enum Record {
    /// The subscribe job settled (typed outcome, never a guessed state).
    Settled(Outcome<SubscribedScope>),
    /// Advisory freshness hints for one subscription generation.
    Hints {
        subscription: Subscription,
        hints: Vec<NotifyHint>,
    },
    /// Native overflow/partial coverage: rescan obligation.
    Overflow { subscription: Subscription },
    /// The initial scan completed (the boundary is positional: FIFO
    /// order against the subscription's own hints).
    Boundary { subscription: Subscription },
    /// A remote scope's subscribe job settled (WK07): the root it
    /// belongs to travels with the outcome — subscription identity
    /// alone never decides placement.
    RemoteSettled {
        root: ResourceLocation,
        outcome: Outcome<SubscribedScope>,
    },
}

/// The settled subscription outcome (in-memory; never traced).
pub(crate) struct SubscribedScope {
    pub subscription: Subscription,
    pub coverage: NotifyCoverage,
    /// Relative native path of this process's own trace output, if it
    /// lives below this watched root. Never suppress unrelated files.
    pub owned_trace: Option<Vec<u8>>,
}

#[derive(Default)]
struct QueueState {
    records: VecDeque<Record>,
    /// The queue itself overflowed: conservative rescan obligation.
    rescan: bool,
    owned_trace: Option<(Subscription, Vec<u8>)>,
}

/// The bounded landing zone between the worker's reader thread (via the
/// forwarding sink) and the event loop. Producers record state here, so
/// the `AppEvent::Notify` wake hint may legally coalesce.
pub(crate) struct NotifyQueue {
    state: Mutex<QueueState>,
}

impl NotifyQueue {
    /// Ignore only this capture's own writes: tracing a notification
    /// would otherwise write another notification, forming an
    /// unbounded event/trace loop. Other hints remain untouched.
    pub(crate) fn set_owned_trace(&self, owned: Option<(Subscription, Vec<u8>)>) {
        self.state.lock().owned_trace = owned;
    }

    /// True when a meaningful record was queued and needs an app wake.
    pub(crate) fn push_event(&self, event: Event) -> bool {
        match event {
            Event::Notify {
                subscription,
                hints,
                ..
            } => self.push_hints(subscription, hints),
            Event::NotifyOverflow { subscription, .. } => {
                self.push_record(Record::Overflow { subscription });
                true
            }
            Event::ReconcileBoundary { subscription, .. } => {
                self.push_record(Record::Boundary { subscription });
                true
            }
            Event::ExecExit { .. } | Event::ExecInput { .. } => false,
        }
    }

    pub(crate) fn push_hints(
        &self,
        subscription: Subscription,
        mut hints: Vec<NotifyHint>,
    ) -> bool {
        let mut state = self.state.lock();
        if strop_trace::enabled() {
            if let Some((owner, path)) = &state.owned_trace {
                if *owner == subscription {
                    hints.retain(|hint| hint.path.as_slice() != path.as_slice());
                }
            }
        }
        if hints.is_empty() {
            return false;
        }
        if let Some(Record::Hints {
            subscription: tail_subscription,
            hints: tail,
        }) = state.records.back_mut()
        {
            if *tail_subscription == subscription && tail.len() + hints.len() <= MAX_HINTS {
                tail.append(&mut hints);
                return true;
            }
        }
        if hints.len() > MAX_HINTS {
            state.records.push_back(Record::Overflow { subscription });
            return true;
        }
        if state.records.len() >= MAX_RECORDS {
            state.rescan = true;
            return true;
        }
        state.records.push_back(Record::Hints {
            subscription,
            hints,
        });
        true
    }

    pub(crate) fn push_record(&self, record: Record) {
        let mut state = self.state.lock();
        if state.records.len() >= MAX_RECORDS {
            state.rescan = true;
            return;
        }
        state.records.push_back(record);
    }

    /// Requeue deferred records at the front (a pending subscribe must
    /// see the hints that raced it, in order).
    fn requeue_front(&self, records: Vec<Record>) {
        let mut state = self.state.lock();
        let room = MAX_RECORDS.saturating_sub(state.records.len());
        if records.len() > room {
            state.rescan = true;
        }
        for record in records.into_iter().take(room).rev() {
            state.records.push_front(record);
        }
    }

    /// Drain everything recorded so far plus the rescan latch.
    pub(crate) fn drain(&self) -> (Vec<Record>, bool) {
        let mut state = self.state.lock();
        (
            state.records.drain(..).collect(),
            std::mem::take(&mut state.rescan),
        )
    }
}

/// A guarded clean-buffer reload's identity (0058 §2 publication rule).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReloadKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
}

/// The editor's notification state: subscription identity, coverage,
/// the in-flight guarded reloads and the honest coverage note.
pub(crate) struct NotifyState {
    pub(crate) queue: Arc<NotifyQueue>,
    tx: Sender<Event>,
    pub(crate) rx: Option<Receiver<Event>>,
    /// The live subscription identity; events stamped otherwise are dead.
    pub(crate) subscription: Option<Subscription>,
    /// The worker session the subscription belongs to.
    session: Option<Session>,
    coverage: Option<NotifyCoverage>,
    subscribing: bool,
    /// The subscribe job's cancel handle; retained until settle
    /// (dropping it cancels the job).
    subscribe_handle: Option<worker::CancelHandle>,
    reloads: HashMap<strop_core::worker::WorkerId, ReloadKey>,
    reloading: HashMap<DocumentId, strop_core::worker::WorkerId>,
    /// Hints arrived while a reload was in flight: one re-observation
    /// owed at completion so a raced write is never stranded.
    reload_again: HashSet<DocumentId>,
    /// Remote workspace scopes (WK07): one subscription per opened
    /// remote directory root on a worker-admitted endpoint.
    remote: HashMap<ResourceLocation, RemoteScope>,
}

/// One remote workspace scope's subscription state. The subscription's
/// authority is its (session, scope, generation) identity exactly as
/// for the local scope; a dead worker incarnation's events are dropped.
pub(crate) struct RemoteScope {
    endpoint: strop_workspace::RemoteEndpoint,
    subscription: Option<Subscription>,
    session: Option<Session>,
    subscribing: bool,
    /// The subscribe job's cancel handle, retained until settle.
    subscribe_handle: Option<worker::CancelHandle>,
}

impl Default for NotifyState {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            queue: Arc::new(NotifyQueue {
                state: Mutex::new(QueueState::default()),
            }),
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
fn reload_run(
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
        .map(super::io::open::filetime_to_systemtime);
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Outcome::Success(Some(Opened {
        document: super::Document::new(strop_core::Buffer::from_read(
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
                    let _ = wake.send(super::events::AppEvent::Notify);
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
    /// Drain the notify queue and apply what it recorded. Settled
    /// records apply first (subscribe completion may race hints across
    /// the job/forwarder threads); hints stamped by a superseded
    /// identity are dropped, never acted on.
    pub(crate) fn handle_notify(&mut self) {
        let (records, queue_rescan) = self.notify.queue.drain();
        if records.is_empty() && !queue_rescan {
            return;
        }
        let mut deferred: Vec<Record> = Vec::new();
        let mut rescan = queue_rescan;
        let mut hints: Vec<NotifyHint> = Vec::new();
        // Lifecycle applies first: a subscribe settle may race hints
        // across the job/forwarder threads, and the last settle wins.
        let (settles, rest): (Vec<_>, Vec<_>) = records.into_iter().partition(|record| {
            matches!(record, Record::Settled(_) | Record::RemoteSettled { .. })
        });
        for record in settles {
            match record {
                Record::Settled(outcome) => self.notify_settled(outcome),
                Record::RemoteSettled { root, outcome } => {
                    self.remote_notify_settled(root, outcome)
                }
                _ => {}
            }
        }
        // Pass 2: identity-checked application. A record's subscription
        // names its owning scope — the local one or one remote root —
        // and events stamped by a superseded identity never act.
        let mut remote_hints: Vec<(ResourceLocation, Vec<NotifyHint>)> = Vec::new();
        let mut remote_rescans: Vec<ResourceLocation> = Vec::new();
        for record in rest {
            let record_subscription = match &record {
                Record::Hints { subscription, .. } | Record::Overflow { subscription } => {
                    *subscription
                }
                Record::Boundary { subscription, .. } => *subscription,
                Record::Settled(_) | Record::RemoteSettled { .. } => continue,
            };
            if Some(record_subscription) == self.notify.subscription {
                match record {
                    Record::Hints { hints: batch, .. } => hints.extend(batch),
                    Record::Overflow { .. } => rescan = true,
                    // The reconcile boundary's ordering guarantee lives in
                    // the consumers' snapshot semantics (picker cache) and
                    // the reload observation guards; the mark itself needs
                    // no editor-side state.
                    Record::Boundary { .. } => {}
                    Record::Settled(_) | Record::RemoteSettled { .. } => {}
                }
                continue;
            }
            let remote_root = self.notify.remote.iter().find_map(|(root, scope)| {
                (scope.subscription == Some(record_subscription)).then(|| root.clone())
            });
            if let Some(root) = remote_root {
                match record {
                    Record::Hints { hints: batch, .. } => {
                        match remote_hints.iter_mut().find(|(known, _)| *known == root) {
                            Some((_, batch_hints)) => batch_hints.extend(batch),
                            None => remote_hints.push((root, batch)),
                        }
                    }
                    Record::Overflow { .. } if !remote_rescans.contains(&root) => {
                        remote_rescans.push(root);
                    }
                    Record::Overflow { .. } => {}
                    _ => {}
                }
                continue;
            }
            if self.notify.subscription.is_none() && self.notify.subscribing {
                // The local subscribe settle has not landed yet; defer so
                // the raced hints apply in order once identity exists.
                deferred.push(record);
            }
            // Otherwise: a dead generation's late events never act.
        }
        for root in remote_rescans {
            self.remote_notify_rescan(&root.path.clone());
        }
        for (root, batch) in remote_hints {
            self.apply_remote_hints(&root.path.clone(), batch);
        }
        if !deferred.is_empty() {
            self.notify.queue.requeue_front(deferred);
        }
        if rescan {
            self.notify_rescan();
            return;
        }
        if !hints.is_empty() {
            self.apply_notify_hints(hints);
        }
    }

    /// The subscribe job settled: adopt the identity, surface coverage
    /// honestly, and arm the search baseline's push coverage.
    fn notify_settled(&mut self, outcome: Outcome<SubscribedScope>) {
        self.notify.subscribing = false;
        self.notify.subscribe_handle = None;
        match outcome {
            Outcome::Success(settled) => {
                self.notify.subscription = Some(settled.subscription);
                self.notify
                    .queue
                    .set_owned_trace(settled.owned_trace.map(|path| (settled.subscription, path)));
                self.notify.session = self.filesystem.worker().session();
                self.notify.coverage = Some(settled.coverage);
                if matches!(
                    settled.coverage,
                    NotifyCoverage::OnDemand | NotifyCoverage::Unsupported
                ) {
                    // CoverageHonest: a visible-but-quiet note, and the
                    // search baseline keeps per-search scanning.
                    self.message = "filesystem notifications unavailable in this namespace; freshness is on demand".into();
                }
            }
            Outcome::Failed { failure, .. } => {
                self.message = format!(
                    "filesystem notifications refused: {}; freshness is on demand",
                    failure.message
                );
            }
            Outcome::Cancelled(_) => {}
        }
        if let Some(source) = self.picker_source.as_ref() {
            source.set_watching(&self.cwd, self.notify.push_coverage());
        }
    }

    /// The remote subscribe job settled: adopt the scope's identity,
    /// surface coverage honestly, never guess state.
    fn remote_notify_settled(&mut self, root: ResourceLocation, outcome: Outcome<SubscribedScope>) {
        let endpoint = self
            .notify
            .remote
            .get(&root)
            .map(|scope| scope.endpoint.clone());
        let Some(endpoint) = endpoint else {
            return;
        };
        match outcome {
            Outcome::Success(settled) => {
                let session = self
                    .remote
                    .workers
                    .get(&endpoint)
                    .and_then(|worker| worker.worker().session());
                if let Some(scope) = self.notify.remote.get_mut(&root) {
                    scope.subscribing = false;
                    scope.subscribe_handle = None;
                    scope.subscription = Some(settled.subscription);
                    scope.session = session;
                }
                if matches!(
                    settled.coverage,
                    NotifyCoverage::OnDemand | NotifyCoverage::Unsupported
                ) {
                    self.message = format!(
                        "filesystem notifications for {} are on demand in this namespace",
                        root.label()
                    );
                }
            }
            Outcome::Failed { failure, .. } => {
                self.notify.remote.remove(&root);
                self.message = format!(
                    "filesystem notifications refused for {}: {}; freshness is on demand",
                    root.label(),
                    failure.message
                );
            }
            Outcome::Cancelled(_) => {
                if let Some(scope) = self.notify.remote.get_mut(&root) {
                    scope.subscribing = false;
                    scope.subscribe_handle = None;
                }
            }
        }
    }

    /// Subscribe every opened remote workspace root whose endpoint
    /// admits a worker (WK07). Browsing never deploys: only an existing
    /// admitted lease subscribes, and the round trip rides a job — the
    /// settle lands on the notify queue like the local scope's.
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
                        let _ = wake.send(super::events::AppEvent::Notify);
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
    fn apply_remote_hints(&mut self, root: &std::path::Path, hints: Vec<NotifyHint>) {
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
    fn remote_notify_rescan(&mut self, root: &std::path::Path) {
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
    fn observe_remote_leases(&mut self) {
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

    pub(crate) fn notify_observe_lease(&mut self) {
        self.observe_remote_leases();
        let Some(session) = self.notify.session else {
            return;
        };
        if self.filesystem.worker().session() == Some(session) {
            return;
        }
        self.notify.subscription = None;
        self.notify.queue.set_owned_trace(None);
        self.notify.session = None;
        self.notify.coverage = None;
        if let Some(source) = self.picker_source.as_ref() {
            source.set_watching(&self.cwd, false);
        }
        self.notify_rescan();
        self.message =
            "filesystem notifications lost with the worker; reestablishing coverage".into();
        self.start_notifications();
    }

    /// Hints against open state: dirty documents gain external-change
    /// state (never clobbered), clean documents reload through the
    /// guarded job, Directory buffers reobserve, Git signs refresh
    /// lazily and the search baseline invalidates the hinted subtrees.
    fn apply_notify_hints(&mut self, hints: Vec<NotifyHint>) {
        use std::os::unix::ffi::OsStrExt;
        let mut picker_paths: Vec<PathBuf> = Vec::new();
        let mut reload: Vec<DocumentId> = Vec::new();
        let mut dirty: Vec<DocumentId> = Vec::new();
        let mut directories: Vec<DocumentId> = Vec::new();
        let mut git_touched = false;
        for hint in hints {
            let relative = PathBuf::from(std::ffi::OsStr::from_bytes(&hint.path));
            if relative.as_os_str().is_empty() {
                // The scope root itself moved/was replaced (parent/name
                // guard): the whole baseline is suspect.
                self.notify_rescan();
                return;
            }
            picker_paths.push(relative.clone());
            let absolute = self.cwd.join(&relative);
            if relative
                .components()
                .any(|component| component.as_os_str() == ".git")
            {
                git_touched = true;
            }
            let canonical = std::fs::canonicalize(&absolute).unwrap_or_else(|_| absolute.clone());
            for (id, document) in self.docs.iter() {
                match &document.source {
                    DocumentSource::File => {
                        let bound = document.buf.path.as_ref() == Some(&absolute)
                            || document.buf.file_identity() == Some(absolute.as_path())
                            || document.buf.file_identity() == Some(canonical.as_path());
                        if !bound {
                            continue;
                        }
                        if Some(id) == Some(self.current()) {
                            git_touched = true;
                        }
                        if document.buf.dirty {
                            if !dirty.contains(&id) {
                                dirty.push(id);
                            }
                        } else if !reload.contains(&id) {
                            reload.push(id);
                        }
                    }
                    DocumentSource::Directory(directory) => {
                        if !matches!(directory.location.filesystem, Filesystem::Local) {
                            continue;
                        }
                        let affects_listing = absolute == directory.location.path
                            || absolute.parent() == Some(directory.location.path.as_path());
                        if affects_listing && !directories.contains(&id) {
                            directories.push(id);
                        }
                    }
                    _ => {}
                }
            }
        }
        for id in dirty {
            self.notify_mark_external(id);
        }
        for id in reload {
            self.request_notify_reload(id);
        }
        for id in directories {
            let _ = self.start_directory_task(id, DirectoryTask::Reload, None);
        }
        if git_touched {
            self.cancel_hunk_owner();
        }
        if let Some(source) = self.picker_source.as_ref() {
            source.invalidate(&self.cwd, &picker_paths, false);
        }
    }

    /// A conservative rescan obligation (overflow, queue bound, root
    /// replacement, lease loss): invalidate the whole affected baseline
    /// and reobserve before freshness is claimed again.
    fn notify_rescan(&mut self) {
        if let Some(source) = self.picker_source.as_ref() {
            source.invalidate(&self.cwd, &[], true);
        }
        self.cancel_hunk_owner();
        let documents: Vec<DocumentId> = self.docs.iter().map(|(id, _)| id).collect();
        for id in documents {
            let source_matches = {
                let document = self.doc(id);
                match &document.source {
                    DocumentSource::File => document
                        .buf
                        .path
                        .as_ref()
                        .is_some_and(|path| path.starts_with(&self.cwd)),
                    DocumentSource::Directory(directory) => {
                        matches!(directory.location.filesystem, Filesystem::Local)
                            && directory.location.path.starts_with(&self.cwd)
                    }
                    _ => false,
                }
            };
            if !source_matches {
                continue;
            }
            if matches!(self.doc(id).source, DocumentSource::Directory(_)) {
                let _ = self.start_directory_task(id, DirectoryTask::Reload, None);
            } else if self.doc(id).buf.dirty {
                self.notify_mark_external(id);
            } else {
                self.request_notify_reload(id);
            }
        }
    }

    /// A dirty (or vanished-from-under-clean) document gains
    /// external-change state; the buffer and its edits are preserved.
    fn notify_mark_external(&mut self, document: DocumentId) {
        let Some(doc) = self.docs.get_mut(document) else {
            return;
        };
        if doc.external_change {
            return;
        }
        doc.external_change = true;
        let label = doc.label(&self.cwd);
        self.message =
            format!("{label}: changed on disk; the buffer keeps your edits (:w! forces)");
    }

    /// Spawn the guarded reload job for one clean local document.
    /// Re-hints during flight record one owed re-observation.
    fn request_notify_reload(&mut self, document: DocumentId) {
        if self.finishing {
            return;
        }
        if self.notify.reloading.contains_key(&document) {
            self.notify.reload_again.insert(document);
            return;
        }
        let Some(document_ref) = self.docs.get(document) else {
            return;
        };
        if document_ref.buf.dirty || !matches!(document_ref.source, DocumentSource::File) {
            return;
        }
        let Some(path) = document_ref.buf.path.clone().or_else(|| {
            document_ref
                .buf
                .file_identity()
                .map(std::path::Path::to_path_buf)
        }) else {
            return;
        };
        let key = ReloadKey {
            document,
            revision: document_ref.buf.revision(),
            path,
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = worker::Ticket {
            request,
            key: key.clone(),
        };
        let path = key.path.clone();
        self.notify.reloads.insert(request, key);
        self.notify.reloading.insert(document, request);
        let worker = self.filesystem.worker().clone();
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "fs-notify-reload",
            move |outcome| {
                let _ = tx.send(IoEvent::NotifyReload(Box::new(Completion {
                    ticket,
                    outcome,
                })));
            },
            move |cancel| reload_run(&worker, &path, &cancel),
        );
        self.worker_handles.insert(request, handle);
    }

    /// Guarded publication (PUBLICATION): only a reload whose document,
    /// binding and revision still match, whose buffer is still clean,
    /// and whose fresh observation differs from the buffer's baseline
    /// may publish. A vanished file marks external change; a stale
    /// in-flight reload never clears newer edits.
    pub(crate) fn notify_reload_done(&mut self, completion: Completion<ReloadKey, Option<Opened>>) {
        let request = completion.ticket.request;
        if self.notify.reloads.get(&request) != Some(&completion.ticket.key) {
            return;
        }
        self.notify.reloads.remove(&request);
        self.worker_handles.remove(&request);
        let key = completion.ticket.key;
        self.notify.reloading.remove(&key.document);
        let reobserve = self.notify.reload_again.remove(&key.document);
        let Some(document) = self.docs.get(key.document) else {
            return;
        };
        let still_bound = matches!(document.source, DocumentSource::File)
            && (document.buf.path.as_ref() == Some(&key.path)
                || document.buf.file_identity() == Some(key.path.as_path()));
        if !still_bound {
            return; // the binding moved on (save-as, operation receipt)
        }
        if document.buf.revision() != key.revision || document.buf.dirty {
            // Edits landed while the job was in flight: the snapshot is
            // stale and never publishes. A dirty buffer takes
            // external-change state instead.
            if document.buf.dirty {
                self.notify_mark_external(key.document);
            }
            return;
        }
        match completion.outcome {
            Outcome::Success(Some(opened)) => {
                let observed_stamp = opened.document.buf.disk_stamp();
                if observed_stamp == document.buf.disk_stamp() {
                    // The baseline already covers this observation — an
                    // own-save receipt or a duplicate hint. Nothing to do.
                } else {
                    let binding = strop_core::Buffer::from_read(
                        opened
                            .document
                            .buf
                            .path
                            .clone()
                            .unwrap_or_else(|| key.path.clone()),
                        ropey::Rope::new(),
                        observed_stamp,
                        opened
                            .document
                            .buf
                            .file_identity()
                            .map(std::path::Path::to_path_buf)
                            .unwrap_or_else(|| key.path.clone()),
                        true,
                    );
                    let label = self.doc(key.document).label(&self.cwd);
                    let readonly = self.doc(key.document).buf.readonly;
                    let mut replacement = opened.document;
                    replacement.buf.readonly = readonly;
                    match self.publish_source_snapshot(key.document, replacement, false) {
                        Ok(()) => {
                            {
                                let mut doc = self.doc_mut(key.document);
                                doc.buf.adopt_file_binding(&binding);
                                doc.external_change = false;
                            }
                            self.message = format!("{label}: reloaded — changed on disk");
                        }
                        Err(error) => {
                            self.message = format!("{label}: reload failed: {error}");
                        }
                    }
                }
            }
            Outcome::Success(None) => {
                // Vanished or no longer a regular file: preserve the
                // buffer, surface the state, never clobber.
                self.notify_mark_external(key.document);
            }
            Outcome::Failed { .. } | Outcome::Cancelled(_) => {}
        }
        if reobserve
            && self
                .docs
                .get(key.document)
                .is_some_and(|document| !document.buf.dirty)
        {
            self.request_notify_reload(key.document);
        }
    }
}

#[cfg(test)]
#[path = "notify/tests.rs"]
mod tests;
