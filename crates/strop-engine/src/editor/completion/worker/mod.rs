//! One persistent CPU owner. Source maintenance and query cancellation have
//! separate generations; typing cannot repeatedly restart the initial index.
mod drive;
mod work;
use super::mailbox::CompletionMailbox;
use super::model::{CompletionDelivery, CompletionQuery, CompletionWorkStats};
use super::words::{IndexError, WordJournal};
use parking_lot::{Condvar, Mutex};
use ropey::Rope;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{FailureKind, Outcome, WorkerId};
pub(super) use work::{Preparation, PreparationOperation};

pub(super) const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_RETIRED_WORK: usize = 8;
const MAX_RETIRED_BYTES: usize = 72 * 1024 * 1024;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error,
)]
pub(super) enum WorkRefusal {
    #[error("completion worker is closed")]
    Closed,
    #[error("completion is waiting for bounded retirement capacity")]
    Busy,
    #[error("completion source exceeds the 64 MiB snapshot bound")]
    SourceTooLarge,
    #[error("completion no longer owns this source")]
    WrongSource,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SourceOwner {
    generation: WorkerId,
    document: DocumentId,
}
struct FrozenSource {
    revision: BufferRevision,
    text: Rope,
}
#[derive(Clone, Copy)]
enum Maintenance {
    Clean,
    Changed,
    Rebuild,
}
struct SourceState {
    owner: SourceOwner,
    current_revision: BufferRevision,
    snapshot: FrozenSource,
    journal: WordJournal,
    maintenance: Maintenance,
    words: bool,
    failure: Option<IndexError>,
}
struct WordQuery {
    query: Arc<CompletionQuery>,
    cached_language: Option<Arc<CompletionDelivery>>,
}
impl WordQuery {
    fn charged_bytes(&self) -> usize {
        1024 * 1024
            + self
                .cached_language
                .as_ref()
                .map_or(0, |cache| cache.payload.charged_bytes())
    }
}
enum RetiredWork {
    Snapshot(Rope),
    Source(SourceState),
    Query(WordQuery),
    Preparation(Preparation),
}
impl RetiredWork {
    fn charged_bytes(&self) -> usize {
        match self {
            Self::Snapshot(text) => text.len_bytes(),
            Self::Source(source) => source.snapshot.text.len_bytes() + 16 * 1024,
            Self::Query(query) => query.charged_bytes(),
            Self::Preparation(work) => work.charged_bytes(),
        }
    }
    fn source_snapshots(&self) -> u64 {
        match self {
            Self::Snapshot(_) | Self::Source(_) => 1,
            Self::Preparation(work)
                if matches!(work.operation, PreparationOperation::Language { .. }) =>
            {
                1
            }
            _ => 0,
        }
    }
}
struct State {
    source: Option<SourceState>,
    query: Option<WordQuery>,
    preparation: Option<Preparation>,
    retired: Vec<RetiredWork>,
    retired_bytes: usize,
    retirement_high_water_bytes: usize,
    wake: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            source: None,
            query: None,
            preparation: None,
            retired: Vec::with_capacity(MAX_RETIRED_WORK),
            retired_bytes: 0,
            retirement_high_water_bytes: 0,
            wake: false,
        }
    }
}
impl State {
    fn room_for_retirement(&self, bytes: usize) -> bool {
        self.retired.len() < MAX_RETIRED_WORK && self.retired_bytes + bytes <= MAX_RETIRED_BYTES
    }
    fn retire(&mut self, value: RetiredWork) {
        let bytes = value.charged_bytes();
        debug_assert!(
            self.room_for_retirement(bytes),
            "input preflights every retired owner"
        );
        self.retired_bytes += bytes;
        self.retirement_high_water_bytes = self.retirement_high_water_bytes.max(self.retired_bytes);
        self.retired.push(value);
    }
    fn replace_snapshot(
        &mut self,
        revision: BufferRevision,
        text: &Rope,
    ) -> Result<(), WorkRefusal> {
        let Some(source) = &self.source else {
            return Err(WorkRefusal::WrongSource);
        };
        if source.snapshot.revision == revision {
            return Ok(());
        }
        if !self.room_for_retirement(source.snapshot.text.len_bytes()) {
            return Err(WorkRefusal::Busy);
        }
        let Some(source) = &mut self.source else {
            unreachable!("source was checked above")
        };
        let previous = std::mem::replace(
            &mut source.snapshot,
            FrozenSource {
                revision,
                text: text.clone(),
            },
        );
        self.retire(RetiredWork::Snapshot(previous.text));
        Ok(())
    }
}
struct Shared {
    source_generation: AtomicU64,
    stopping: AtomicBool,
    state: Mutex<State>,
    available: Condvar,
    mailbox: Arc<CompletionMailbox>,
}
impl Shared {
    fn wake(&self) {
        self.state.lock().wake = true;
        self.available.notify_one();
    }
    fn owns_source(&self, owner: SourceOwner) -> bool {
        !self.stopping.load(Ordering::Acquire)
            && self.source_generation.load(Ordering::Acquire) == owner.generation.get()
    }
    fn retire_native(&self, retired: &mut Vec<RetiredWork>, stats: &mut CompletionWorkStats) {
        debug_assert!(retired.is_empty());
        let mut state = self.state.lock();
        std::mem::swap(&mut state.retired, retired);
        state.retired_bytes = 0;
        stats.retirement_high_water_bytes = stats
            .retirement_high_water_bytes
            .max(state.retirement_high_water_bytes);
        drop(state);
        let freed = !retired.is_empty();
        for value in retired.iter() {
            stats.retired_snapshots = stats
                .retired_snapshots
                .saturating_add(value.source_snapshots());
        }
        retired.clear();
        self.mailbox.retire_native();
        if freed {
            self.mailbox.capacity();
        }
    }
    fn clear_native(&self) {
        let state = std::mem::take(&mut *self.state.lock());
        drop(state);
    }
}

