//! Subscription manager: generation-stamped subscriptions, each owning one
//! inotify instance, with bounded pending queues, kernel-cookie rename
//! pairing and honest loss reporting. Every published event is the wire's
//! own type ([`Event`]); inotify masks never leave this module.
//!
//! ## Coverage model
//!
//! - A directory scope holds a *tree* watch on the root (a bounded
//!   recursive walk when `recursive`) plus a *guard* watch on the parent
//!   directory filtered to the root's own name — replacement/rename of the
//!   scope root itself is observed, never guessed.
//! - A file scope holds only the guard watch: parent/name coverage sees
//!   creation, modification, deletion, rename and atomic replacement of
//!   the object. A scope that does not exist yet still gets guard coverage
//!   when its parent is watchable, so creation is observed.
//! - Symlinked subtrees are not recursed into (cycle safety); the symlink
//!   itself produces create/delete hints, so its target change reobserves,
//!   but changes *inside* a symlinked tree are out of coverage — stated
//!   honestly rather than walked into a cycle.
//!
//! ## Loss model
//!
//! `IN_Q_OVERFLOW`, a full pending queue, a truncated read buffer, a queue
//! read failure, an unpaired directory move, a self-moved or deleted root
//! or guard, watch-descriptor reuse after retirement and any registration
//! gap all latch the subscription's rescan obligation, published as
//! [`Event::NotifyOverflow`]. The staleness signal is never dropped;
//! pending hints superseded by an overflow are discarded *because* the
//! overflow strictly covers them. A queue read failure invalidates the
//! baseline the same way (overflow), and reinstalling the subscription
//! re-initializes the instance — a persistent fault self-heals through
//! resubscription instead of looping a typed error.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use strop_worker_protocol::{Event, NotifyCoverage, NotifyHint, NotifyKind, Subscription};
use strop_workspace::{Filesystem, ResourceLocation};

use super::sys::Inotify;
use super::{NotifyConfig, NotifyError};

/// Every object-level mask this adapter registers for. `IN_Q_OVERFLOW` and
/// `IN_IGNORED` are delivered by the kernel regardless.
const WATCH_MASK: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MODIFY
    | libc::IN_CLOSE_WRITE
    | libc::IN_ATTRIB
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

/// What a subscription admitted: its fresh identity and the honest
/// coverage (`Native` for the local namespace; partial coverage arrives as
/// an immediate overflow event, not a downgraded enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subscribed {
    pub subscription: Subscription,
    pub coverage: NotifyCoverage,
}

/// One live subscription. Owns its inotify instance: overflow is scoped to
/// exactly this baseline, watch descriptors are never shared, and teardown
/// is one `close(2)`.
#[derive(Debug)]
struct SubscriptionState {
    identity: Subscription,
    root: PathBuf,
    recursive: bool,
    inotify: Inotify,
    /// Tree watches: watch descriptor → directory it covers.
    tree: HashMap<i32, PathBuf>,
    /// Reverse index (directory → descriptor) for subtree re-key/teardown.
    dirs: HashMap<PathBuf, i32>,
    root_wd: Option<i32>,
    /// Guard watch on the parent, filtered to `guard_name`.
    guard_wd: Option<i32>,
    guard_name: OsString,
    /// Descriptors retired by `inotify_rm_watch`, awaiting `IN_IGNORED`;
    /// late events and descriptor reuse are checked against this set.
    retired: HashSet<i32>,
    /// Next per-subscription sequence; monotone so a reconcile boundary
    /// can never erase a newer invalidation.
    sequence: u64,
    boundary_pending: bool,
    pending: VecDeque<NotifyHint>,
    pending_keys: HashSet<(Vec<u8>, u8)>,
    /// The conservative rescan obligation: once latched, only the
    /// overflow event speaks for this baseline.
    rescan: bool,
}

/// The notify backend for one worker session.
#[derive(Debug)]
pub struct NotifyManager {
    config: NotifyConfig,
    subs: HashMap<u64, SubscriptionState>,
    /// Scope root → subscription id, so reinstalling a scope re-enters
    /// with the same id and a bumped generation (the wire's staleness
    /// contract: reused descriptors/inodes never resurrect old authority).
    roots: HashMap<PathBuf, u64>,
    next_id: u64,
}

