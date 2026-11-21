//! The local, in-memory inventory. Only overlapping requests with the same
//! folder and selection policy may share enumeration. A later request walks
//! again: neither a watcher nor yesterday's list proves membership today.
//! Retained syntax shards are candidates for byte verification, not answers.

mod search;
pub(super) use search::SearchScan;
mod walk;
pub(super) use walk::inventory;

use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use strop_core::worker::CancelToken;
use strop_kernel::admission::{self, Refresh, Resource, Scan};

use super::catalog::{CatalogStats, DeclaredBoundary, ProjectCatalog};
use super::selection::{self, SelectionPolicy};
use super::symbols::{ScanStats, SymbolIndex};
use crate::query::{FileSelectionPlan, SearchQuery};

const INVALIDATION_LIMIT: usize = 512;
const SCOPE_LIMIT: usize = 4;
// Four retained scopes together hold at most 64 MiB of disk shard payloads.
const SHARD_BYTES_PER_SCOPE: usize = 64 * 1024 * 1024 / SCOPE_LIMIT;
// The demand and speculative lanes can each have one incompatible walk.
const FLIGHTS: usize = 2;

/// One consumer's complete selection and producer requirement. A refresh
/// always uses the compiled plan belonging to this captured query.
pub(super) struct ScanRequest<'a> {
    pub root: &'a Path,
    pub query: &'a Arc<SearchQuery>,
    pub plan: &'a FileSelectionPlan,
    pub policy: SelectionPolicy,
    pub need_symbols: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BaselineStats {
    pub symbols: ScanStats,
    pub catalog: CatalogStats,
    pub full: bool,
}

#[derive(Clone)]
pub(super) struct Inventory {
    pub paths: Arc<Vec<PathBuf>>,
    pub catalog: Arc<ProjectCatalog>,
    pub symbols: Option<Arc<SymbolIndex>>,
    pub epoch: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
pub(super) enum InventoryError {
    Cancelled,
    Limit,
    Walk(String),
    Busy,
}

impl From<selection::CollectError> for InventoryError {
    fn from(error: selection::CollectError) -> Self {
        match error {
            selection::CollectError::Limit => Self::Limit,
            selection::CollectError::Walk(message) => Self::Walk(message),
        }
    }
}

impl InventoryError {
    pub(super) fn outcome(self, surface: &str) -> strop_core::worker::Outcome<()> {
        use strop_core::worker::{CancelReason, FailureKind, Outcome};
        match self {
            Self::Cancelled => Outcome::Cancelled(CancelReason::Superseded),
            Self::Walk(message) => Outcome::failed(FailureKind::Io, message),
            Self::Limit => Outcome::failed(
                FailureKind::Unavailable,
                format!("{surface} selection exceeds 100000 paths or 16 MiB; narrow the scope"),
            ),
            Self::Busy => Outcome::failed(
                FailureKind::Unavailable,
                "local inventory lanes are occupied",
            ),
        }
    }
}

/// Opaque host ticket: a declaration load issued for a previous incarnation
/// cannot rebind the same native root after `:cd` or watch loss.
#[derive(Clone)]
pub struct InventoryRoot(Arc<PathBuf>, Arc<()>);
type Delivery = Result<Inventory, InventoryError>;

// These are native allocation identities, not path hashes. A live ticket
// keeps both allocations alive, including across eviction and same-name reuse.
fn identity<T>(value: &Arc<T>) -> u64 {
    Arc::as_ptr(value) as usize as u64
}

struct Flight {
    root: Arc<PathBuf>,
    folder: Arc<PathBuf>,
    current: Arc<AtomicBool>,
    query: Arc<SearchQuery>,
    policy: SelectionPolicy,
    need_symbols: bool,
    leader: CancelToken,
    follower: Mutex<Option<CancelToken>>,
    result: Mutex<Option<Delivery>>,
    ready: Condvar,
}

impl Flight {
    fn stopped(&self) -> bool {
        !self.current.load(Ordering::Acquire)
            || (self.leader.is_cancelled()
                && self
                    .follower
                    .lock()
                    .as_ref()
                    .is_none_or(CancelToken::is_cancelled))
    }

    fn join(&self, token: &CancelToken) -> bool {
        // Serialize attachment with the leader's last-consumer cancellation
        // check. Once the walk could have stopped, its prefix is not reusable.
        let mut follower = self.follower.lock();
        if follower.as_ref().is_some_and(|prior| !prior.is_cancelled())
            || self.leader.is_cancelled()
            || !self.current.load(Ordering::Acquire)
        {
            return false;
        }
        *follower = Some(token.clone());
        true
    }

