use super::super::model::{CompletionPayload, DeliveryScope};
use super::super::words::{IndexWork, WordIndex};
use super::work::DocumentationWorker;
use super::*;

struct OwnedIndex {
    owner: SourceOwner,
    index: WordIndex,
}
struct PendingQuery {
    request: WordQuery,
    words_done: bool,
}
struct Tasks {
    query: Option<PendingQuery>,
    documentation: DocumentationWorker,
    retired: Vec<RetiredWork>,
    stats: CompletionWorkStats,
}
impl Default for Tasks {
    fn default() -> Self {
        Self {
            query: None,
            documentation: DocumentationWorker::default(),
            retired: Vec::with_capacity(MAX_RETIRED_WORK),
            stats: CompletionWorkStats::default(),
        }
    }
}
impl Tasks {
    /// Also called every scan quantum during a cold build. An initial index
    /// cannot withhold selected documentation, acceptance or a cached LSP list.
    fn pump(&mut self, shared: &Shared) {
        shared.retire_native(&mut self.retired, &mut self.stats);
        let (source, query, preparation) = {
            let mut state = shared.state.lock();
            let source = if state
                .source
                .as_ref()
                .is_some_and(|source| !shared.owns_source(source.owner))
            {
                state.source.take()
            } else {
                None
            };
            (source, state.query.take(), state.preparation.take())
        };
        if source.is_some() {
            drop(source);
            shared.mailbox.capacity();
        }
        if let Some(request) = query {
            self.query = Some(PendingQuery {
                words_done: !request.query.context.invocation.words(),
                request,
            });
        }
        if self.query.as_ref().is_some_and(|query| {
            !shared
                .mailbox
                .owns(query.request.query.request, DeliveryScope::Query)
        }) {
            self.query = None;
        }
        if let Some(preparation) = preparation {
            self.documentation.prepare(preparation, &shared.mailbox);
        }
        let cache = self
            .query
            .as_ref()
            .and_then(|pending| pending.request.cached_language.as_ref())
            .and_then(|_| {
                let pending = self.query.as_ref()?;
                let target = pending.request.query.context.primary()?;
                let state = shared.state.lock();
                let source = state.source.as_ref()?;
                (source.owner.document == target.source
                    && source.snapshot.revision == target.revision
                    && shared.owns_source(source.owner))
                .then(|| source.snapshot.text.clone())
            });
        if let Some(source) = cache {
            if let Some(pending) = &mut self.query {
                if let Some(cached) = pending.request.cached_language.take() {
                    let query = pending.request.query.clone();
                    let published = match &cached.payload {
                        CompletionPayload::Language(Outcome::Success(previous)) => {
                            super::work::refilter_cached(
                                query.clone(),
                                previous,
                                &source,
                                &shared.mailbox,
                            )
                        }
                        _ => false,
                    };
                    if !published && shared.mailbox.owns(query.request, DeliveryScope::Query) {
                        shared.mailbox.publish(CompletionDelivery { query, scope: DeliveryScope::Query,
                            payload: CompletionPayload::Language(Outcome::failed(FailureKind::InvalidInput,
                                "cached completion no longer applies; request fresh language completions")) });
                    }
                }
            }
        }
        shared.mailbox.retire_native();
    }

    fn words(&mut self, shared: &Shared, index: Option<&OwnedIndex>) {
        let Some(pending) = &self.query else { return };
        if pending.words_done {
            return;
        }
        let query = pending.request.query.clone();
        let Some(target) = query.context.primary() else {
            return;
        };
        let (source, failure) = {
            let state = shared.state.lock();
            let Some(source) = state.source.as_ref() else {
                return;
            };
            if source.owner.document != target.source
                || source.snapshot.revision != target.revision
                || !shared.owns_source(source.owner)
            {
                return;
            }
            (source.snapshot.text.clone(), source.failure)
        };
        let outcome = if let Some(error) = failure {
            Outcome::failed(FailureKind::Protocol, error.to_string())
        } else {
            let Some(index) = index.filter(|index| {
                index.owner.document == target.source
                    && shared.owns_source(index.owner)
                    && index.index.revision() == Some(target.revision)
            }) else {
                return;
            };
            let prefix = match source.get_byte_slice(target.start..target.caret) {
                Some(prefix) => prefix.to_string(),
                None => {
                    self.fail_words(shared, &query, IndexError::InvalidGeometry);
                    return;
                }
            };
            self.stats.queries = self.stats.queries.saturating_add(1);
            let result =
                index
                    .index
                    .query(target.revision, &prefix, target.end > target.caret, || {
                        self.pump(shared);
                        !shared.mailbox.owns(query.request, DeliveryScope::Query)
                    });
            match result {
                Ok(items) => Outcome::Success(items),
                Err(IndexError::Cancelled) => return,
                Err(error) => Outcome::failed(FailureKind::Protocol, error.to_string()),
            }
        };
        self.mark_words_done(query.request);
        shared.mailbox.publish(CompletionDelivery {
            query,
            scope: DeliveryScope::Query,
            payload: CompletionPayload::Words {
                outcome,
                work: self.stats,
            },
        });
    }