pub(super) struct Worker {
    shared: Arc<Shared>,
    thread: std::thread::JoinHandle<()>,
}
impl Worker {
    pub fn start(mailbox: Arc<CompletionMailbox>) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            source_generation: AtomicU64::new(0),
            stopping: AtomicBool::new(false),
            state: Mutex::new(State::default()),
            available: Condvar::new(),
            mailbox,
        });
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("strop-completion".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drive::run(&worker)));
                worker.stopping.store(true, Ordering::Release);
                worker.clear_native();
                worker.mailbox.finish_native(match result {
                    Ok(()) => Outcome::Success(()),
                    Err(_) => Outcome::failed(FailureKind::Panic, "completion worker panicked"),
                });
            })?;
        Ok(Self { shared, thread })
    }
    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }
    pub fn stop(&self) {
        self.shared.stopping.store(true, Ordering::Release);
        self.shared.source_generation.store(0, Ordering::Release);
        self.shared.mailbox.query(None);
        self.shared.wake();
    }
    pub fn released(&self) {
        self.shared.mailbox.released();
        self.shared.wake();
    }
    pub fn forget_source(&self) {
        self.shared.source_generation.store(0, Ordering::Release);
        self.shared.wake();
    }

    pub fn source(
        &self,
        generation: WorkerId,
        document: DocumentId,
        revision: BufferRevision,
        text: &Rope,
        words: bool,
    ) -> Result<(), WorkRefusal> {
        if self.shared.stopping.load(Ordering::Acquire) {
            return Err(WorkRefusal::Closed);
        }
        if text.len_bytes() > MAX_SOURCE_BYTES {
            self.forget_source();
            return Err(WorkRefusal::SourceTooLarge);
        }
        self.shared
            .source_generation
            .store(generation.get(), Ordering::Release);
        let owner = SourceOwner {
            generation,
            document,
        };
        let mut state = self.shared.state.lock();
        state.wake = true;
        self.shared.available.notify_one();
        if state
            .source
            .as_ref()
            .is_some_and(|source| source.owner == owner)
        {
            let Some(source) = &mut state.source else {
                unreachable!("source was checked above")
            };
            if source.current_revision != revision {
                // A bounded journal can lose continuity, but cannot publish as
                // current without a real build of the replacement snapshot.
                source.journal.reset(revision, text.len_bytes());
                source.current_revision = revision;
                source.maintenance = Maintenance::Rebuild;
            }
            if words && !source.words {
                source.maintenance = Maintenance::Rebuild;
            }
            source.words |= words;
            return state.replace_snapshot(revision, text);
        }
        if let Some(source) = &state.source {
            if !state.room_for_retirement(source.snapshot.text.len_bytes() + 16 * 1024) {
                return Err(WorkRefusal::Busy);
            }
        }
        if let Some(source) = state.source.take() {
            state.retire(RetiredWork::Source(source));
        }
        state.source = Some(SourceState {
            owner,
            current_revision: revision,
            snapshot: FrozenSource {
                revision,
                text: text.clone(),
            },
            journal: WordJournal::new(revision, text.len_bytes()),
            maintenance: Maintenance::Rebuild,
            words,
            failure: None,
        });
        Ok(())
    }

    pub fn edits(
        &self,
        document: DocumentId,
        revision: BufferRevision,
        text: &Rope,
        changes: &[strop_core::Change],
    ) -> Result<(), WorkRefusal> {
        if self.shared.stopping.load(Ordering::Acquire) {
            return Err(WorkRefusal::Closed);
        }
        let mut state = self.shared.state.lock();
        let Some(source) = &mut state.source else {
            return Err(WorkRefusal::WrongSource);
        };
        if source.owner.document != document {
            return Err(WorkRefusal::WrongSource);
        }
        if text.len_bytes() > MAX_SOURCE_BYTES {
            self.shared.source_generation.store(0, Ordering::Release);
            state.wake = true;
            self.shared.available.notify_one();
            return Err(WorkRefusal::SourceTooLarge);
        }
        source.current_revision = revision;
        match source.journal.record(changes) {
            Ok(()) => {
                if matches!(source.maintenance, Maintenance::Clean) {
                    source.maintenance = Maintenance::Changed;
                }
            }
            Err(IndexError::JournalGap | IndexError::JournalLimit) => {
                source.journal.reset(revision, text.len_bytes());
                source.maintenance = Maintenance::Rebuild;
            }
            Err(error) => source.failure = Some(error),
        }
        state.wake = true;
        self.shared.available.notify_one();
        state.replace_snapshot(revision, text)
    }

    pub fn query(
        &self,
        query: &Arc<CompletionQuery>,
        cached_language: Option<&Arc<CompletionDelivery>>,
    ) -> Result<(), WorkRefusal> {
        if self.shared.stopping.load(Ordering::Acquire) {
            return Err(WorkRefusal::Closed);
        }
        let mut state = self.shared.state.lock();
        if state
            .query
            .as_ref()
            .is_some_and(|query| !state.room_for_retirement(query.charged_bytes()))
        {
            return Err(WorkRefusal::Busy);
        }
        if let Some(previous) = state.query.take() {
            state.retire(RetiredWork::Query(previous));
        }
        state.query = Some(WordQuery {
            query: query.clone(),
            cached_language: cached_language.cloned(),
        });
        state.wake = true;
        self.shared.available.notify_one();
        Ok(())
    }

    pub fn prepare(&self, preparation: Preparation) -> Result<(), WorkRefusal> {
        if self.shared.stopping.load(Ordering::Acquire) {
            return Err(WorkRefusal::Closed);
        }
        if matches!(&preparation.operation, PreparationOperation::Language { source, .. } if source.len_bytes() > MAX_SOURCE_BYTES)
        {
            return Err(WorkRefusal::SourceTooLarge);
        }
        let mut state = self.shared.state.lock();
        if state
            .preparation
            .as_ref()
            .is_some_and(|work| !state.room_for_retirement(work.charged_bytes()))
        {
            return Err(WorkRefusal::Busy);
        }
        if let Some(previous) = state.preparation.take() {
            state.retire(RetiredWork::Preparation(previous));
        }
        state.preparation = Some(preparation);
        state.wake = true;
        self.shared.available.notify_one();
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
