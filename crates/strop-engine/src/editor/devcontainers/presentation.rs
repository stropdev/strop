use super::model::*;
use super::{DevcontainerState, WorkspaceKey};
use crate::editor::feedback::{
    FeedbackEvent, FeedbackKind, FeedbackOwner, FeedbackSeverity, TypedAction,
};
use crate::editor::Editor;
use std::fmt::Write;
use strop_containers::devcontainer::{
    CliEngine, ConfigurationKind, EngineIdentity, MountSpec, Reconciliation,
};

fn mount_mapping(mount: Option<&MountSpec>) -> String {
    match mount {
        Some(MountSpec::Object(object)) => format!(
            "type {}, target {}, source and options redacted",
            object.kind.as_deref().unwrap_or("unspecified"),
            object.target
        ),
        Some(MountSpec::String(_)) => "string mount; source, target and options redacted".into(),
        None => "engine/default mapping; no source value displayed".into(),
    }
}

fn provider(kind: ConfigurationKind) -> &'static str {
    match kind {
        ConfigurationKind::Image => "image",
        ConfigurationKind::Dockerfile => "Dockerfile/build",
        ConfigurationKind::Compose => "Compose",
        ConfigurationKind::Unsupported => "unsupported",
    }
}

fn engine(engine: &EngineIdentity) -> String {
    match engine {
        EngineIdentity::DefaultLocal => "default local engine".into(),
        EngineIdentity::Pinned {
            server_version,
            context,
        } => format!(
            "pinned inspected engine (server {server_version}, {} pinned context argument(s))",
            context.len()
        ),
    }
}

impl DevcontainerState {
    pub(crate) fn review_text(&self, key: &WorkspaceKey) -> String {
        let Some(review) = self.review(key) else {
            return "Dev container review is closed or its discovery is stale.\n".into();
        };
        let mut text = String::from(
            "Dev container trust review — read-only; Space a or Space o opens actions\n\
             No command, environment value, mount source or credential is displayed here.\n\n",
        );
        let _ = writeln!(text, "Workspace root: {}", key.root.display());
        let _ = writeln!(
            text,
            "Configuration: {}",
            review.identity.config_path().display()
        );
        let _ = writeln!(text, "Digest: {}", review.identity.digest());
        let _ = writeln!(text, "Provider: {}", provider(review.trust.provider));
        let _ = writeln!(text, "Engine: {}", engine(&review.engine));
        let _ = writeln!(
            text,
            "Container user: {}",
            review
                .trust
                .container_user
                .as_deref()
                .unwrap_or("image/default")
        );
        let _ = writeln!(
            text,
            "Remote user: {}",
            review
                .trust
                .remote_user
                .as_deref()
                .unwrap_or("container/default")
        );
        let _ = writeln!(
            text,
            "Container workspace folder: {}",
            review
                .trust
                .workspace_folder
                .as_deref()
                .unwrap_or("CLI/default")
        );
        let _ = writeln!(
            text,
            "Workspace mount: {}",
            mount_mapping(review.trust.workspace_mount.as_ref())
        );
        let _ = writeln!(text, "Readiness gate: {:?}", review.trust.wait_for);
        let _ = writeln!(text, "Approved for this digest: {}", review.approved);
        let cli = match &review.cli {
            CliAvailability::NotProbed => "not probed; no CLI or engine work has occurred".into(),
            CliAvailability::Ready { path, version } => {
                format!("ready at {} (supported version {version})", path.display())
            }
            CliAvailability::Unavailable(error) => format!(
                "unavailable: {error}. Strop never installs the CLI; supported versions are {}",
                strop_containers::devcontainer::SUPPORTED_VERSIONS
            ),
        };
        let _ = writeln!(text, "Reference CLI: {cli}");
        let _ = writeln!(text, "Configuration read: {:?}", review.read);

        if let Some(configurations) = self.configurations(key) {
            text.push_str("\n[configurations]\n");
            for configuration in configurations {
                let selected = configuration.identity == review.identity;
                let _ = writeln!(
                    text,
                    "  {}{} — {} ({})",
                    if selected { "selected " } else { "" },
                    configuration.identity.config_path().display(),
                    configuration
                        .configuration
                        .name
                        .as_deref()
                        .unwrap_or("unnamed"),
                    provider(configuration.configuration.kind)
                );
            }
        }

        text.push_str("\n[mount mappings — sources redacted]\n");
        if review.trust.mounts.is_empty() {
            text.push_str("  no additional mounts\n");
        }
        for (index, mount) in review.trust.mounts.iter().enumerate() {
            let _ = writeln!(text, "  {}: {}", index + 1, mount_mapping(Some(mount)));
        }

        text.push_str("\n[effects — command and secret values redacted]\n");
        for effect in &review.trust.effects {
            let _ = writeln!(
                text,
                "  {:?} / {:?} / {:?}: {}",
                effect.severity, effect.domain, effect.kind, effect.summary
            );
        }
        text.push_str(
            "\nAttach, Start and Rebuild are separate explicit actions. Plain read performs a CLI/engine lookup; merged read additionally consents to image/Feature metadata resolution.\n",
        );
        text
    }