    fn wait(&self, token: &CancelToken, heartbeat: &impl Fn()) -> Delivery {
        let mut result = self.result.lock();
        loop {
            if token.is_cancelled() || !self.current.load(Ordering::Acquire) {
                return Err(InventoryError::Cancelled);
            }
            if let Some(delivery) = result.as_ref() {
                return delivery.clone();
            }
            heartbeat();
            self.ready.wait_for(&mut result, Duration::from_millis(10));
        }
    }
}

// Hold the actual directory open: an unlinked inode cannot be recycled under
// the same dev/ino while a ticket still bears it. A FIFO replacement fails at
// open rather than parking the worker. No descriptor I/O on the editor thread.
struct Directory {
    _handle: std::fs::File,
    id: (u64, u64),
}

impl Directory {
    fn open(path: &Path) -> Result<Arc<Self>, InventoryError> {
        let handle = std::fs::File::options()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(path)
            .map_err(|e| InventoryError::Walk(format!("{}: {e}", path.display())))?;
        let metadata = handle
            .metadata()
            .map_err(|e| InventoryError::Walk(e.to_string()))?;
        Ok(Arc::new(Self {
            _handle: handle,
            id: (metadata.dev(), metadata.ino()),
        }))
    }

    fn still_at(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| self.id == (m.dev(), m.ino()))
    }
}

struct Directories {
    folder: Arc<Directory>,
    pinned: Option<(InventoryRoot, Arc<Directory>)>,
}

struct Scope {
    root: Arc<PathBuf>,
    folder: Arc<PathBuf>,
    directory: Arc<Directory>,
    retired: bool,
    watching: bool,
    watch: u64,
    declaration: u64,
    invalidation: u64,
    current: Arc<AtomicBool>,
    search_hint: Option<Arc<Path>>,
    flights: [Weak<Flight>; FLIGHTS],
    writer: Weak<Flight>,
    rescan: bool,
    invalidated: Arc<Vec<PathBuf>>,
    stats: BaselineStats,
}

impl Scope {
    fn new(
        root: Arc<PathBuf>,
        folder: Arc<PathBuf>,
        directory: Arc<Directory>,
        watching: bool,
    ) -> Self {
        Self {
            root,
            folder,
            directory,
            retired: false,
            watching,
            watch: 0,
            declaration: 0,
            invalidation: 0,
            current: Arc::new(AtomicBool::new(true)),
            search_hint: None,
            flights: Default::default(),
            writer: Weak::new(),
            rescan: true,
            invalidated: Default::default(),
            stats: BaselineStats::default(),
        }
    }

    fn stamp(&self, flight: &Arc<Flight>) -> Scan {
        Scan {
            resource: Resource {
                root: identity(&self.root),
                root_incarnation: identity(&self.root),
                path: identity(&self.folder),
                path_incarnation: identity(&self.folder),
            },
            declaration: self.declaration,
            scan: identity(flight),
            invalidation: self.invalidation,
            watch: self.watch,
        }
    }

    fn retire(&mut self) {
        self.retired = true;
        self.current.store(false, Ordering::Release);
    }

    fn changed(&mut self, watch: bool) {
        self.current.store(false, Ordering::Release);
        let epoch = if watch {
            &mut self.watch
        } else {
            &mut self.invalidation
        };
        if let Some(next) = epoch.checked_add(1) {
            *epoch = next;
        } else {
            // A new incarnation will be minted on the worker. Never wrap a
            // counter while a pre-exhaustion completion could still arrive.
            self.retire();
        }
    }
}

// A bounded table of *candidates*. Finding the right root/version here never
// bypasses the per-file byte/schema verifier.
struct Shards {
    slots: [strop_kernel::cache::Slot; SCOPE_LIMIT],
    values: [Option<Arc<SymbolIndex>>; SCOPE_LIMIT],
}

impl Default for Shards {
    fn default() -> Self {
        Self {
            slots: [strop_kernel::cache::vacant(); SCOPE_LIMIT],
            values: Default::default(),
        }
    }
}

impl Shards {
    fn find(&self, scope: &Scope) -> Option<&Arc<SymbolIndex>> {
        let at = strop_kernel::cache::find(
            &self.slots,
            identity(&scope.folder),
            0,
            identity(&scope.root),
        )?;
        self.values[at].as_ref()
    }

    fn retire(&mut self, scope: &Scope) {
        strop_kernel::cache::retire(&mut self.slots, identity(&scope.folder), 0, true, false);
    }

