use super::context::{Invocation, SourceBinding};
use super::mailbox::{CompletionMailbox, PublicationUsage};
use super::model::{
    CandidateKey, CompletionDelivery, CompletionPayload, CompletionQuery, LanguageSuggestions,
};
use super::worker::Worker;
use std::sync::Arc;
use strop_core::id::BufferRevision;
use strop_core::worker::{Failure, WorkerId};
use strop_lsp::completion::{CompletionEntry, CompletionItem, CompletionWake};
use strop_lsp::RequestStamp;

#[derive(Clone, Copy)]
pub(super) enum Direction {
    Next,
    Previous,
}

pub(super) enum Selection {
    None,
    Awaiting(Direction),
    Chosen {
        key: CandidateKey,
        generation: WorkerId,
    },
}
pub(super) enum ProviderState {
    Off,
    Pending,
    WaitingCapacity,
    Initializing,
    Unavailable(&'static str),
    Failed(Failure),
    Settled(Arc<CompletionDelivery>),
}
impl ProviderState {
    pub fn pending(&self) -> bool {
        matches!(
            self,
            Self::Pending | Self::WaitingCapacity | Self::Initializing
        )
    }
}
pub(super) enum Acceptance {
    Idle,
    Resolving {
        candidate: CandidateKey,
    },
    Preparing {
        candidate: CandidateKey,
        work: WorkerId,
        /// Tab cycling (0059 §6.1): the prepared plan applies as a
        /// speculative preview, not an acceptance.
        preview: bool,
    },
    WaitingCapacity {
        candidate: CandidateKey,
        work: WorkerId,
        preview: bool,
    },
}
/// One applied Tab live preview (0059 §6.1). `revision` is the post-apply
/// revision, so a foreign interleaved edit refuses the revert instead of
/// moving user data. `inverse` is the exact inverse of the applied plan
/// against the post-preview snapshot; the revert replays it inside the same
/// insert undo unit, so previews never add user-visible undo steps or
/// history branches.
pub(super) struct ActivePreview {
    pub document: strop_core::id::DocumentId,
    pub revision: BufferRevision,
    pub inverse: Vec<strop_core::Replacement>,
}
pub(super) enum ResolveState {
    Idle,
    Pending(RequestStamp),
    WaitingCapacity,
    Failed(strop_lsp::RequestRefusal),
    Complete(Arc<CompletionDelivery>),
}
impl ResolveState {
    pub fn request(&self) -> Option<RequestStamp> {
        match self {
            Self::Pending(stamp) => Some(*stamp),
            _ => None,
        }
    }
}

pub(super) struct Session {
    pub query: Arc<CompletionQuery>,
    pub words: ProviderState,
    pub language: ProviderState,
    pub selection: Selection,
    /// Applied Tab live preview awaiting revert/commit (0059 §6.1).
    pub preview: Option<ActivePreview>,
    pub acceptance: Acceptance,
    pub request: Option<RequestStamp>,
    pub resolve: ResolveState,
    pub documentation: Option<Arc<CompletionDelivery>>,
    pub preparation: Option<WorkerId>,
    pub query_waiting: bool,
    pub cached_language: Option<Arc<CompletionDelivery>>,
    pub trigger: strop_lsp::completion::CompletionTrigger,
}
impl Session {
    pub fn new(query: Arc<CompletionQuery>, direction: Option<Direction>) -> Self {
        let words = if query.context.invocation.words() {
            ProviderState::Pending
        } else {
            ProviderState::Off
        };
        let language = if query.context.invocation.language() {
            ProviderState::Pending
        } else {
            ProviderState::Off
        };
        Self {
            query,
            words,
            language,
            selection: direction.map_or(Selection::None, Selection::Awaiting),
            preview: None,
            acceptance: Acceptance::Idle,
            request: None,
            resolve: ResolveState::Idle,
            documentation: None,
            preparation: None,
            query_waiting: false,
            cached_language: None,
            trigger: strop_lsp::completion::CompletionTrigger::Invoked,
        }
    }
    pub fn language(&self) -> Option<&LanguageSuggestions> {
        let ProviderState::Settled(delivery) = &self.language else {
            return None;
        };
        match &delivery.payload {
            CompletionPayload::Language(strop_core::worker::Outcome::Success(suggestions)) => {
                Some(suggestions)
            }
            _ => None,
        }
    }
    pub fn words(&self) -> Option<&super::words::WordSuggestions> {
        let ProviderState::Settled(delivery) = &self.words else {
            return None;
        };
        match &delivery.payload {
            CompletionPayload::Words {
                outcome: strop_core::worker::Outcome::Success(words),
                ..
            } => Some(words),
            _ => None,
        }
    }
    pub fn count(&self) -> usize {
        self.language()
            .map_or(0, |suggestions| suggestions.items.items.len())
            + self
                .words()
                .map_or(0, |suggestions| suggestions.items.len())
    }
    pub fn candidate(&self, index: usize) -> Option<Candidate<'_>> {
        let language = self.language();
        let language_len = language.map_or(0, |suggestions| suggestions.items.items.len());
        if index < language_len {
            let language = language?;
            Some(Candidate::Language {
                entry: &language.items.items[index],
                suggestions: language,
            })
        } else {
            self.words()?
                .items
                .get(index - language_len)
                .map(Candidate::Word)
        }
    }
    pub fn selected(&self) -> Option<(usize, Candidate<'_>)> {
        let Selection::Chosen { key, .. } = &self.selection else {
            return None;
        };
        (0..self.count()).find_map(|index| {
            let candidate = self.candidate(index)?;
            candidate.matches(key).then_some((index, candidate))
        })
    }
    pub fn resolved_item(&self, candidate: &CandidateKey) -> Option<&Arc<CompletionItem>> {
        let ResolveState::Complete(delivery) = &self.resolve else {
            return None;
        };
        match &delivery.payload {
            CompletionPayload::Resolved {
                candidate: owner,
                reply:
                    strop_lsp::completion::CompletionReply {
                        outcome:
                            strop_core::worker::Outcome::Success(
                                strop_lsp::completion::CompletionResult::Resolved(item),
                            ),
                        ..
                    },
            } if owner == candidate => Some(item),
            _ => None,
        }
    }
    pub fn documentation_pending(&self) -> bool {
        let (
            Selection::Chosen {
                key: CandidateKey::Language { .. },
                generation,
            },
            Some(work),
        ) = (&self.selection, self.preparation)
        else {
            return false;
        };
        if !matches!(self.acceptance, Acceptance::Idle) {
            return false;
        }
        let scope = super::model::DeliveryScope::Preparation {
            selection: *generation,
            work,
        };
        self.documentation
            .as_ref()
            .is_none_or(|delivery| delivery.scope != scope)
    }
}

