use super::WorkspaceKey;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use strop_containers::devcontainer::{
    CliConfiguration, CliOperation, CliVersion, ConfigIdentity, DevcontainerError, DiscoveryReport,
    EngineIdentity, EnvironmentContext, JobId, JobOutcome, JobSnapshot, JobState, OwnershipReceipt,
    PartialResource, Reconciliation, ReferenceCli, TrustReview,
};
use strop_core::worker::CancelHandle;
use strop_workspace::ResourceLocation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct OperationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OperationKind {
    ProbeCli,
    ReadConfiguration { merged: bool },
    Attach,
    Start,
    Rebuild,
}
impl OperationKind {
    pub fn lifecycle(self) -> Option<CliOperation> {
        match self {
            Self::Attach => Some(CliOperation::Attach),
            Self::Start => Some(CliOperation::Start),
            Self::Rebuild => Some(CliOperation::Rebuild),
            Self::ProbeCli | Self::ReadConfiguration { .. } => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ProbeCli => "CLI probe",
            Self::ReadConfiguration { merged: false } => "configuration read",
            Self::ReadConfiguration { merged: true } => "merged configuration read",
            Self::Attach => "attach",
            Self::Start => "start",
            Self::Rebuild => "rebuild",
        }
    }
}

/// Keyboard-reachable Dev Container intent. Every discovery-scoped action is
/// revalidated against the selected configuration and current discovery before
/// it can invoke the state API; no command text or notification callback is
/// retained here.
#[derive(Debug, Clone)]
pub enum DevcontainerAction {
    Review {
        workspace: WorkspaceKey,
        identity: ConfigIdentity,
        discovery: u64,
    },
    Later {
        workspace: WorkspaceKey,
        identity: ConfigIdentity,
        discovery: u64,
    },
    SuppressWorkspace {
        workspace: WorkspaceKey,
        discovery: u64,
    },
    SelectConfiguration {
        workspace: WorkspaceKey,
        identity: ConfigIdentity,
        discovery: u64,
    },
    SelectEngine {
        workspace: WorkspaceKey,
        engine: strop_containers::devcontainer::CliEngine,
        discovery: u64,
    },
    Approve {
        workspace: WorkspaceKey,
        identity: ConfigIdentity,
        engine: EngineIdentity,
        discovery: u64,
    },
    CloseReview {
        workspace: WorkspaceKey,
        discovery: u64,
    },
    ProbeCli {
        workspace: WorkspaceKey,
        discovery: u64,
    },
    ReadConfiguration {
        workspace: WorkspaceKey,
        identity: ConfigIdentity,
        engine: EngineIdentity,
        discovery: u64,
        merged: bool,
    },
    Lifecycle {
        workspace: WorkspaceKey,
        identity: ConfigIdentity,
        engine: EngineIdentity,
        discovery: u64,
        kind: OperationKind,
    },
    CancelOperation {
        workspace: WorkspaceKey,
        discovery: u64,
        operation: OperationId,
    },
    InspectOutput {
        workspace: WorkspaceKey,
        discovery: u64,
        operation: OperationId,
    },
    Dirty {
        id: u64,
        choice: DirtyTransitionChoice,
    },
}