    pub(crate) fn operation_text(&mut self, key: &WorkspaceKey) -> Option<String> {
        let status = self.operation_status(key)?;
        let mut text = format!(
            "Dev container {} — operation {} — {:?}\nCancellation requested: {}\n",
            status.key.kind.label(),
            status.key.id.0,
            status.phase,
            status.cancellation_requested
        );
        let _ = writeln!(
            text,
            "Workspace root: {}",
            status.key.workspace.root.display()
        );
        let _ = writeln!(
            text,
            "Configuration: {}",
            status.key.identity.config_path().display()
        );
        let _ = writeln!(text, "Discovery incarnation: {}", status.key.discovery);
        if status.cancellation_requested && !status.phase.is_terminal() {
            text.push_str("Cancellation remains requested until the worker actually settles.\n");
        }
        match status.outcome {
            Some(OperationOutcome::Probe(Ok(version))) => {
                let _ = writeln!(text, "Supported reference CLI version: {version}");
            }
            Some(OperationOutcome::Probe(Err(error))) => {
                let _ = writeln!(text, "CLI probe failed: {error}");
                text.push_str(
                    "No installation was attempted. Make a supported reference CLI available explicitly, then probe again.\n",
                );
            }
            Some(OperationOutcome::Read { merged, result }) => {
                let _ = writeln!(text, "Merged metadata: {merged}; result: {result:?}");
            }
            Some(OperationOutcome::Lifecycle(Ok(result))) => {
                let _ = writeln!(text, "Job: {:?}; state: {:?}", result.job, result.state);
                let _ = writeln!(text, "Reconciliation: {:?}", result.reconciliation);
                if let Some(partial) = &result.partial_resource {
                    let _ = writeln!(
                        text,
                        "Partial resource retained for reconciliation: container {}, CLI reported stopped {:?}, coverage {:?}",
                        partial.container_id, partial.cli_reported_stopped, partial.coverage
                    );
                }
                if let Some(error) = &result.error {
                    let _ = writeln!(text, "Lifecycle error: {error}");
                }
                if let Some(context) = &result.context {
                    let _ = writeln!(text, "Verified container: {}", context.container_id);
                    let _ = writeln!(
                        text,
                        "Verified container root: {}",
                        context.workspace.remote_folder
                    );
                    let _ = writeln!(
                        text,
                        "Host root used only for verification: {}",
                        context.workspace.host_root.display()
                    );
                    if !context.readiness.deferred_stages.is_empty() {
                        let _ = writeln!(
                            text,
                            "Deferred lifecycle stages (not claimed complete): {:?}",
                            context.readiness.deferred_stages
                        );
                    }
                }
                if matches!(
                    result.reconciliation,
                    Reconciliation::CreationRequiresReconciliation
                        | Reconciliation::RebuildRequiresReconciliation
                ) {
                    text.push_str(
                        "The partial resource was not stopped or removed by Strop; inspect and reconcile it explicitly.\n",
                    );
                }
            }
            Some(OperationOutcome::Lifecycle(Err(error))) => {
                let _ = writeln!(text, "Lifecycle worker failed: {error}");
            }
            None => text.push_str("Operation is still active; no outcome is claimed.\n"),
        }
        let _ = writeln!(
            text,
            "Redacted log bytes dropped by the bounded worker buffer: {}",
            status.output.dropped_log_bytes()
        );
        text.push_str("\n[redacted output]\n");
        if status.output.text().is_empty() {
            text.push_str("(no output)\n");
        } else {
            text.push_str(status.output.text());
            if !status.output.text().ends_with('\n') {
                text.push('\n');
            }
        }
        Some(text)
    }