    fn mark_words_done(&mut self, request: WorkerId) {
        if let Some(query) = self
            .query
            .as_mut()
            .filter(|query| query.request.query.request == request)
        {
            query.words_done = true;
        }
    }
    fn fail_words(&mut self, shared: &Shared, query: &Arc<CompletionQuery>, error: IndexError) {
        self.mark_words_done(query.request);
        shared.mailbox.publish(CompletionDelivery {
            query: query.clone(),
            scope: DeliveryScope::Query,
            payload: CompletionPayload::Words {
                outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                work: self.stats,
            },
        });
    }
}

struct MaintenanceJob {
    owner: SourceOwner,
    snapshot: FrozenSource,
    maintenance: Maintenance,
    words: bool,
}
fn maintenance(shared: &Shared, journal: &mut WordJournal) -> Option<MaintenanceJob> {
    let mut state = shared.state.lock();
    let source = state.source.as_mut()?;
    if !shared.owns_source(source.owner)
        || source.snapshot.revision != source.current_revision
        || matches!(source.maintenance, Maintenance::Clean)
        || source.failure.is_some()
    {
        return None;
    }
    std::mem::swap(&mut source.journal, journal);
    source
        .journal
        .reset(source.snapshot.revision, source.snapshot.text.len_bytes());
    Some(MaintenanceJob {
        owner: source.owner,
        snapshot: FrozenSource {
            revision: source.snapshot.revision,
            text: source.snapshot.text.clone(),
        },
        maintenance: std::mem::replace(&mut source.maintenance, Maintenance::Clean),
        words: source.words,
    })
}

pub(super) fn run(shared: &Shared) {
    let mut index: Option<OwnedIndex> = None;
    let mut tasks = Tasks::default();
    let mut journal = WordJournal::new(BufferRevision::new(0), 0);
    loop {
        {
            let mut state = shared.state.lock();
            if shared.stopping.load(Ordering::Acquire) {
                break;
            }
            state.wake = false;
        }
        tasks.pump(shared);
        if index
            .as_ref()
            .is_some_and(|index| !shared.owns_source(index.owner))
        {
            index = None;
            tasks.stats.indexed_words = 0;
            tasks.stats.indexed_word_bytes = 0;
        }
        if let Some(job) = maintenance(shared, &mut journal) {
            if job.words {
                let rebuild = !matches!(job.maintenance, Maintenance::Changed)
                    || index.as_ref().is_none_or(|index| index.owner != job.owner);
                let mut cancelled = || {
                    tasks.pump(shared);
                    !shared.owns_source(job.owner)
                };
                let result: Result<IndexWork, IndexError> = if rebuild {
                    index = None;
                    match WordIndex::build(job.snapshot.revision, job.snapshot.text, &mut cancelled)
                    {
                        Ok((built, work)) => {
                            index = Some(OwnedIndex {
                                owner: job.owner,
                                index: built,
                            });
                            Ok(work)
                        }
                        Err(error) => Err(error),
                    }
                } else if let Some(index) = &mut index {
                    index
                        .index
                        .advance(job.snapshot.text, &journal, &mut cancelled)
                } else {
                    unreachable!("an absent index requires a rebuild")
                };
                match result {
                    Ok(work) => {
                        tasks.stats.scanned_bytes = tasks
                            .stats
                            .scanned_bytes
                            .saturating_add(work.scanned_bytes as u64);
                        if rebuild {
                            tasks.stats.builds = tasks.stats.builds.saturating_add(1);
                        } else {
                            tasks.stats.incremental_updates =
                                tasks.stats.incremental_updates.saturating_add(1);
                        }
                        if let Some(index) = &index {
                            tasks.stats.indexed_words = index.index.retained_words();
                            tasks.stats.indexed_word_bytes = index.index.retained_word_bytes();
                        }
                    }
                    Err(IndexError::Cancelled) => {
                        index = None;
                    }
                    Err(error) => {
                        index = None;
                        let mut state = shared.state.lock();
                        if let Some(source) = state
                            .source
                            .as_mut()
                            .filter(|source| source.owner == job.owner)
                        {
                            source.failure = Some(error);
                        }
                    }
                }
            }
        }
        tasks.pump(shared);
        tasks.words(shared, index.as_ref());
        shared.mailbox.retire_native();
        let mut state = shared.state.lock();
        while !state.wake && !shared.stopping.load(Ordering::Acquire) {
            shared.available.wait(&mut state);
        }
    }
}