    fn publish(
        &mut self,
        scope: &Scope,
        index: &Arc<SymbolIndex>,
        cost: usize,
    ) -> Option<Arc<SymbolIndex>> {
        self.retire(scope);
        if !scope.watching {
            return None;
        }
        let at = strop_kernel::cache::place(
            &mut self.slots,
            identity(&scope.folder),
            0,
            identity(&scope.root),
            cost,
            SHARD_BYTES_PER_SCOPE,
            0,
        )?;
        self.values[at].replace(index.clone())
    }
}

/// The editor only marks generations/prefixes. Allocation-heavy values are
/// released on a worker after it has released the owner's lock.
#[derive(Default)]
pub(crate) struct Caches {
    // Replaced on every root, watch and declaration transition. Loads must
    // return the ticket they received, not look it up again on delivery.
    declaration_ticket: Arc<()>,
    declarations: Arc<[DeclaredBoundary]>,
    pinned: Option<(Arc<PathBuf>, bool)>,
    pinned_directory: Option<Arc<Directory>>,
    // Oldest first. Four entries don't warrant a second path-to-slot map.
    scopes: Vec<Scope>,
    shards: Shards,
}

struct Build {
    flight: Arc<Flight>,
    started: Scan,
    slot: usize,
    prior: Option<Arc<SymbolIndex>>,
    invalidated: Arc<Vec<PathBuf>>,
    declarations: Arc<[DeclaredBoundary]>,
    // If the builder unwinds, a joined consumer must still wake up.
    delivered: bool,
}

impl Drop for Build {
    fn drop(&mut self) {
        if !self.delivered {
            *self.flight.result.lock() = Some(Err(InventoryError::Cancelled));
            self.flight.ready.notify_all();
        }
    }
}

enum Work {
    Build(Build),
    Joined(Arc<Flight>),
}

impl Caches {
    pub fn set_watching(&mut self, root: &Path, watching: bool) {
        match self.pinned.as_mut() {
            Some((pinned, state)) if pinned.as_path() == root => {
                if *state == watching {
                    return;
                }
                *state = watching;
            }
            _ => {
                for scope in &mut self.scopes {
                    scope.retire();
                }
                self.pinned = Some((Arc::new(root.to_owned()), watching));
                self.pinned_directory = None;
            }
        }
        self.declaration_ticket = Arc::new(());
        self.declarations = Default::default();
        for scope in self
            .scopes
            .iter_mut()
            .filter(|s| s.folder.starts_with(root))
        {
            scope.watching = watching;
            scope.rescan = true;
            scope.changed(true);
        }
    }

    pub(crate) fn retire_root(&mut self, root: &Path) {
        if self
            .pinned
            .as_ref()
            .is_some_and(|(p, _)| p.as_path() == root)
        {
            self.pinned = None;
            self.pinned_directory = None;
            self.declaration_ticket = Arc::new(());
            self.declarations = Default::default();
        }
        for scope in self
            .scopes
            .iter_mut()
            .filter(|s| s.folder.starts_with(root))
        {
            scope.retire();
        }
    }

    pub(crate) fn root_ticket(&self, root: &Path) -> Option<InventoryRoot> {
        self.pinned
            .as_ref()
            .filter(|(p, _)| p.as_path() == root)
            .map(|(p, _)| InventoryRoot(p.clone(), self.declaration_ticket.clone()))
    }

    pub(crate) fn disk_shard(
        &self,
        owner: &InventoryRoot,
        absolute: &Path,
    ) -> Option<Arc<super::symbols::DiskShard>> {
        if !Arc::ptr_eq(&owner.1, &self.declaration_ticket) {
            return None;
        }
        self.scopes.iter().rev().find_map(|scope| {
            if scope.retired
                || scope.rescan
                || !scope.watching
                || !Arc::ptr_eq(&scope.root, &owner.0)
            {
                return None;
            }
            let relative = absolute.strip_prefix(scope.folder.as_path()).ok()?;
            if scope
                .invalidated
                .iter()
                .any(|prefix| relative.starts_with(prefix))
            {
                return None;
            }
            self.shards.find(scope)?.disk_shard(relative).cloned()
        })
    }

    fn replaced_directory(&mut self, path: &Path, directory: &Directory) {
        if let Some((root, watching)) = &self.pinned {
            if root.as_path() == path
                && self
                    .pinned_directory
                    .as_ref()
                    .is_some_and(|d| d.id == directory.id)
            {
                let watching = *watching;
                self.retire_root(path);
                self.set_watching(path, watching);
                return;
            }
        }
        for scope in self
            .scopes
            .iter_mut()
            .filter(|s| s.folder.as_path() == path && s.directory.id == directory.id)
        {
            scope.retire();
        }
    }