impl NotifyManager {
    pub fn new(config: NotifyConfig) -> Self {
        Self {
            config,
            subs: HashMap::new(),
            roots: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.subs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }

    /// Install a subscription over one local scope. Reinstalling a live
    /// scope root retires the old registration promptly and re-enters with
    /// the same id and a bumped generation. Remote/container scopes are a
    /// typed refusal here; the relay is a later slice.
    pub fn subscribe(
        &mut self,
        scope: &ResourceLocation,
        recursive: bool,
    ) -> Result<Subscribed, NotifyError> {
        if scope.filesystem != Filesystem::Local {
            return Err(NotifyError::UnsupportedNamespace);
        }
        if !scope.path.is_absolute() {
            return Err(NotifyError::RelativeScope);
        }
        let root = scope.path.clone();
        let (id, generation) = match self.roots.get(&root) {
            Some(&id) => {
                // Drop closes the old instance: all its watches die here.
                let generation = self.subs.remove(&id).map(|s| s.identity.generation + 1);
                (id, generation.unwrap_or(1))
            }
            None => {
                if self.subs.len() >= self.config.max_subscriptions {
                    return Err(NotifyError::SubscriptionLimit(self.subs.len()));
                }
                let id = self.next_id;
                self.next_id += 1;
                (id, 1)
            }
        };
        let identity = Subscription { id, generation };
        let state = SubscriptionState::register(identity, &root, recursive, &self.config)?;
        self.subs.insert(id, state);
        self.roots.insert(root, id);
        Ok(Subscribed {
            subscription: identity,
            coverage: NotifyCoverage::Native,
        })
    }

    /// Retire one subscription by full identity. Stale generations and
    /// unknown ids are typed refusals; a live retire closes the instance
    /// and drops its pending queue immediately — no late event can be
    /// published for it afterwards.
    pub fn unsubscribe(&mut self, subscription: Subscription) -> Result<(), NotifyError> {
        let Some(state) = self.subs.get(&subscription.id) else {
            return Err(NotifyError::UnknownSubscription {
                id: subscription.id,
            });
        };
        if state.identity.generation != subscription.generation {
            return Err(NotifyError::StaleGeneration {
                id: subscription.id,
                current: state.identity.generation,
            });
        }
        let state = self.subs.remove(&subscription.id);
        if let Some(state) = state {
            self.roots.remove(&state.root);
        }
        Ok(())
    }

    /// Drain every live subscription: read its kernel queue, classify into
    /// hints and publish bounded wire events in per-subscription sequence
    /// order. Subscriptions drain in id order for determinism.
    pub fn drain(&mut self) -> Result<Vec<Event>, NotifyError> {
        let mut ids: Vec<u64> = self.subs.keys().copied().collect();
        ids.sort_unstable();
        let mut out = Vec::new();
        for id in ids {
            if let Some(state) = self.subs.get_mut(&id) {
                state.drain(&self.config, &mut out);
            }
        }
        Ok(out)
    }

    /// Block until any subscription's queue is readable or `timeout`
    /// elapses (the worker loop's wait primitive). `true` means at least
    /// one fd reports readable/error/hang-up; `false` is timeout or no
    /// live subscriptions.
    pub fn poll(&mut self, timeout: Option<Duration>) -> Result<bool, NotifyError> {
        let fds: Vec<i32> = self.subs.values().map(|s| s.inotify.fd()).collect();
        super::sys::poll(&fds, timeout).map_err(NotifyError::Poll)
    }
}

impl SubscriptionState {
    fn register(
        identity: Subscription,
        root: &Path,
        recursive: bool,
        config: &NotifyConfig,
    ) -> Result<Self, NotifyError> {
        let mut state = Self {
            identity,
            root: root.to_path_buf(),
            recursive,
            inotify: Inotify::init().map_err(NotifyError::Init)?,
            tree: HashMap::new(),
            dirs: HashMap::new(),
            root_wd: None,
            guard_wd: None,
            guard_name: OsString::new(),
            retired: HashSet::new(),
            sequence: 0,
            boundary_pending: true,
            pending: VecDeque::new(),
            pending_keys: HashSet::new(),
            rescan: false,
        };
        let mut covered = false;
        let mut fatal: Option<(PathBuf, std::io::Error)> = None;
        let is_dir = std::fs::metadata(root).map(|m| m.is_dir()).unwrap_or(false);
        if is_dir {
            match state.register_subtree(root, recursive, config) {
                Ok(complete) => {
                    covered = true;
                    state.rescan |= !complete;
                    state.root_wd = state.dirs.get(root).copied();
                }
                Err(source) => fatal = Some((root.to_path_buf(), source)),
            }
        }
        if let (Some(parent), Some(name)) = (root.parent(), root.file_name()) {
            match state.inotify.add_watch(parent, WATCH_MASK) {
                Ok(wd) => {
                    if state.retired.remove(&wd) {
                        state.rescan = true;
                    }
                    state.guard_wd = Some(wd);
                    state.guard_name = name.to_os_string();
                    covered = true;
                }
                Err(source) => {
                    if covered {
                        // Tree coverage stands but root replacement is now
                        // invisible: degraded coverage invalidates the
                        // baseline honestly.
                        state.rescan = true;
                    } else if fatal.is_none() {
                        fatal = Some((parent.to_path_buf(), source));
                    }
                }
            }
        }
        if !covered {
            let (path, source) = fatal.unwrap_or_else(|| {
                (
                    root.to_path_buf(),
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "scope does not exist and has no watchable parent",
                    ),
                )
            });
            return Err(NotifyError::Register { path, source });
        }
        Ok(state)
    }

