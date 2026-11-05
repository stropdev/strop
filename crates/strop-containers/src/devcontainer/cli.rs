use super::config::{LifecycleStage, WaitFor};
use super::job::{CliOperation, JobId, OperationFailure, OperationSuccess, OwnedJob};
use super::model::*;
use super::process::{
    process_error, CliProcess, ProcessOutput, ProcessRequest, StreamRedactor, SupervisedCliProcess,
};
use super::DevcontainerError;
use serde::Deserialize;
use serde_json::Value;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use strop_core::worker::CancelToken;

const VERSION_LIMIT: usize = 4 * 1024;
const CONFIGURATION_LIMIT: usize = 8 * 1024 * 1024;
const RESULT_LIMIT: usize = 1024 * 1024;
const VERSION_DEADLINE: Duration = Duration::from_secs(10);
const CONFIGURATION_DEADLINE: Duration = Duration::from_secs(60);
pub const SUPPORTED_VERSIONS: &str = "0.87.0 through 0.90.x";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CliVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}
impl CliVersion {
    pub fn parse(text: &str) -> Result<Self, DevcontainerError> {
        let mut components = text.trim().split('.');
        let (Some(major), Some(minor), Some(patch), None) = (
            components.next(),
            components.next(),
            components.next(),
            components.next(),
        ) else {
            return Err(DevcontainerError::VersionProtocol);
        };
        let parse = |component: &str| {
            if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            component.parse().ok()
        };
        Ok(Self {
            major: parse(major).ok_or(DevcontainerError::VersionProtocol)?,
            minor: parse(minor).ok_or(DevcontainerError::VersionProtocol)?,
            patch: parse(patch).ok_or(DevcontainerError::VersionProtocol)?,
        })
    }
    pub fn is_supported(self) -> bool {
        self.major == 0 && (87..=90).contains(&self.minor)
    }
}
impl fmt::Display for CliVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Locate an optional external tool, never install it. An explicit path is
/// used exactly; otherwise only PATH entries named `devcontainer` are considered.
pub fn locate_cli(explicit: Option<&Path>) -> Result<PathBuf, DevcontainerError> {
    if let Some(path) = explicit {
        return path
            .is_file()
            .then(|| path.to_path_buf())
            .ok_or(DevcontainerError::CliNotFound);
    }
    let paths = std::env::var_os("PATH").ok_or(DevcontainerError::CliNotFound)?;
    for directory in std::env::split_paths(&paths) {
        let candidate = directory.join("devcontainer");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(DevcontainerError::CliNotFound)
}

#[derive(Clone)]
pub struct ReferenceCli {
    path: PathBuf,
    version: CliVersion,
    runner: Arc<dyn CliProcess>,
    next_job: Arc<AtomicU64>,
}
impl fmt::Debug for ReferenceCli {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReferenceCli")
            .field("path", &self.path)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
impl ReferenceCli {
    pub fn probe(explicit: Option<&Path>, token: &CancelToken) -> Result<Self, DevcontainerError> {
        Self::probe_with_runner(explicit, Arc::new(SupervisedCliProcess), token)
    }

    /// Process-boundary injection for deterministic tests or an engine-owned
    /// process facility. Product behavior remains in this adapter.
    pub fn probe_with_runner(
        explicit: Option<&Path>,
        runner: Arc<dyn CliProcess>,
        token: &CancelToken,
    ) -> Result<Self, DevcontainerError> {
        let path = locate_cli(explicit)?;
        let request = ProcessRequest {
            operation: "version check",
            program: path.clone(),
            args: vec![OsString::from("--version")],
            deadline: VERSION_DEADLINE,
            stdout_limit: VERSION_LIMIT,
            log_stdout: false,
        };
        let redactor = StreamRedactor::new(Vec::new());
        let output = runner
            .run(&request, token, &mut |_| {})
            .map_err(|failure| process_error(&request, failure, &redactor))?;
        ensure_exit("version check", &output, &redactor)?;
        let text =
            String::from_utf8(output.stdout).map_err(|_| DevcontainerError::VersionProtocol)?;
        let found = CliVersion::parse(&text)?;
        if !found.is_supported() {
            return Err(DevcontainerError::UnsupportedVersion {
                found: found.to_string(),
                supported: SUPPORTED_VERSIONS.into(),
            });
        }
        Ok(Self {
            path,
            version: found,
            runner,
            next_job: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn version(&self) -> CliVersion {
        self.version
    }

    /// Structured reference-CLI read. This is an explicit engine/CLI operation,
    /// not passive discovery or pure `TrustReview`: the primary CLI performs a
    /// container lookup even without merged metadata. It never runs hooks/up.
    pub fn read_configuration(
        &self,
        selection: &ConfigSelection,
        token: &CancelToken,
    ) -> Result<CliConfiguration, DevcontainerError> {
        self.read_configuration_inner(selection, None, token)
    }

    /// Request image/Feature merged metadata. The consent boundary is explicit
    /// because the primary CLI may resolve image metadata through the engine or
    /// network. No discovery/review function calls this implicitly.
    pub fn read_merged_configuration(
        &self,
        selection: &ConfigSelection,
        _consent: MetadataResolutionConsent,
        token: &CancelToken,
    ) -> Result<CliConfiguration, DevcontainerError> {
        self.read_configuration_inner(selection, Some(_consent), token)
    }

    fn read_configuration_inner(
        &self,
        selection: &ConfigSelection,
        merged: Option<MetadataResolutionConsent>,
        token: &CancelToken,
    ) -> Result<CliConfiguration, DevcontainerError> {
        let mut args = vec![
            OsString::from("read-configuration"),
            OsString::from("--workspace-folder"),
            selection.identity.workspace_root().as_os_str().to_owned(),
            OsString::from("--config"),
            selection.identity.config_path().as_os_str().to_owned(),
            OsString::from("--log-format"),
            OsString::from("json"),
        ];
        args.extend(selection.engine.args());
        if merged.is_some() {
            args.push(OsString::from("--include-features-configuration"));
            args.push(OsString::from("--include-merged-configuration"));
        }
        let request = ProcessRequest {
            operation: "read-configuration",
            program: self.path.clone(),
            args,
            deadline: CONFIGURATION_DEADLINE,
            stdout_limit: CONFIGURATION_LIMIT,
            log_stdout: false,
        };
        let redactor = StreamRedactor::new(selection.configuration.sensitive_values());
        let output = self
            .runner
            .run(&request, token, &mut |_| {})
            .map_err(|failure| process_error(&request, failure, &redactor))?;
        ensure_exit("read-configuration", &output, &redactor)?;
        let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            DevcontainerError::CliProtocol {
                operation: "read-configuration".into(),
                detail: error.to_string(),
            }
        })?;
        let configuration =
            value
                .get("configuration")
                .cloned()
                .ok_or_else(|| DevcontainerError::CliProtocol {
                    operation: "read-configuration".into(),
                    detail: "missing configuration".into(),
                })?;
        Ok(CliConfiguration {
            configuration,
            features: value.get("featuresConfiguration").cloned(),
            merged: value.get("mergedConfiguration").cloned(),
        })
    }

    pub fn attach(&self, request: AttachRequest) -> Result<OwnedJob, DevcontainerError> {
        validate_selection(&request.selection)?;
        let expected = request.expected_container.clone();
        let process = self.operation_request(
            "attach",
            &request.selection,
            Some(LockfilePolicy::Disabled),
            request.deadline,
            CliOperation::Attach,
            request.ownership_label.as_deref(),
        );
        let id = self.allocate_job();
        Ok(OwnedJob::new(
            id,
            CliOperation::Attach,
            self.runner.clone(),
            process,
            request.selection.configuration.sensitive_values(),
            Reconciliation::AttachFailedExternalResourceRetained,
            Box::new(move |output, job| {
                let parsed =
                    parse_up_result(output, &request.selection, CliOperation::Attach, None)?;
                if let Some(expected) = &expected {
                    if &parsed.container_id != expected {
                        return Err(OperationFailure {
                            error: Box::new(DevcontainerError::OwnershipRefused {
                                detail:
                                    "reference CLI matched a different container than requested"
                                        .into(),
                            }),
                            reconciliation: Reconciliation::AttachFailedExternalResourceRetained,
                            partial_resource: None,
                        });
                    }
                }
                let context = environment_context(
                    job,
                    CliOperation::Attach,
                    &request.selection,
                    parsed,
                    None,
                );
                Ok(OperationSuccess {
                    context,
                    receipt: None,
                    reconciliation: Reconciliation::ExternalResourceNotOwned,
                })
            }),
        ))
    }

    pub fn start(&self, request: StartRequest) -> Result<OwnedJob, DevcontainerError> {
        validate_selection(&request.selection)?;
        let id = self.allocate_job();
        let ownership_label = OwnershipReceipt::new_label(id);
        let process = self.operation_request(
            "up/start",
            &request.selection,
            Some(request.lockfile),
            request.deadline,
            CliOperation::Start,
            Some(&ownership_label),
        );
        Ok(OwnedJob::new(
            id,
            CliOperation::Start,
            self.runner.clone(),
            process,
            request.selection.configuration.sensitive_values(),
            Reconciliation::CreationRequiresReconciliation,
            Box::new(move |output, job| {
                let parsed = parse_up_result(
                    output,
                    &request.selection,
                    CliOperation::Start,
                    Some(&ownership_label),
                )?;
                let provider = request.selection.configuration.kind;
                let context = environment_context(
                    job,
                    CliOperation::Start,
                    &request.selection,
                    parsed.clone(),
                    None,
                );
                let receipt = OwnershipReceipt {
                    job,
                    container_id: context.container_id.clone(),
                    identity: request.selection.identity.clone(),
                    engine: request.selection.engine.identity(),
                    coverage: OwnershipCoverage::for_provider(provider),
                    label: ownership_label.clone(),
                };
                Ok(OperationSuccess {
                    context,
                    receipt: Some(receipt),
                    reconciliation: Reconciliation::OwnedResourcesPresent,
                })
            }),
        ))
    }

    pub fn rebuild(&self, request: RebuildRequest) -> Result<OwnedJob, DevcontainerError> {
        validate_selection(&request.selection)?;
        let previous = request.previous.clone();
        let ownership_label = previous.label.clone();
        let process = self.operation_request(
            "up/rebuild",
            &request.selection,
            Some(request.lockfile),
            request.deadline,
            CliOperation::Rebuild,
            Some(&ownership_label),
        );
        let id = self.allocate_job();
        Ok(OwnedJob::new(
            id,
            CliOperation::Rebuild,
            self.runner.clone(),
            process,
            request.selection.configuration.sensitive_values(),
            Reconciliation::RebuildRequiresReconciliation,
            Box::new(move |output, job| {
                let parsed = parse_up_result(
                    output,
                    &request.selection,
                    CliOperation::Rebuild,
                    Some(&ownership_label),
                )?;
                if parsed.container_id == previous.container_id {
                    return Err(OperationFailure {
                        error: Box::new(DevcontainerError::CliProtocol {
                            operation: "up/rebuild".into(),
                            detail: "rebuild returned the old container incarnation".into(),
                        }),
                        reconciliation: Reconciliation::RebuildRequiresReconciliation,
                        partial_resource: None,
                    });
                }
                let provider = request.selection.configuration.kind;
                let context = environment_context(
                    job,
                    CliOperation::Rebuild,
                    &request.selection,
                    parsed,
                    Some(previous.container_id.clone()),
                );
                let receipt = OwnershipReceipt {
                    job,
                    container_id: context.container_id.clone(),
                    identity: request.selection.identity.clone(),
                    engine: request.selection.engine.identity(),
                    coverage: OwnershipCoverage::for_provider(provider),
                    label: ownership_label.clone(),
                };
                Ok(OperationSuccess {
                    context,
                    receipt: Some(receipt),
                    reconciliation: Reconciliation::ReplacedOwned {
                        previous_container_id: previous.container_id.clone(),
                    },
                })
            }),
        ))
    }

    fn allocate_job(&self) -> JobId {
        JobId(self.next_job.fetch_add(1, Ordering::Relaxed))
    }

    fn operation_request(
        &self,
        operation: &'static str,
        selection: &ConfigSelection,
        lockfile: Option<LockfilePolicy>,
        deadline: Duration,
        kind: CliOperation,
        ownership_label: Option<&str>,
    ) -> ProcessRequest {
        let mut args = vec![
            OsString::from("up"),
            OsString::from("--workspace-folder"),
            selection.identity.workspace_root().as_os_str().to_owned(),
            OsString::from("--config"),
            selection.identity.config_path().as_os_str().to_owned(),
            OsString::from("--log-format"),
            OsString::from("json"),
            // The primary CLI defines this as stopping at waitFor (default
            // updateContentCommand), rather than waiting for later hooks.
            OsString::from("--skip-non-blocking-commands"),
        ];
        args.extend(selection.engine.args());
        for label in selection.identity.labels() {
            args.push(OsString::from("--id-label"));
            args.push(OsString::from(label));
        }
        if let Some(label) = ownership_label {
            args.push(OsString::from("--id-label"));
            args.push(OsString::from(format!("strop.devcontainer.owner={label}")));
        }
        match lockfile {
            Some(LockfilePolicy::Frozen) => args.push(OsString::from("--frozen-lockfile")),
            Some(LockfilePolicy::Disabled) => args.push(OsString::from("--no-lockfile")),
            Some(LockfilePolicy::GenerateOrUpdate) | None => {}
        }
        if kind == CliOperation::Attach {
            args.push(OsString::from("--expect-existing-container"));
        }
        if kind == CliOperation::Rebuild {
            args.push(OsString::from("--remove-existing-container"));
        }
        ProcessRequest {
            operation,
            program: self.path.clone(),
            args,
            deadline,
            stdout_limit: RESULT_LIMIT,
            log_stdout: true,
        }
    }
}

#[derive(Clone)]
struct ParsedUp {
    container_id: String,
    remote_user: String,
    remote_workspace: String,
}
#[derive(Deserialize)]
struct WireUp {
    outcome: String,
    #[serde(rename = "containerId")]
    container_id: Option<String>,
    #[serde(rename = "remoteUser")]
    remote_user: Option<String>,
    #[serde(rename = "remoteWorkspaceFolder")]
    remote_workspace: Option<String>,
    #[serde(rename = "didStopContainer")]
    did_stop_container: Option<bool>,
    description: Option<String>,
    message: Option<String>,
}

fn parse_up_result(
    output: &ProcessOutput,
    selection: &ConfigSelection,
    operation: CliOperation,
    ownership_label: Option<&str>,
) -> Result<ParsedUp, OperationFailure> {
    let redactor = StreamRedactor::new(selection.configuration.sensitive_values());
    let reconciliation = match operation {
        CliOperation::Attach => Reconciliation::AttachFailedExternalResourceRetained,
        CliOperation::Start => Reconciliation::CreationRequiresReconciliation,
        CliOperation::Rebuild => Reconciliation::RebuildRequiresReconciliation,
    };
    if output.code != Some(0) {
        let partial_resource = parse_wire(&output.stdout)
            .ok()
            .and_then(|wire| partial_resource(&wire, selection, operation, ownership_label));
        return Err(OperationFailure {
            error: Box::new(cli_output_error("up", output, &redactor)),
            reconciliation,
            partial_resource,
        });
    }
    let wire = parse_wire(&output.stdout).map_err(|error| OperationFailure {
        error: Box::new(DevcontainerError::CliProtocol {
            operation: "up".into(),
            detail: error,
        }),
        reconciliation: reconciliation.clone(),
        partial_resource: None,
    })?;
    let partial_resource = partial_resource(&wire, selection, operation, ownership_label);
    if wire.outcome != "success" {
        return Err(OperationFailure {
            error: Box::new(DevcontainerError::CliFailure {
                operation: "up".into(),
                code: output.code,
                detail: redactor.redact_complete(
                    wire.description
                        .as_deref()
                        .or(wire.message.as_deref())
                        .unwrap_or("reference CLI reported an error"),
                ),
            }),
            reconciliation,
            partial_resource,
        });
    }
    let result = ParsedUp {
        container_id: wire.container_id.clone().unwrap_or_default(),
        remote_user: wire.remote_user.clone().unwrap_or_default(),
        remote_workspace: wire.remote_workspace.clone().unwrap_or_default(),
    };
    let valid = validate_container_id(&result.container_id).is_ok()
        && !result.remote_user.is_empty()
        && !result.remote_workspace.is_empty();
    if !valid {
        return Err(OperationFailure {
            error: Box::new(DevcontainerError::CliProtocol {
                operation: "up".into(),
                detail: "success lacks canonical containerId, remoteUser, or remoteWorkspaceFolder"
                    .into(),
            }),
            reconciliation,
            partial_resource,
        });
    }
    Ok(result)
}

fn parse_wire(bytes: &[u8]) -> Result<WireUp, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "non-UTF-8 result".to_owned())?;
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    serde_json::from_str(line).map_err(|error| error.to_string())
}

fn partial_resource(
    wire: &WireUp,
    selection: &ConfigSelection,
    operation: CliOperation,
    ownership_label: Option<&str>,
) -> Option<PartialResource> {
    if operation == CliOperation::Attach {
        return None;
    }
    let container_id = wire
        .container_id
        .clone()
        .filter(|id| validate_container_id(id).is_ok())?;
    Some(PartialResource {
        container_id,
        cli_reported_stopped: wire.did_stop_container,
        coverage: OwnershipCoverage::for_provider(selection.configuration.kind),
        ownership_label: ownership_label.map(str::to_owned),
    })
}

fn environment_context(
    job: JobId,
    operation: CliOperation,
    selection: &ConfigSelection,
    result: ParsedUp,
    supersedes: Option<String>,
) -> EnvironmentContext {
    let required_stage = selection.configuration.wait_for;
    let candidates = [
        (
            WaitFor::OnCreateCommand,
            LifecycleStage::OnCreate,
            selection.configuration.on_create_command.is_some(),
        ),
        (
            WaitFor::UpdateContentCommand,
            LifecycleStage::UpdateContent,
            selection.configuration.update_content_command.is_some(),
        ),
        (
            WaitFor::PostCreateCommand,
            LifecycleStage::PostCreate,
            selection.configuration.post_create_command.is_some(),
        ),
    ];
    let required_rank = match required_stage {
        WaitFor::OnCreateCommand => 0,
        WaitFor::UpdateContentCommand => 1,
        WaitFor::PostCreateCommand => 2,
    };
    let deferred_stages = candidates
        .into_iter()
        .enumerate()
        .filter(|(rank, (_, _, configured))| *rank > required_rank && *configured)
        .map(|(_, (_, stage, _))| stage)
        .collect();
    EnvironmentContext {
        job,
        operation,
        identity: selection.identity.clone(),
        provider: selection.configuration.kind,
        compose_main_service: selection.configuration.service.clone(),
        compose_run_services: selection.configuration.run_services.clone(),
        engine: selection.engine.identity(),
        container_id: result.container_id,
        user: UserMapping {
            container_user: selection.configuration.container_user.clone(),
            remote_user: result.remote_user,
        },
        workspace: WorkspaceMapping {
            host_root: selection.identity.workspace_root().to_path_buf(),
            configured_folder: selection.configuration.workspace_folder.clone(),
            remote_folder: result.remote_workspace,
            mount: selection.configuration.workspace_mount.clone(),
        },
        readiness: Readiness {
            required_stage,
            deferred_stages,
        },
        supersedes,
    }
}

fn validate_selection(selection: &ConfigSelection) -> Result<(), DevcontainerError> {
    if selection.identity.workspace_root().to_str().is_none()
        || selection.identity.config_path().to_str().is_none()
    {
        return Err(DevcontainerError::UnsupportedConfiguration {
            detail: "reference-CLI identity labels require UTF-8 workspace/configuration paths"
                .into(),
        });
    }
    if selection.configuration.has_blocking_issues() {
        return Err(DevcontainerError::UnsupportedConfiguration {
            detail: "configuration has invalid types, missing provider properties, or conflicting providers".into(),
        });
    }
    Ok(())
}

fn ensure_exit(
    operation: &'static str,
    output: &ProcessOutput,
    redactor: &StreamRedactor,
) -> Result<(), DevcontainerError> {
    if output.code == Some(0) {
        Ok(())
    } else {
        Err(cli_output_error(operation, output, redactor))
    }
}

fn cli_output_error(
    operation: &'static str,
    output: &ProcessOutput,
    redactor: &StreamRedactor,
) -> DevcontainerError {
    let detail = String::from_utf8_lossy(&output.stderr_tail);
    DevcontainerError::CliFailure {
        operation: operation.into(),
        code: output.code,
        detail: redactor.redact_complete(detail.trim()),
    }
}
