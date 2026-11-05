use super::config::{ConfigurationKind, LifecycleStage, MountSpec, RawConfiguration, WaitFor};
use super::discovery::{ConfigIdentity, DiscoveredConfiguration};
use super::job::{CliOperation, JobId};
use super::DevcontainerError;
use crate::EngineRef;
use serde_json::Value;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) const OPERATION_DEADLINE: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone)]
pub struct ConfigSelection {
    pub identity: ConfigIdentity,
    pub configuration: RawConfiguration,
    pub engine: CliEngine,
}
impl ConfigSelection {
    pub fn new(discovered: &DiscoveredConfiguration, engine: CliEngine) -> Self {
        Self {
            identity: discovered.identity.clone(),
            configuration: discovered.configuration.clone(),
            engine,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CliEngine {
    DefaultLocal,
    Pinned(EngineRef),
}
impl CliEngine {
    pub(crate) fn args(&self) -> Vec<OsString> {
        match self {
            Self::DefaultLocal => Vec::new(),
            Self::Pinned(engine) => {
                let mut args = vec![OsString::from("--docker-path"), OsString::from("docker")];
                for argument in engine.context_args() {
                    args.push(OsString::from("--docker-arg"));
                    args.push(OsString::from(argument));
                }
                args
            }
        }
    }
    pub fn identity(&self) -> EngineIdentity {
        match self {
            Self::DefaultLocal => EngineIdentity::DefaultLocal,
            Self::Pinned(engine) => EngineIdentity::Pinned {
                server_version: engine.server_version().to_owned(),
                context: engine
                    .context_args()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EngineIdentity {
    DefaultLocal,
    Pinned {
        server_version: String,
        context: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockfilePolicy {
    /// Use existing `.devcontainer-lock.json` and fail rather than modify it.
    Frozen,
    /// Explicitly permit reference-CLI lockfile generation/update.
    GenerateOrUpdate,
    Disabled,
}

#[derive(Debug)]
pub struct AttachRequest {
    pub selection: ConfigSelection,
    pub expected_container: Option<String>,
    pub deadline: Duration,
    pub(crate) ownership_label: Option<String>,
}
impl AttachRequest {
    pub fn new(selection: ConfigSelection) -> Self {
        Self {
            selection,
            expected_container: None,
            deadline: OPERATION_DEADLINE,
            ownership_label: None,
        }
    }
    pub fn expect_container(mut self, canonical_id: &str) -> Result<Self, DevcontainerError> {
        validate_container_id(canonical_id)?;
        self.expected_container = Some(canonical_id.to_owned());
        Ok(self)
    }
    /// Reopen a resource whose unique ownership label was created by `start`.
    /// This both narrows the reference-CLI match and pins the expected id.
    pub fn owned(mut self, receipt: &OwnershipReceipt) -> Result<Self, DevcontainerError> {
        if !receipt.identity.same_location(&self.selection.identity)
            || receipt.engine != self.selection.engine.identity()
        {
            return Err(DevcontainerError::OwnershipRefused {
                detail:
                    "attach receipt belongs to a different workspace, configuration path, or engine"
                        .into(),
            });
        }
        self.expected_container = Some(receipt.container_id.clone());
        self.ownership_label = Some(receipt.label.clone());
        Ok(self)
    }
}

#[derive(Debug)]
pub struct StartRequest {
    pub selection: ConfigSelection,
    pub lockfile: LockfilePolicy,
    pub deadline: Duration,
}
impl StartRequest {
    pub fn new(selection: ConfigSelection, lockfile: LockfilePolicy) -> Self {
        Self {
            selection,
            lockfile,
            deadline: OPERATION_DEADLINE,
        }
    }
}

#[derive(Debug)]
pub struct RebuildRequest {
    pub selection: ConfigSelection,
    pub lockfile: LockfilePolicy,
    pub deadline: Duration,
    pub(crate) previous: OwnershipReceipt,
}
impl RebuildRequest {
    pub fn new(
        selection: ConfigSelection,
        lockfile: LockfilePolicy,
        previous: &OwnershipReceipt,
    ) -> Result<Self, DevcontainerError> {
        if !previous.identity.same_location(&selection.identity) {
            return Err(DevcontainerError::OwnershipRefused {
                detail: "rebuild receipt belongs to a different workspace/configuration path"
                    .into(),
            });
        }
        if previous.engine != selection.engine.identity() {
            return Err(DevcontainerError::OwnershipRefused {
                detail: "rebuild receipt belongs to a different engine".into(),
            });
        }
        Ok(Self {
            selection,
            lockfile,
            deadline: OPERATION_DEADLINE,
            previous: previous.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OwnershipCoverage {
    SingleContainer,
    /// The reference CLI reports the main container only. No sidecar identity
    /// is guessed and no sidecar cleanup API is exposed.
    ComposeMainOnlySidecarsNotEnumerated,
}
impl OwnershipCoverage {
    pub(crate) fn for_provider(provider: ConfigurationKind) -> Self {
        if provider == ConfigurationKind::Compose {
            Self::ComposeMainOnlySidecarsNotEnumerated
        } else {
            Self::SingleContainer
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OwnershipReceipt {
    pub(crate) job: JobId,
    pub(crate) container_id: String,
    pub(crate) identity: ConfigIdentity,
    pub(crate) engine: EngineIdentity,
    pub(crate) coverage: OwnershipCoverage,
    /// Private unique id-label required for rebuild/owned-attach matching.
    pub(crate) label: String,
}
impl OwnershipReceipt {
    pub(crate) fn new_label(job: JobId) -> String {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("strop-{}-{}-{}", std::process::id(), nonce, job.0)
    }
    pub fn job(&self) -> JobId {
        self.job
    }
    pub fn container_id(&self) -> &str {
        &self.container_id
    }
    pub fn identity(&self) -> &ConfigIdentity {
        &self.identity
    }
    pub fn engine(&self) -> &EngineIdentity {
        &self.engine
    }
    pub fn coverage(&self) -> OwnershipCoverage {
        self.coverage
    }
    pub(crate) fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PartialResource {
    pub container_id: String,
    pub cli_reported_stopped: Option<bool>,
    pub coverage: OwnershipCoverage,
    pub ownership_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Reconciliation {
    ExternalResourceNotOwned,
    AttachFailedExternalResourceRetained,
    OwnedResourcesPresent,
    CreationRequiresReconciliation,
    RebuildRequiresReconciliation,
    ReplacedOwned { previous_container_id: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserMapping {
    pub container_user: Option<String>,
    pub remote_user: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceMapping {
    pub host_root: PathBuf,
    pub configured_folder: Option<String>,
    pub remote_folder: String,
    pub mount: Option<MountSpec>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Readiness {
    pub required_stage: WaitFor,
    /// Creation stages configured after waitFor and deliberately deferred by
    /// `--skip-non-blocking-commands`; success never claims they finished.
    pub deferred_stages: Vec<LifecycleStage>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnvironmentContext {
    pub job: JobId,
    pub operation: CliOperation,
    pub identity: ConfigIdentity,
    pub provider: ConfigurationKind,
    pub compose_main_service: Option<String>,
    pub compose_run_services: Vec<String>,
    pub engine: EngineIdentity,
    pub container_id: String,
    pub user: UserMapping,
    pub workspace: WorkspaceMapping,
    pub readiness: Readiness,
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct MetadataResolutionConsent(());
impl MetadataResolutionConsent {
    pub fn explicit() -> Self {
        Self(())
    }
}

/// Structured CLI output retains JSON for inspection without deriving Debug
/// over potentially secret-bearing configuration values.
pub struct CliConfiguration {
    pub(crate) configuration: Value,
    pub(crate) features: Option<Value>,
    pub(crate) merged: Option<Value>,
}
impl fmt::Debug for CliConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CliConfiguration")
            .field("has_features", &self.features.is_some())
            .field("has_merged", &self.merged.is_some())
            .finish_non_exhaustive()
    }
}
impl CliConfiguration {
    pub fn configuration(&self) -> Result<RawConfiguration, DevcontainerError> {
        RawConfiguration::from_value(self.configuration.clone())
    }
    pub fn merged_configuration(&self) -> Option<Result<RawConfiguration, DevcontainerError>> {
        self.merged.clone().map(RawConfiguration::from_value)
    }
    pub fn features_json(&self) -> Option<&Value> {
        self.features.as_ref()
    }
}

pub(crate) fn validate_container_id(id: &str) -> Result<(), DevcontainerError> {
    if id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(DevcontainerError::OwnershipRefused {
            detail: "expected a canonical lowercase 64-hex container id".into(),
        })
    }
}