    pub(crate) fn set_declarations(
        &mut self,
        owner: &InventoryRoot,
        records: Arc<[DeclaredBoundary]>,
    ) -> bool {
        if !Arc::ptr_eq(&owner.1, &self.declaration_ticket)
            || !self
                .pinned
                .as_ref()
                .is_some_and(|(root, _)| Arc::ptr_eq(root, &owner.0))
            || !super::catalog::declarations_fit(&records)
        {
            return false;
        }
        self.declarations = records;
        self.declaration_ticket = Arc::new(());
        for scope in self
            .scopes
            .iter_mut()
            .filter(|s| Arc::ptr_eq(&s.root, &owner.0))
        {
            scope.current.store(false, Ordering::Release);
            if let Some(next) = scope.declaration.checked_add(1) {
                scope.declaration = next;
            } else {
                scope.retire();
            }
        }
        true
    }

    pub fn invalidate(&mut self, root: &Path, paths: &[PathBuf], rescan: bool) {
        for scope in self.scopes.iter_mut().filter(|s| !s.retired) {
            let Ok(folder) = scope.folder.strip_prefix(root) else {
                continue;
            };
            let mut changed = rescan;
            let mut full = rescan;
            for path in paths {
                if full {
                    break;
                }
                // A moved ancestor invalidates the entire nested scope.
                if folder.starts_with(path) {
                    full = true;
                    changed = true;
                    break;
                }
                let Ok(prefix) = path.strip_prefix(folder) else {
                    continue;
                };
                changed = true;
                if scope.rescan || scope.invalidated.iter().any(|old| prefix.starts_with(old)) {
                    continue;
                }
                let hints = Arc::make_mut(&mut scope.invalidated);
                hints.retain(|old| !old.starts_with(prefix));
                if hints.len() >= INVALIDATION_LIMIT {
                    full = true;
                } else {
                    hints.push(prefix.to_owned());
                }
            }
            if changed {
                if full {
                    scope.rescan = true;
                    scope.invalidated = Default::default();
                }
                scope.changed(false);
            }
        }
    }

    fn scope_for(
        &mut self,
        folder: &Path,
        directories: &Directories,
    ) -> (Result<usize, InventoryError>, Option<Scope>) {
        let pinned = self
            .pinned
            .as_ref()
            .filter(|(p, _)| folder.starts_with(p.as_path()));
        match (pinned, &directories.pinned) {
            (Some((p, watching)), Some((ticket, observed)))
                if Arc::ptr_eq(p, &ticket.0)
                    && Arc::ptr_eq(&self.declaration_ticket, &ticket.1) =>
            {
                if self
                    .pinned_directory
                    .as_ref()
                    .is_some_and(|old| old.id != observed.id)
                {
                    let watching = *watching;
                    self.retire_root(&ticket.0);
                    self.set_watching(&ticket.0, watching);
                    return (Err(InventoryError::Cancelled), None);
                }
                self.pinned_directory
                    .get_or_insert_with(|| observed.clone());
            }
            (None, None) => {}
            _ => return (Err(InventoryError::Cancelled), None),
        }
        for scope in self
            .scopes
            .iter_mut()
            .filter(|s| !s.retired && s.folder.as_path() == folder)
        {
            if scope.directory.id != directories.folder.id {
                scope.retire();
            }
        }
        let mut evicted = None;
        let scope = if let Some(at) = self
            .scopes
            .iter()
            .position(|s| s.folder.as_path() == folder && !s.retired)
        {
            self.scopes.remove(at)
        } else {
            if self.scopes.len() == SCOPE_LIMIT {
                let at = self.scopes.iter().position(|s| s.retired).unwrap_or(0);
                let mut old = self.scopes.remove(at);
                old.retire();
                self.shards.retire(&old);
                evicted = Some(old);
            }
            let folder = Arc::new(folder.to_owned());
            let (root, watching) = self
                .pinned
                .as_ref()
                .filter(|(root, _)| folder.starts_with(root.as_path()))
                .cloned()
                .unwrap_or_else(|| (folder.clone(), false));
            Scope::new(root, folder, directories.folder.clone(), watching)
        };
        let at = self.scopes.len();
        self.scopes.push(scope);
        let scope = &mut self.scopes[at];
        if !scope.current.load(Ordering::Acquire) {
            scope.current = Arc::new(AtomicBool::new(true));
            scope.search_hint = None;
        }
        (Ok(at), evicted)
    }