impl DevcontainerAction {
    pub(crate) fn title(&self) -> String {
        match self {
            Self::Review { .. } => "Review dev container configuration".into(),
            Self::Later { .. } => "Later for this exact configuration".into(),
            Self::SuppressWorkspace { .. } => {
                "Suppress dev container offers for this workspace".into()
            }
            Self::SelectConfiguration { identity, .. } => {
                format!("Select configuration {}", identity.config_path().display())
            }
            Self::SelectEngine { engine, .. } => match engine.identity() {
                EngineIdentity::DefaultLocal => "Use the default local container engine".into(),
                EngineIdentity::Pinned { server_version, .. } => {
                    format!("Use inspected container engine (server {server_version})")
                }
            },
            Self::Approve { .. } => "Approve this digest and its displayed effects".into(),
            Self::CloseReview { .. } => "Close dev container review".into(),
            Self::ProbeCli { .. } => "Probe the optional reference CLI (never install)".into(),
            Self::ReadConfiguration { merged: false, .. } => {
                "Read plain configuration through the reference CLI".into()
            }
            Self::ReadConfiguration { merged: true, .. } => {
                "Consent to image/Feature resolution and read merged configuration".into()
            }
            Self::Lifecycle {
                kind: OperationKind::Attach,
                ..
            } => "Attach to the selected dev container".into(),
            Self::Lifecycle {
                kind: OperationKind::Start,
                ..
            } => "Start the approved dev container (frozen lockfile)".into(),
            Self::Lifecycle {
                kind: OperationKind::Rebuild,
                ..
            } => "Rebuild the owned dev container (frozen lockfile)".into(),
            Self::Lifecycle { .. } => "Run dev container lifecycle".into(),
            Self::CancelOperation { .. } => "Cancel the active dev container operation".into(),
            Self::InspectOutput { .. } => "Inspect redacted dev container output".into(),
            Self::Dirty {
                choice: DirtyTransitionChoice::Save,
                ..
            } => "Save all dirty local buffers, then enter the container".into(),
            Self::Dirty {
                choice: DirtyTransitionChoice::Stay,
                ..
            } => "Stay in the original workspace and preserve unsaved work".into(),
            Self::Dirty {
                choice: DirtyTransitionChoice::Cancel,
                ..
            } => "Cancel the container workspace transition".into(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OperationKey {
    pub id: OperationId,
    pub workspace: WorkspaceKey,
    pub identity: ConfigIdentity,
    pub discovery: u64,
    pub kind: OperationKind,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum OperationError {
    #[error(transparent)]
    Devcontainer(#[from] DevcontainerError),
    #[error("devcontainer operation cancelled")]
    Cancelled,
    #[error("devcontainer worker panicked")]
    WorkerPanic,
    #[error("devcontainer worker protocol failed: {0}")]
    Protocol(String),
    #[error("could not start devcontainer worker: {0}")]
    ThreadStart(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OperationPhase {
    Running,
    CancellationRequested,
    Succeeded,
    Failed,
    Cancelled,
}
impl OperationPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Bounded, redacted text suitable for constructing a real read-only editor
/// buffer. It never contains raw process output.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct OutputBuffer {
    text: String,
    dropped_log_bytes: usize,
}
impl OutputBuffer {
    pub fn update(&mut self, snapshot: &JobSnapshot) {
        self.text.clear();
        self.dropped_log_bytes = snapshot.dropped_log_bytes;
        for log in &snapshot.logs {
            self.text.push_str(&log.text);
        }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn dropped_log_bytes(&self) -> usize {
        self.dropped_log_bytes
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OperationResult {
    pub job: JobId,
    pub operation: CliOperation,
    pub state: JobState,
    pub context: Option<EnvironmentContext>,
    pub receipt: Option<OwnershipReceipt>,
    pub partial_resource: Option<PartialResource>,
    pub reconciliation: Reconciliation,
    pub error: Option<DevcontainerError>,
}
impl From<JobOutcome> for OperationResult {
    fn from(outcome: JobOutcome) -> Self {
        Self {
            job: outcome.id,
            operation: outcome.operation,
            state: outcome.state,
            context: outcome.context,
            receipt: outcome.receipt,
            partial_resource: outcome.partial_resource,
            reconciliation: outcome.reconciliation,
            error: outcome.error,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum OperationOutcome {
    Probe(Result<CliVersion, OperationError>),
    Read {
        merged: bool,
        result: Result<(), OperationError>,
    },
    Lifecycle(Result<Box<OperationResult>, OperationError>),
}
impl OperationOutcome {
    fn phase(&self) -> OperationPhase {
        match self {
            Self::Probe(Ok(_)) | Self::Read { result: Ok(()), .. } => OperationPhase::Succeeded,
            Self::Probe(Err(OperationError::Cancelled))
            | Self::Read {
                result: Err(OperationError::Cancelled),
                ..
            }
            | Self::Lifecycle(Err(OperationError::Cancelled)) => OperationPhase::Cancelled,
            Self::Probe(Err(_)) | Self::Read { result: Err(_), .. } | Self::Lifecycle(Err(_)) => {
                OperationPhase::Failed
            }
            Self::Lifecycle(Ok(result)) => match result.state {
                JobState::Succeeded => OperationPhase::Succeeded,
                JobState::Cancelled => OperationPhase::Cancelled,
                _ => OperationPhase::Failed,
            },
        }
    }
}

#[derive(Debug)]
pub struct OperationStatus<'a> {
    pub key: &'a OperationKey,
    pub phase: OperationPhase,
    pub cancellation_requested: bool,
    pub output: &'a OutputBuffer,
    pub outcome: Option<&'a OperationOutcome>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OperationPublication {
    pub key: OperationKey,
    pub phase: OperationPhase,
    pub cancellation_requested: bool,
    pub output: OutputBuffer,
    pub outcome: OperationOutcome,
}

pub(crate) enum OperationControl {
    Cli(Option<CancelHandle>),
    Lifecycle(strop_containers::devcontainer::JobControl),
}
pub(crate) struct ActiveOperation {
    pub key: OperationKey,
    pub control: OperationControl,
    pub cancellation_requested: bool,
    pub output: OutputBuffer,
}
impl ActiveOperation {
    pub fn refresh(&mut self) {
        if let OperationControl::Lifecycle(control) = &self.control {
            self.output.update(&control.snapshot());
        }
    }
    pub fn into_publication(self, outcome: OperationOutcome) -> OperationPublication {
        OperationPublication {
            phase: outcome.phase(),
            key: self.key,
            cancellation_requested: self.cancellation_requested,
            output: self.output,
            outcome,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CliAvailability {
    NotProbed,
    Ready { path: PathBuf, version: CliVersion },
    Unavailable(OperationError),
}

pub(crate) enum CliState {
    NotProbed,
    Ready(ReferenceCli),
    Unavailable(OperationError),
}
impl CliState {
    pub fn availability(&self) -> CliAvailability {
        match self {
            Self::NotProbed => CliAvailability::NotProbed,
            Self::Ready(cli) => CliAvailability::Ready {
                path: cli.path().to_path_buf(),
                version: cli.version(),
            },
            Self::Unavailable(error) => CliAvailability::Unavailable(error.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationReadAvailability {
    NotRequested,
    Running { merged: bool },
    Ready { merged: bool },
    Failed,
}

pub(crate) enum ConfigurationReadState {
    NotRequested,
    Running { merged: bool },
    Ready { merged: bool },
    Failed,
}
impl ConfigurationReadState {
    pub fn availability(&self) -> ConfigurationReadAvailability {
        match self {
            Self::NotRequested => ConfigurationReadAvailability::NotRequested,
            Self::Running { merged } => ConfigurationReadAvailability::Running { merged: *merged },
            Self::Ready { merged } => ConfigurationReadAvailability::Ready { merged: *merged },
            Self::Failed => ConfigurationReadAvailability::Failed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewSnapshot {
    pub identity: ConfigIdentity,
    pub trust: TrustReview,
    pub engine: EngineIdentity,
    pub approved: bool,
    pub cli: CliAvailability,
    pub read: ConfigurationReadAvailability,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DevcontainerOffer {
    pub workspace: WorkspaceKey,
    pub identity: ConfigIdentity,
    pub name: Option<String>,
    pub configuration_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscoverySummary {
    pub configurations: usize,
    pub issues: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum DevcontainerEvent {
    DiscoverySettled {
        workspace: WorkspaceKey,
        incarnation: u64,
        result: Result<DiscoverySummary, OperationError>,
        offer: Option<Box<DevcontainerOffer>>,
    },
    OperationSettled(Arc<OperationPublication>),
}

pub(crate) enum DiscoveryState {
    Idle,
    Running { id: u64 },
    Ready { id: u64 },
    Failed,
    Cancelled,
}

#[derive(Default)]
pub(crate) struct OfferDecisions {
    pub later: HashSet<ConfigIdentity>,
    pub workspace_suppressed: HashSet<ConfigIdentity>,
}

pub(crate) struct WorkspaceState {
    pub canonical_root: Option<PathBuf>,
    pub discovery: DiscoveryState,
    pub discovery_cancel: Option<CancelHandle>,
    pub report: Option<DiscoveryReport>,
    pub selected: Option<ConfigIdentity>,
    pub review_open: bool,
    pub approved: Option<ConfigIdentity>,
    pub engine: strop_containers::devcontainer::CliEngine,
    pub cli: CliState,
    pub read: ConfigurationReadState,
    pub configuration: Option<CliConfiguration>,
    pub active: Option<ActiveOperation>,
    pub last: Option<Arc<OperationPublication>>,
    pub ownership: Option<OwnershipReceipt>,
    pub decisions: OfferDecisions,
}
impl WorkspaceState {
    pub fn new() -> Self {
        Self {
            canonical_root: None,
            discovery: DiscoveryState::Idle,
            discovery_cancel: None,
            report: None,
            selected: None,
            review_open: false,
            approved: None,
            engine: strop_containers::devcontainer::CliEngine::DefaultLocal,
            cli: CliState::NotProbed,
            read: ConfigurationReadState::NotRequested,
            configuration: None,
            active: None,
            last: None,
            ownership: None,
            decisions: OfferDecisions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentIdentity {
    pub container_id: String,
    pub configuration: ConfigIdentity,
    pub job: JobId,
}
impl EnvironmentIdentity {
    pub fn of(context: &EnvironmentContext) -> Self {
        Self {
            container_id: context.container_id.clone(),
            configuration: context.identity.clone(),
            job: context.job,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransitionPermit {
    pub id: u64,
    pub workspace: WorkspaceKey,
    pub environment: EnvironmentContext,
    pub root: ResourceLocation,
    pub retire_previous: Option<EnvironmentIdentity>,
}

#[derive(Debug, Clone)]
pub struct DirtyTransitionPrompt {
    pub id: u64,
    pub workspace: WorkspaceKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyTransitionChoice {
    Save,
    Stay,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyTransitionResolution {
    SaveRequired(u64),
    Stayed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum TransitionRequest {
    Ready(Box<TransitionPermit>),
    Dirty(DirtyTransitionPrompt),
}

pub(crate) enum TransitionStage {
    AwaitingChoice,
    Saving,
    SaveFailed,
    Ready,
}
pub(crate) struct PendingTransition {
    pub permit: TransitionPermit,
    pub stage: TransitionStage,
}

#[derive(Debug, Clone)]
pub struct ActiveEnvironment {
    pub identity: EnvironmentIdentity,
    pub root: ResourceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DevcontainerStateError {
    #[error("devcontainer operation identity space exhausted")]
    IdentityExhausted,
    #[error("devcontainer discovery requires an exact local workspace")]
    NonLocalWorkspace,
    #[error("workspace has not completed devcontainer discovery")]
    UnknownWorkspace,
    #[error("no discovered devcontainer configuration is available")]
    NoConfiguration,
    #[error("selected configuration is not part of the current discovery incarnation")]
    StaleConfiguration,
    #[error("the current configuration digest must be reviewed and approved again")]
    ReviewRequired,
    #[error("the reference CLI is not ready for this workspace")]
    CliUnavailable,
    #[error("another devcontainer operation is still active")]
    OperationInProgress,
    #[error("there is no active devcontainer operation to cancel")]
    NoActiveOperation,
    #[error("there is no pending workspace transition")]
    NoPendingTransition,
    #[error("pending transition requires Save/Stay/Cancel before it can continue")]
    DirtyChoiceRequired,
    #[error("pending transition is waiting for the explicit save result")]
    SaveInProgress,
    #[error("could not encode the explicitly read configuration: {0}")]
    ConfigurationEncoding(String),
    #[error("saving failed; the original workspace remains active: {0}")]
    SaveFailed(String),
    #[error("environment does not verify the selected workspace/container mapping")]
    UnverifiedEnvironment,
    #[error("a previous owned environment receipt is required for rebuild")]
    OwnershipRequired,
    #[error(transparent)]
    Devcontainer(#[from] DevcontainerError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryDisposition {
    Started(u64),
    AlreadySettledOrRunning,
}
