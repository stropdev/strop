use std::path::PathBuf;

use strop_core::id::{BufferRevision, DocumentId};
use strop_terminal::model::SessionId;
use strop_workspace::RemoteEndpoint;

use crate::editor::remote::RemoteControl;
use crate::editor::terminal::actions::TerminalAction;

/// Stable identity of one retained feedback record. Invocations capture this
/// identity rather than a position in the bounded history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FeedbackId(u64);

impl FeedbackId {
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FeedbackSeverity {
    Info,
    Warning,
    Failure,
}

impl FeedbackSeverity {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Failure => "failure",
        }
    }
}

/// Resolution removes recovery actions but retains the recent result for
/// `:explain`. Dismissal only changes presentation; the record is unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackLifecycle {
    Active,
    Dismissed,
    Resolved,
}

impl FeedbackLifecycle {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Dismissed => "dismissed",
            Self::Resolved => "resolved",
        }
    }

    pub const fn unresolved(self) -> bool {
        !matches!(self, Self::Resolved)
    }
}

/// The resource whose recovery settles a record. Generational document IDs,
/// and endpoint incarnations prevent a stale record
/// from addressing a replacement resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalFeedbackPhase {
    Live,
    Exited,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FeedbackOwner {
    Save {
        document: DocumentId,
        remote: bool,
    },
    Remote {
        endpoint: RemoteEndpoint,
        incarnation: u64,
    },
    RemoteControls,
    Terminal {
        session: SessionId,
        incarnation: u64,
        phase: TerminalFeedbackPhase,
    },
}

impl FeedbackOwner {
    pub fn label(&self) -> String {
        match self {
            Self::Save { document, remote } => format!(
                "{} document {}:{}",
                if *remote { "remote" } else { "local" },
                document.index(),
                document.generation()
            ),
            Self::Remote { endpoint, .. } => endpoint.to_string(),
            Self::RemoteControls => "remote connections".into(),
            Self::Terminal {
                session,
                incarnation,
                phase,
            } => format!(
                "terminal #{} (incarnation {incarnation}, {phase:?})",
                session.get()
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FeedbackKind {
    Save,
    Connection,
    TerminalProcess,
    TerminalHostControl,
}

/// A reference to evidence already owned by the save machinery. Receipts and
/// prepared contents stay in `StoreAttempt`; feedback never
/// clones them into a second evidence store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackReference {
    LocalStore {
        document: DocumentId,
        revision: BufferRevision,
    },
}

impl FeedbackReference {
    pub fn label(self) -> String {
        match self {
            Self::LocalStore { document, revision } => format!(
                "local StoreAttempt for document {}:{}, revision {} (receipt retained by save machinery)",
                document.index(),
                document.generation(),
                revision.get()
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SaveRetry {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub target: Option<PathBuf>,
    pub force: bool,
    pub close: bool,
}

/// Typed, revalidated recovery. No executable command text is retained.
#[derive(Debug, Clone)]
pub(crate) enum TypedAction {
    Dismiss,
    RetrySave(SaveRetry),
    VerifySave {
        document: DocumentId,
        revision: BufferRevision,
    },
    RetryRemote {
        operation: RemoteControl,
        incarnation: u64,
    },
    Terminal(TerminalAction),
}

impl TypedAction {
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            Self::Dismiss => "Dismiss notice (operation remains unchanged)",
            Self::RetrySave(_) => "Retry the failed save from the same buffer revision",
            Self::VerifySave { .. } => "Verify the unconfirmed save without rewriting",
            Self::RetryRemote { .. } => "Retry the remote connection",
            Self::Terminal(action) => action.label(),
        }
    }
}

pub struct FeedbackRecord {
    id: FeedbackId,
    severity: FeedbackSeverity,
    owner: FeedbackOwner,
    pub(crate) kind: FeedbackKind,
    summary: String,
    detail: String,
    reference: Option<FeedbackReference>,
    lifecycle: FeedbackLifecycle,
    occurrences: u64,
    pub(crate) actions: Vec<TypedAction>,
}

impl FeedbackRecord {
    pub(crate) fn new(id: FeedbackId, event: FeedbackEvent) -> Self {
        Self {
            id,
            severity: event.severity,
            owner: event.owner,
            kind: event.kind,
            summary: event.summary,
            detail: event.detail,
            reference: event.reference,
            lifecycle: FeedbackLifecycle::Active,
            occurrences: 1,
            actions: event.actions,
        }
    }

    pub const fn id(&self) -> FeedbackId {
        self.id
    }
    pub const fn severity(&self) -> FeedbackSeverity {
        self.severity
    }
    pub const fn owner(&self) -> &FeedbackOwner {
        &self.owner
    }
    pub fn summary(&self) -> &str {
        &self.summary
    }
    pub fn detail(&self) -> &str {
        &self.detail
    }
    pub const fn reference(&self) -> Option<FeedbackReference> {
        self.reference
    }
    pub const fn lifecycle(&self) -> FeedbackLifecycle {
        self.lifecycle
    }
    pub const fn occurrences(&self) -> u64 {
        self.occurrences
    }
    pub const fn unresolved(&self) -> bool {
        self.lifecycle.unresolved()
    }
    pub fn action_labels(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.actions.iter().map(TypedAction::label)
    }

    pub(crate) fn replace(
        &mut self,
        severity: FeedbackSeverity,
        summary: String,
        detail: String,
        reference: Option<FeedbackReference>,
        actions: Vec<TypedAction>,
    ) {
        self.severity = severity;
        self.summary = summary;
        self.detail = detail;
        self.reference = reference;
        self.actions = actions;
        self.lifecycle = FeedbackLifecycle::Active;
        self.occurrences = self.occurrences.saturating_add(1);
    }

    pub(crate) fn dismiss(&mut self) {
        if self.lifecycle == FeedbackLifecycle::Active {
            self.lifecycle = FeedbackLifecycle::Dismissed;
        }
    }

    pub(crate) fn resolve(&mut self) {
        self.lifecycle = FeedbackLifecycle::Resolved;
        self.actions.clear();
    }
}

pub(crate) struct FeedbackEvent {
    pub severity: FeedbackSeverity,
    pub owner: FeedbackOwner,
    pub kind: FeedbackKind,
    pub summary: String,
    pub detail: String,
    pub reference: Option<FeedbackReference>,
    pub actions: Vec<TypedAction>,
}