pub(super) enum Candidate<'a> {
    Word(&'a super::words::WordCandidate),
    Language {
        entry: &'a CompletionEntry,
        suggestions: &'a LanguageSuggestions,
    },
}
impl Candidate<'_> {
    pub fn key(&self) -> CandidateKey {
        match self {
            Self::Word(word) => CandidateKey::Word(word.text.clone()),
            Self::Language { entry, suggestions } => CandidateKey::Language {
                request: suggestions.origin.stamp,
                ordinal: entry.ordinal,
            },
        }
    }
    pub fn matches(&self, key: &CandidateKey) -> bool {
        match (self, key) {
            (Self::Word(word), CandidateKey::Word(text)) => word.text == *text,
            (
                Self::Language { entry, suggestions },
                CandidateKey::Language { request, ordinal },
            ) => suggestions.origin.stamp == *request && entry.ordinal == *ordinal,
            _ => false,
        }
    }
}

pub(super) struct SourceLease {
    pub generation: WorkerId,
    pub binding: SourceBinding,
    pub revision: BufferRevision,
    pub snapshot_waiting: bool,
    pub language_attachment_requested: bool,
    pub language_context: Option<Arc<super::context::LanguageContext>>,
}
pub(super) enum RetirementStage {
    Record,
    ThreadExit,
}
pub(super) enum WorkerLifecycle {
    Idle,
    Failed(Failure),
    Running {
        native: Option<Worker>,
        mailbox: Arc<CompletionMailbox>,
        wake: CompletionWake,
    },
    Stopping {
        native: Option<Worker>,
        mailbox: Arc<CompletionMailbox>,
        stage: RetirementStage,
    },
}
impl WorkerLifecycle {
    pub fn mailbox(&self) -> Option<&Arc<CompletionMailbox>> {
        match self {
            Self::Running { mailbox, .. } | Self::Stopping { mailbox, .. } => Some(mailbox),
            Self::Idle | Self::Failed(_) => None,
        }
    }
    pub fn native(&self) -> Option<&Worker> {
        match self {
            Self::Running { native, .. } | Self::Stopping { native, .. } => native.as_ref(),
            Self::Idle | Self::Failed(_) => None,
        }
    }
}