    fn begin(
        &mut self,
        folder: &Path,
        directories: &Directories,
        query: &Arc<SearchQuery>,
        policy: SelectionPolicy,
        need_symbols: bool,
        token: &CancelToken,
    ) -> (Result<Work, InventoryError>, Option<Scope>) {
        if token.is_cancelled() {
            return (Err(InventoryError::Cancelled), None);
        }
        let (at, evicted) = self.scope_for(folder, directories);
        let at = match at {
            Ok(at) => at,
            Err(error) => return (Err(error), evicted),
        };
        let scope = &mut self.scopes[at];
        let mut vacant = None;
        for (at, slot) in scope.flights.iter().enumerate() {
            let Some(live) = slot.upgrade().filter(|f| !f.stopped()) else {
                vacant.get_or_insert(at);
                continue;
            };
            if live.policy == policy
                && (!need_symbols || live.need_symbols)
                && live.query.same_scope(query)
                && live.join(token)
            {
                return (Ok(Work::Joined(live)), evicted);
            }
        }
        let Some(slot) = vacant else {
            return (Err(InventoryError::Busy), evicted);
        };
        let flight = Arc::new(Flight {
            root: scope.root.clone(),
            folder: scope.folder.clone(),
            current: scope.current.clone(),
            query: query.clone(),
            policy,
            need_symbols,
            leader: token.clone(),
            follower: Mutex::new(None),
            result: Mutex::default(),
            ready: Condvar::new(),
        });
        let started = scope.stamp(&flight);
        scope.flights[slot] = Arc::downgrade(&flight);
        if need_symbols {
            scope.writer = Arc::downgrade(&flight);
        }
        let declarations = if self
            .pinned
            .as_ref()
            .is_some_and(|(p, _)| Arc::ptr_eq(p, &scope.root))
        {
            self.declarations.clone()
        } else {
            Default::default()
        };
        (
            Ok(Work::Build(Build {
                flight,
                started,
                slot,
                prior: if need_symbols && scope.watching && !scope.rescan {
                    self.shards.find(scope).cloned()
                } else {
                    None
                },
                invalidated: scope.invalidated.clone(),
                declarations,
                delivered: false,
            })),
            evicted,
        )
    }

    /// No entry creation here. The slot must still own this physical scan;
    /// a successful full walk cannot clear a newer watch/overflow obligation.
    fn settle(
        &mut self,
        build: &Build,
        delivery: &Delivery,
        stats: BaselineStats,
        cost: usize,
    ) -> (bool, Option<Arc<SymbolIndex>>) {
        let f = &build.flight;
        let scope = self.scopes.iter_mut().find(|s| {
            !s.retired && Arc::ptr_eq(&s.root, &f.root) && Arc::ptr_eq(&s.folder, &f.folder)
        });
        let Some(scope) = scope else {
            return (false, None);
        };
        let slot = &scope.flights[build.slot];
        let current = slot.upgrade().map_or(Refresh::Retired, |live| {
            Refresh::Pending(scope.stamp(&live))
        });
        let admitted = matches!(
            admission::settle(current, &build.started, delivery.is_ok() && !f.stopped()),
            Refresh::Clean(_)
        );
        if slot.as_ptr() == Arc::as_ptr(f) {
            scope.flights[build.slot] = Weak::new();
        }
        if !admitted {
            return (false, None);
        }
        scope.stats = stats;
        // A catalog-only refresh does not consume obligations for shards.
        // An overlapping older scan can serve its own subscribers but cannot
        // overwrite a newer shard publication.
        if f.need_symbols && scope.writer.as_ptr() == Arc::as_ptr(f) {
            if let Ok(Inventory {
                catalog,
                symbols: Some(index),
                ..
            }) = delivery
            {
                // A bounded/partial walk may deliver its known rows. It cannot
                // discharge a full rescan or certify absent entries.
                if !index.coverage_gap() && !catalog.truncated {
                    scope.rescan = false;
                    scope.invalidated = Default::default();
                }
                return (true, self.shards.publish(scope, index, cost));
            }
        }
        (true, None)
    }

    #[cfg(test)]
    pub(crate) fn stats(&self, root: &Path) -> Option<BaselineStats> {
        self.scopes
            .iter()
            .find(|s| !s.retired && s.folder.as_path() == root)
            .map(|s| s.stats)
    }
}

#[cfg(test)]
mod tests;