    /// Watch `root` (fatal on failure) and, when `recursive`, every real
    /// subdirectory beneath it within the watch bound. Returns `false`
    /// when any subtree was excluded (unreadable, over the bound): the
    /// caller latches the rescan obligation.
    fn register_subtree(
        &mut self,
        root: &Path,
        recursive: bool,
        config: &NotifyConfig,
    ) -> std::io::Result<bool> {
        let mut complete = true;
        let mut first = true;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            if self.dirs.contains_key(&dir) {
                continue;
            }
            let fatal = first;
            first = false;
            // +1 reserves the guard watch inside the same bound.
            if self.dirs.len() + 1 >= config.max_watches_per_subscription {
                complete = false;
                break;
            }
            match self.inotify.add_watch(&dir, WATCH_MASK) {
                Ok(wd) => {
                    if self.retired.remove(&wd) {
                        self.rescan = true;
                    }
                    self.tree.insert(wd, dir.clone());
                    self.dirs.insert(dir.clone(), wd);
                }
                Err(e) if fatal => return Err(e),
                Err(_) => {
                    complete = false;
                    continue;
                }
            }
            if recursive {
                match std::fs::read_dir(&dir) {
                    Ok(entries) => {
                        for entry in entries {
                            match entry.and_then(|e| e.file_type().map(|t| (e.path(), t))) {
                                Ok((path, kind)) if kind.is_dir() => stack.push(path),
                                // Symlinks are never followed (cycles);
                                // unreadable entries are excluded coverage.
                                Ok(_) => {}
                                Err(_) => complete = false,
                            }
                        }
                    }
                    Err(_) => complete = false,
                }
            }
        }
        Ok(complete)
    }

    /// One drain cycle for this subscription.
    fn drain(&mut self, config: &NotifyConfig, out: &mut Vec<Event>) {
        let read = self.inotify.read_events(config.max_read_cycles);
        if read.truncated || read.error.is_some() {
            // Kernel-queue state is unknown; invalidate the baseline. A
            // persistent fault heals through reinstall (fresh instance).
            self.rescan = true;
        }
        self.process_batch(read.events, config);
        self.flush(config, out);
    }

    /// Classify one kernel batch. Two passes: cookie pairs are decided
    /// over the whole batch first, then events classify in order, with
    /// `IN_MOVE_SELF` deferred to last so in-batch re-keys land before
    /// self-move teardown checks. A pair split across batches (read-cycle
    /// bound) degrades to `Ambiguous` + `Created`, never a guessed pair.
    fn process_batch(&mut self, batch: Vec<super::sys::RawEvent>, config: &NotifyConfig) {
        let mut from_cookies: HashSet<u32> = HashSet::new();
        let mut pairs: HashSet<u32> = HashSet::new();
        for event in &batch {
            if event.name.is_empty() || !self.tree.contains_key(&event.wd) {
                continue;
            }
            if event.mask & libc::IN_MOVED_FROM != 0 {
                from_cookies.insert(event.cookie);
            }
            if event.mask & libc::IN_MOVED_TO != 0 && from_cookies.contains(&event.cookie) {
                pairs.insert(event.cookie);
            }
        }
        let mut rekeyed: HashSet<i32> = HashSet::new();
        let mut pending_pairs: HashMap<u32, PathBuf> = HashMap::new();
        let mut self_moved: Vec<super::sys::RawEvent> = Vec::new();
        for event in batch {
            self.classify(
                event,
                &pairs,
                &mut pending_pairs,
                &mut rekeyed,
                &mut self_moved,
                config,
            );
        }
        for event in self_moved {
            self.on_move_self(&event, &rekeyed, config);
        }
    }

    fn classify(
        &mut self,
        event: super::sys::RawEvent,
        pairs: &HashSet<u32>,
        pending_pairs: &mut HashMap<u32, PathBuf>,
        rekeyed: &mut HashSet<i32>,
        self_moved: &mut Vec<super::sys::RawEvent>,
        config: &NotifyConfig,
    ) {
        if event.mask & libc::IN_Q_OVERFLOW != 0 {
            self.rescan = true;
            return;
        }
        if event.mask & libc::IN_IGNORED != 0 {
            let was_retired = self.retired.remove(&event.wd);
            if was_retired && (self.tree.contains_key(&event.wd) || self.guard_wd == Some(event.wd))
            {
                // Descriptor reused before the retirement notice arrived;
                // the reuse already latched the rescan obligation.
                return;
            }
            if let Some(dir) = self.tree.remove(&event.wd) {
                self.dirs.remove(&dir);
            }
            if self.root_wd == Some(event.wd) {
                self.root_wd = None;
                if !was_retired {
                    self.push_hint(Vec::new(), NotifyKind::Removed, config);
                    self.rescan = true;
                }
            }
            if self.guard_wd == Some(event.wd) {
                self.guard_wd = None;
                if !was_retired {
                    self.push_hint(Vec::new(), NotifyKind::Ambiguous, config);
                    self.rescan = true;
                }
            }
            return;
        }
        if event.mask & libc::IN_MOVE_SELF != 0 {
            self_moved.push(event);
            return;
        }
        if self.guard_wd == Some(event.wd) {
            self.classify_guard(&event, config);
            return;
        }
        let Some(dir) = self.tree.get(&event.wd).cloned() else {
            return; // retired or never ours: late events die here
        };
        if event.name.is_empty() {
            if event.mask & libc::IN_DELETE_SELF != 0 {
                let rel = self.relative(&dir);
                self.push_hint(rel, NotifyKind::Removed, config);
                if self.root_wd == Some(event.wd) {
                    self.rescan = true;
                }
            } else if event.mask & (libc::IN_ATTRIB | libc::IN_MODIFY) != 0 {
                let rel = self.relative(&dir);
                self.push_hint(rel, NotifyKind::Modified, config);
            }
            return;
        }
        let full = dir.join(OsStr::from_bytes(&event.name));
        let is_dir = event.mask & libc::IN_ISDIR != 0;
        if event.mask & libc::IN_MOVED_FROM != 0 {
            if pairs.contains(&event.cookie) {
                pending_pairs.insert(event.cookie, full);
            } else {
                // Left the scope (or a split pair): reconcile by
                // observation, never a guessed destination.
                let rel = self.relative(&full);
                self.push_hint(rel, NotifyKind::Ambiguous, config);
                if is_dir {
                    self.teardown_subtree(&full);
                    self.rescan = true;
                }
            }
        } else if event.mask & libc::IN_MOVED_TO != 0 {
            if let Some(old) = pending_pairs.remove(&event.cookie) {
                let rel_old = self.relative(&old);
                let rel_new = self.relative(&full);
                self.push_hint(rel_old, NotifyKind::Renamed, config);
                self.push_hint(rel_new, NotifyKind::Renamed, config);
                if is_dir && self.recursive {
                    // Watches follow the inode; re-key bookkeeping, or
                    // register fresh where coverage had excluded it.
                    if self.rekey_subtree(&old, &full, rekeyed) == 0 {
                        self.register_dynamic(full, config);
                    }
                }
            } else {
                let rel = self.relative(&full);
                self.push_hint(rel, NotifyKind::Created, config);
                if is_dir && self.recursive {
                    self.register_dynamic(full, config);
                }
            }
        } else if event.mask & libc::IN_CREATE != 0 {
            let rel = self.relative(&full);
            self.push_hint(rel, NotifyKind::Created, config);
            if is_dir && self.recursive {
                self.register_dynamic(full, config);
            }
        } else if event.mask & libc::IN_DELETE != 0 {
            let rel = self.relative(&full);
            self.push_hint(rel, NotifyKind::Removed, config);
        } else if event.mask & (libc::IN_MODIFY | libc::IN_CLOSE_WRITE | libc::IN_ATTRIB) != 0 {
            let rel = self.relative(&full);
            self.push_hint(rel, NotifyKind::Modified, config);
        }
    }

    /// Guard-watch events: only the scope root's own name matters; the
    /// hint path is empty (the scope root itself). A root rename is always
    /// `Ambiguous` plus a rescan obligation — relocation is never tracked
    /// by guessing.
    fn classify_guard(&mut self, event: &super::sys::RawEvent, config: &NotifyConfig) {
        if event.name.is_empty() {
            // The parent itself was deleted; its move arrives as
            // IN_MOVE_SELF and is handled there.
            if event.mask & libc::IN_DELETE_SELF != 0 {
                self.push_hint(Vec::new(), NotifyKind::Ambiguous, config);
                self.rescan = true;
                self.retire_guard();
            }
            return;
        }
        if event.name != self.guard_name.as_bytes() {
            return;
        }
        if event.mask & libc::IN_MOVED_FROM != 0 {
            self.push_hint(Vec::new(), NotifyKind::Ambiguous, config);
            self.rescan = true;
        } else if event.mask & (libc::IN_MOVED_TO | libc::IN_CREATE) != 0 {
            self.push_hint(Vec::new(), NotifyKind::Created, config);
        } else if event.mask & libc::IN_DELETE != 0 {
            self.push_hint(Vec::new(), NotifyKind::Removed, config);
        } else if event.mask & (libc::IN_MODIFY | libc::IN_CLOSE_WRITE | libc::IN_ATTRIB) != 0 {
            self.push_hint(Vec::new(), NotifyKind::Modified, config);
        }
    }

    /// A watched object moved itself. Root and guard moves invalidate the
    /// baseline; subtree moves tear down the now-unverifiable watches
    /// (events from outside the scope must never be reported) and latch
    /// the rescan obligation — the small rm/add window is covered by it.
    fn on_move_self(
        &mut self,
        event: &super::sys::RawEvent,
        rekeyed: &HashSet<i32>,
        config: &NotifyConfig,
    ) {
        if rekeyed.contains(&event.wd) {
            return; // in-scope rename, already re-keyed this batch
        }
        if self.guard_wd == Some(event.wd) {
            self.push_hint(Vec::new(), NotifyKind::Ambiguous, config);
            self.rescan = true;
            self.retire_guard();
            return;
        }
        let Some(dir) = self.tree.get(&event.wd).cloned() else {
            return;
        };
        let rel = self.relative(&dir);
        self.push_hint(rel, NotifyKind::Ambiguous, config);
        self.rescan = true;
        if self.root_wd == Some(event.wd) {
            self.root_wd = None;
        }
        self.teardown_subtree(&dir);
    }

    /// Register a newly appeared directory's subtree; failure or partial
    /// coverage is an excluded-subtree outcome and latches the rescan
    /// obligation.
    fn register_dynamic(&mut self, dir: PathBuf, config: &NotifyConfig) {
        match self.register_subtree(&dir, true, config) {
            Ok(complete) => self.rescan |= !complete,
            Err(_) => self.rescan = true,
        }
    }

    /// Re-key subtree bookkeeping after a kernel-paired in-scope rename.
    /// No syscalls: the watches already follow the inode. Returns the
    /// number of watches re-keyed.
    fn rekey_subtree(&mut self, old: &Path, new: &Path, rekeyed: &mut HashSet<i32>) -> usize {
        let mut moves = Vec::new();
        for (dir, wd) in &self.dirs {
            if let Ok(suffix) = dir.strip_prefix(old) {
                moves.push((dir.clone(), new.join(suffix), *wd));
            }
        }
        let count = moves.len();
        for (old_dir, new_dir, wd) in moves {
            self.dirs.remove(&old_dir);
            self.dirs.insert(new_dir.clone(), wd);
            if let Some(recorded) = self.tree.get_mut(&wd) {
                *recorded = new_dir;
            }
            rekeyed.insert(wd);
        }
        count
    }

    /// Retire every tree watch at or below `dir` (it left the scope or
    /// moved to an unverifiable location). `EINVAL` means the kernel
    /// already retired the descriptor (`IN_IGNORED` in flight) — the
    /// expected teardown race, bounded by the watch cap.
    fn teardown_subtree(&mut self, dir: &Path) {
        let mut gone = Vec::new();
        for (covered, wd) in &self.dirs {
            if covered.starts_with(dir) {
                gone.push((covered.clone(), *wd));
            }
        }
        for (covered, wd) in gone {
            self.dirs.remove(&covered);
            self.tree.remove(&wd);
            if let Err(e) = self.inotify.remove_watch(wd) {
                debug_assert!(
                    e.raw_os_error() == Some(libc::EINVAL),
                    "rm_watch on a live descriptor failed: {e}"
                );
            }
            self.retired.insert(wd);
        }
    }

    fn retire_guard(&mut self) {
        if let Some(wd) = self.guard_wd.take() {
            if let Err(e) = self.inotify.remove_watch(wd) {
                debug_assert!(
                    e.raw_os_error() == Some(libc::EINVAL),
                    "rm_watch on the guard descriptor failed: {e}"
                );
            }
            self.retired.insert(wd);
        }
    }

    /// Path of `full` relative to the scope root, as native bytes. The
    /// scope root itself is the empty path. A path that no longer strips
    /// is bookkeeping confusion: latch the rescan obligation and say
    /// nothing rather than publish a wrong path.
    fn relative(&mut self, full: &Path) -> Vec<u8> {
        match full.strip_prefix(&self.root) {
            Ok(rel) => rel.as_os_str().as_bytes().to_vec(),
            Err(_) => {
                self.rescan = true;
                Vec::new()
            }
        }
    }

    /// Enqueue one hint with dedup. A full queue latches the rescan
    /// obligation and drops only *incoming* hints: what was already
    /// observed still publishes ahead of the overflow event, which
    /// strictly covers everything beyond it. The staleness signal itself
    /// is never lost.
    fn push_hint(&mut self, path: Vec<u8>, kind: NotifyKind, config: &NotifyConfig) {
        if self.rescan {
            return;
        }
        if self.pending.len() >= config.max_pending_hints {
            self.rescan = true;
            return;
        }
        // `NotifyKind` is wire data without `Hash`; its tag is stable.
        if self.pending_keys.insert((path.clone(), kind as u8)) {
            self.pending.push_back(NotifyHint { path, kind });
        }
    }

    /// Publish this subscription's events in order: the reconcile boundary
    /// first (sequence 0 after every install), then bounded hint batches,
    /// then the overflow event when the rescan obligation latched —
    /// observed hints precede the invalidation they predated, each with
    /// the next monotone sequence.
    fn flush(&mut self, config: &NotifyConfig, out: &mut Vec<Event>) {
        let subscription = self.identity;
        if self.boundary_pending {
            self.boundary_pending = false;
            out.push(Event::ReconcileBoundary {
                subscription,
                sequence: self.sequence,
            });
            self.sequence += 1;
        }
        while !self.pending.is_empty() {
            let take = config.max_hints_per_event.min(self.pending.len());
            let mut hints = Vec::with_capacity(take);
            for _ in 0..take {
                if let Some(hint) = self.pending.pop_front() {
                    hints.push(hint);
                }
            }
            out.push(Event::Notify {
                subscription,
                sequence: self.sequence,
                hints,
            });
            self.sequence += 1;
        }
        self.pending_keys.clear();
        if self.rescan {
            self.rescan = false;
            out.push(Event::NotifyOverflow {
                subscription,
                sequence: self.sequence,
            });
            self.sequence += 1;
        }
    }
}