    pub(crate) fn configuration_text(
        &self,
        key: &WorkspaceKey,
        merged: bool,
    ) -> Result<String, DevcontainerStateError> {
        let configuration = if merged {
            self.merged_configuration_value(key)?
        } else {
            self.read_configuration_value(key)?
        };
        let Some(configuration) = configuration else {
            return Ok("The requested configuration representation is not available.\n".into());
        };
        let body = serde_json::to_string_pretty(configuration.source())
            .map_err(|error| DevcontainerStateError::ConfigurationEncoding(error.to_string()))?;
        Ok(format!(
            "Dev container {} configuration — explicitly read through the reference CLI\n\
             This explicit inspection may contain configuration secrets; review effects remain redacted.\n\n{body}\n",
            if merged { "merged" } else { "plain" }
        ))
    }
}

pub(crate) struct DevcontainerMessage {
    summary: String,
    detail: String,
}
impl DevcontainerMessage {
    pub(crate) fn new(summary: String, detail: String) -> Self {
        Self { summary, detail }
    }
}

impl Editor {
    pub(crate) fn devcontainer_owner(
        &self,
        workspace: &WorkspaceKey,
        discovery: u64,
    ) -> FeedbackOwner {
        FeedbackOwner::Devcontainer {
            workspace: workspace.clone(),
            incarnation: discovery,
        }
    }

    pub(crate) fn publish_devcontainer_record(
        &mut self,
        workspace: WorkspaceKey,
        discovery: u64,
        kind: FeedbackKind,
        severity: FeedbackSeverity,
        message: DevcontainerMessage,
        actions: Vec<DevcontainerAction>,
    ) {
        let owner = self.devcontainer_owner(&workspace, discovery);
        if self.feedback.unresolved().any(|record| {
            record.owner() == &owner
                && record.kind == kind
                && record.summary() == message.summary
                && record.detail() == message.detail
        }) {
            return;
        }
        let mut actions: Vec<TypedAction> =
            actions.into_iter().map(TypedAction::Devcontainer).collect();
        actions.push(TypedAction::Dismiss);
        self.record(FeedbackEvent {
            severity,
            owner,
            kind,
            summary: message.summary,
            detail: message.detail,
            reference: None,
            actions,
        });
    }