pub(super) struct RequestIntent {
    pub invocation: Invocation,
    pub direction: Option<Direction>,
    pub trigger: strop_lsp::completion::CompletionTrigger,
    pub cached_language: Option<Arc<CompletionDelivery>>,
    pub selection: Option<CandidateKey>,
}
#[derive(Default)]
pub(super) enum InsertPrefix {
    #[default]
    None,
    ControlX,
}

pub(crate) struct CompletionState {
    pub(super) settings_generation: u64,
    pub(super) session: Option<Session>,
    pub(super) worker: WorkerLifecycle,
    pub(super) source: Option<SourceLease>,
    pub(super) intent: Option<RequestIntent>,
    pub(super) prefix: InsertPrefix,
    pub(super) metrics: PublicationUsage,
    pub(super) prefix_growth: Option<(WorkerId, BufferRevision)>,
}
impl Default for CompletionState {
    fn default() -> Self {
        Self::from_seed(0)
    }
}
impl CompletionState {
    pub fn seed_generation(&self) -> Option<u64> {
        (matches!(self.worker, WorkerLifecycle::Idle)
            && self.session.is_none()
            && self.intent.is_none())
        .then_some(self.settings_generation)
    }
    pub fn from_seed(settings_generation: u64) -> Self {
        Self {
            settings_generation,
            session: None,
            worker: WorkerLifecycle::Idle,
            source: None,
            intent: None,
            prefix: InsertPrefix::None,
            metrics: PublicationUsage::default(),
            prefix_growth: None,
        }
    }
    pub fn connect_events(&self, sender: crate::editor::events::EventSender) {
        if let Some(mailbox) = self.worker.mailbox() {
            mailbox.connect(sender);
        }
    }
    pub fn pending(&self) -> bool {
        matches!(self.worker, WorkerLifecycle::Stopping { .. })
            || self.session.as_ref().is_some_and(|session| {
                session.words.pending()
                    || session.language.pending()
                    || session.resolve.request().is_some()
                    || !matches!(session.acceptance, Acceptance::Idle)
                    || session.documentation_pending()
            })
    }
    pub fn blocks_deferred_input(&self) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        match session.acceptance {
            Acceptance::Preparing { .. } | Acceptance::WaitingCapacity { .. } => return true,
            Acceptance::Resolving { .. } => return session.resolve.request().is_some(),
            Acceptance::Idle => {}
        }
        let requested = session.query.context.invocation.manual()
            || matches!(session.selection, Selection::Awaiting(_));
        requested
            && session.count() == 0
            && (matches!(session.words, ProviderState::Pending)
                || matches!(
                    session.language,
                    ProviderState::Pending | ProviderState::Initializing
                ))
    }
    pub fn released(&self) {
        if let Some(mailbox) = self.worker.mailbox() {
            mailbox.released();
        }
        if let Some(worker) = self.worker.native() {
            worker.released();
        }
    }
}
impl Drop for CompletionState {
    fn drop(&mut self) {
        self.session = None;
        self.intent = None;
        if let Some(worker) = self.worker.native() {
            worker.stop();
        }
        self.released();
    }
}