    pub(crate) fn devcontainer_actions(
        &mut self,
        workspace: &WorkspaceKey,
        discovery: u64,
    ) -> Vec<DevcontainerAction> {
        if self.devcontainers.discovery_incarnation(workspace) != Some(discovery) {
            return Vec::new();
        }
        let pending_id = self
            .devcontainers
            .pending_transition()
            .filter(|permit| &permit.workspace == workspace)
            .map(|permit| permit.id);
        if let Some(id) = pending_id {
            let mut actions: Vec<_> = [
                DirtyTransitionChoice::Save,
                DirtyTransitionChoice::Stay,
                DirtyTransitionChoice::Cancel,
            ]
            .into_iter()
            .map(|choice| DevcontainerAction::Dirty { id, choice })
            .collect();
            if let Some(status) = self.devcontainers.operation_status(workspace) {
                actions.push(DevcontainerAction::InspectOutput {
                    workspace: workspace.clone(),
                    discovery,
                    operation: status.key.id,
                });
            }
            return actions;
        }

        let mut actions = Vec::new();
        let review = self.devcontainers.review(workspace);
        let Some(selected) = self.devcontainers.selected_configuration(workspace) else {
            return actions;
        };
        if review.is_none() {
            actions.push(DevcontainerAction::Review {
                workspace: workspace.clone(),
                identity: selected,
                discovery,
            });
        }
        if let Some(review) = &review {
            if let Some(configurations) = self.devcontainers.configurations(workspace) {
                actions.extend(configurations.iter().map(|configuration| {
                    DevcontainerAction::SelectConfiguration {
                        workspace: workspace.clone(),
                        identity: configuration.identity.clone(),
                        discovery,
                    }
                }));
            }
            for choice in self.devcontainer_engine_choices() {
                if choice.identity() != review.engine {
                    actions.push(DevcontainerAction::SelectEngine {
                        workspace: workspace.clone(),
                        engine: choice,
                        discovery,
                    });
                }
            }
            if !review.approved {
                actions.push(DevcontainerAction::Approve {
                    workspace: workspace.clone(),
                    identity: review.identity.clone(),
                    engine: review.engine.clone(),
                    discovery,
                });
            }
            actions.push(DevcontainerAction::CloseReview {
                workspace: workspace.clone(),
                discovery,
            });
            let active = self
                .devcontainers
                .operation_status(workspace)
                .is_some_and(|status| !status.phase.is_terminal());
            if !active {
                actions.push(DevcontainerAction::ProbeCli {
                    workspace: workspace.clone(),
                    discovery,
                });
                if matches!(review.cli, CliAvailability::Ready { .. }) {
                    actions.push(DevcontainerAction::ReadConfiguration {
                        workspace: workspace.clone(),
                        identity: review.identity.clone(),
                        engine: review.engine.clone(),
                        discovery,
                        merged: false,
                    });
                    actions.push(DevcontainerAction::ReadConfiguration {
                        workspace: workspace.clone(),
                        identity: review.identity.clone(),
                        engine: review.engine.clone(),
                        discovery,
                        merged: true,
                    });
                    if review.approved {
                        actions.extend(
                            [OperationKind::Attach, OperationKind::Start]
                                .into_iter()
                                .map(|kind| DevcontainerAction::Lifecycle {
                                    workspace: workspace.clone(),
                                    identity: review.identity.clone(),
                                    engine: review.engine.clone(),
                                    discovery,
                                    kind,
                                }),
                        );
                        if self.devcontainers.has_ownership(workspace) {
                            actions.push(DevcontainerAction::Lifecycle {
                                workspace: workspace.clone(),
                                identity: review.identity.clone(),
                                engine: review.engine.clone(),
                                discovery,
                                kind: OperationKind::Rebuild,
                            });
                        }
                    }
                }
            }
        }
        if let Some(status) = self.devcontainers.operation_status(workspace) {
            if !status.phase.is_terminal() {
                actions.push(DevcontainerAction::CancelOperation {
                    workspace: workspace.clone(),
                    discovery,
                    operation: status.key.id,
                });
            }
            actions.push(DevcontainerAction::InspectOutput {
                workspace: workspace.clone(),
                discovery,
                operation: status.key.id,
            });
        }
        actions
    }

    pub(crate) fn devcontainer_engine_choices(&self) -> Vec<CliEngine> {
        let mut engines = vec![CliEngine::DefaultLocal];
        engines.extend(
            self.containers
                .workers
                .engines()
                .into_iter()
                .map(CliEngine::Pinned),
        );
        engines
    }
}
