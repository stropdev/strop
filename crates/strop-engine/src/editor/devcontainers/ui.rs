use super::model::*;
use super::presentation::DevcontainerMessage;
use super::{DevcontainerState, WorkspaceKey};
use crate::editor::feedback::{FeedbackKind, FeedbackSeverity};
use crate::editor::io::OpenIntent;
use crate::editor::{DocumentSource, Editor};
use std::collections::HashMap;
use std::sync::Arc;
use strop_containers::devcontainer::JobState;
use strop_core::id::{BufferRevision, DocumentId};
use strop_workspace::{ContainerId, Filesystem};

#[derive(Debug, Clone)]
enum ViewKind {
    Review,
    Output,
    Configuration { merged: bool },
}

#[derive(Debug, Clone)]
struct ViewBinding {
    workspace: WorkspaceKey,
    kind: ViewKind,
}

#[derive(Debug, Clone)]
struct SavingDocument {
    id: DocumentId,
    revision: BufferRevision,
}

#[derive(Debug, Clone)]
struct TransitionSave {
    id: u64,
    documents: Vec<SavingDocument>,
}

#[derive(Default)]
pub(crate) struct DevcontainerUiState {
    views: HashMap<DocumentId, ViewBinding>,
    review: Option<DocumentId>,
    pub(super) read_focus: Option<OperationId>,
    saving: Option<TransitionSave>,
}

impl DevcontainerState {
    pub(crate) fn document_workspace(&self, document: DocumentId) -> Option<WorkspaceKey> {
        self.ui
            .views
            .get(&document)
            .map(|binding| binding.workspace.clone())
    }
}

impl Editor {
    /// Drain discovery and lifecycle facts, then project them into the one
    /// owner-scoped feedback store and any explicitly opened real buffers.
    /// Passive discovery never focuses a view or invokes the CLI.
    pub(crate) fn handle_devcontainer_events(&mut self) {
        let events = self.devcontainers.poll();
        self.apply_devcontainer_events(events);
    }

    /// Apply settled facts from either a live wake or its recorded payload.
    /// This handler deliberately has no native channel/job polling path.
    pub(crate) fn apply_devcontainer_events(&mut self, events: Vec<DevcontainerEvent>) {
        self.prune_devcontainer_views();
        for event in events {
            match event {
                DevcontainerEvent::DiscoverySettled {
                    workspace,
                    incarnation,
                    result,
                    offer,
                } => self.handle_devcontainer_discovery(workspace, incarnation, result, offer),
                DevcontainerEvent::OperationSettled(publication) => {
                    self.handle_devcontainer_operation(publication)
                }
            }
        }

        let workspaces: Vec<_> = self.devcontainers.workspaces.keys().cloned().collect();
        for workspace in workspaces {
            let active = self
                .devcontainers
                .operation_status(&workspace)
                .is_some_and(|status| !status.phase.is_terminal());
            if active {
                self.publish_devcontainer_operation(&workspace);
            }
        }
        self.refresh_devcontainer_views();
        self.poll_devcontainer_transition_save();
    }

    fn prune_devcontainer_views(&mut self) {
        let closed: Vec<_> = self
            .devcontainers
            .ui
            .views
            .keys()
            .copied()
            .filter(|document| self.docs.get(*document).is_none())
            .collect();
        for document in closed {
            if let Some(binding) = self.devcontainers.ui.views.remove(&document) {
                if matches!(binding.kind, ViewKind::Review)
                    && self.devcontainers.ui.review == Some(document)
                {
                    self.devcontainers.ui.review = None;
                    let _ = self.devcontainers.close_review(&binding.workspace);
                }
            }
        }
    }

    fn handle_devcontainer_discovery(
        &mut self,
        workspace: WorkspaceKey,
        incarnation: u64,
        result: Result<DiscoverySummary, OperationError>,
        offer: Option<Box<DevcontainerOffer>>,
    ) {
        match result {
            Ok(summary) => {
                if let Some(offer) = offer {
                    self.publish_devcontainer_offer_at(*offer, incarnation);
                } else if summary.configurations > 0 || summary.issues > 0 {
                    // A workspace with no devcontainer at all is the common
                    // case and says nothing (0088 U13); only a discovery that
                    // found something, or tripped over something, reports.
                    let detail = format!(
                        "Workspace {}\nConfigurations discovered: {}\nDiscovery issues: {}\nNo offer is active: it was deferred, suppressed, already under review, or no configuration was found.",
                        workspace.root.display(), summary.configurations, summary.issues
                    );
                    let actions = self.devcontainer_actions(&workspace, incarnation);
                    self.publish_devcontainer_record(
                        workspace,
                        incarnation,
                        FeedbackKind::DevcontainerOperation,
                        FeedbackSeverity::Info,
                        DevcontainerMessage::new(
                            "Dev container discovery completed".into(),
                            detail,
                        ),
                        actions,
                    );
                }
            }
            Err(error) => {
                self.publish_devcontainer_record(
                    workspace.clone(),
                    incarnation,
                    FeedbackKind::DevcontainerOperation,
                    if error == OperationError::Cancelled {
                        FeedbackSeverity::Warning
                    } else {
                        FeedbackSeverity::Failure
                    },
                    DevcontainerMessage::new(
                        if error == OperationError::Cancelled {
                            "Dev container discovery cancelled".into()
                        } else {
                            "Dev container discovery failed".into()
                        },
                        format!("Workspace {}\n{error}", workspace.root.display()),
                    ),
                    Vec::new(),
                );
            }
        }
        self.refresh_devcontainer_views();
    }

    pub(super) fn publish_devcontainer_offer(&mut self, offer: DevcontainerOffer) {
        let Some(discovery) = self.devcontainers.discovery_incarnation(&offer.workspace) else {
            return;
        };
        self.publish_devcontainer_offer_at(offer, discovery);
    }

    fn publish_devcontainer_offer_at(&mut self, offer: DevcontainerOffer, discovery: u64) {
        let workspace = offer.workspace.clone();
        let detail = format!(
            "Workspace: {}\nConfiguration: {}\nName: {}\n{} configuration(s) discovered.\nReview is read-only and passive. Later applies only to this digest; Suppress applies to current configurations in this workspace.",
            workspace.root.display(),
            offer.identity.config_path().display(),
            offer.name.as_deref().unwrap_or("unnamed"),
            offer.configuration_count
        );
        self.publish_devcontainer_record(
            workspace.clone(),
            discovery,
            FeedbackKind::DevcontainerOffer,
            FeedbackSeverity::Info,
            DevcontainerMessage::new("Dev container configuration found".into(), detail),
            vec![
                DevcontainerAction::Review {
                    workspace: workspace.clone(),
                    identity: offer.identity.clone(),
                    discovery,
                },
                DevcontainerAction::Later {
                    workspace: workspace.clone(),
                    identity: offer.identity,
                    discovery,
                },
                DevcontainerAction::SuppressWorkspace {
                    workspace,
                    discovery,
                },
            ],
        );
    }

    fn handle_devcontainer_operation(&mut self, publication: Arc<OperationPublication>) {
        let workspace = publication.key.workspace.clone();
        let operation = publication.key.id;
        let merged = matches!(
            publication.key.kind,
            OperationKind::ReadConfiguration { merged: true }
        );
        let read_success = matches!(
            &publication.outcome,
            OperationOutcome::Read { result: Ok(()), .. }
        );
        let context = match &publication.outcome {
            OperationOutcome::Lifecycle(Ok(result)) if result.state == JobState::Succeeded => {
                result.context.clone()
            }
            _ => None,
        };
        self.publish_devcontainer_operation(&workspace);

        if read_success && self.devcontainers.ui.read_focus == Some(operation) {
            self.devcontainers.ui.read_focus = None;
            self.open_devcontainer_configuration(&workspace, merged);
        }
        if let Some(context) = context {
            self.begin_devcontainer_transition(&workspace, context);
        }
    }

    pub(super) fn publish_devcontainer_operation(&mut self, workspace: &WorkspaceKey) {
        let Some(discovery) = self.devcontainers.discovery_incarnation(workspace) else {
            return;
        };
        let Some(detail) = self.devcontainers.operation_text(workspace) else {
            return;
        };
        let Some(status) = self.devcontainers.operation_status(workspace) else {
            return;
        };
        let phase = status.phase;
        let kind = status.key.kind;
        let summary = match phase {
            OperationPhase::Running => format!("Dev container {} in progress", kind.label()),
            OperationPhase::CancellationRequested => {
                format!("Dev container {} cancellation requested", kind.label())
            }
            OperationPhase::Succeeded => format!("Dev container {} succeeded", kind.label()),
            OperationPhase::Failed => format!("Dev container {} failed", kind.label()),
            OperationPhase::Cancelled => format!("Dev container {} cancelled", kind.label()),
        };
        let severity = match phase {
            OperationPhase::Failed => FeedbackSeverity::Failure,
            OperationPhase::CancellationRequested | OperationPhase::Cancelled => {
                FeedbackSeverity::Warning
            }
            OperationPhase::Running | OperationPhase::Succeeded => FeedbackSeverity::Info,
        };
        let actions = self.devcontainer_actions(workspace, discovery);
        self.publish_devcontainer_record(
            workspace.clone(),
            discovery,
            FeedbackKind::DevcontainerOperation,
            severity,
            DevcontainerMessage::new(summary, detail),
            actions,
        );
    }

    pub(super) fn publish_devcontainer_review(&mut self, workspace: &WorkspaceKey) {
        let Some(discovery) = self.devcontainers.discovery_incarnation(workspace) else {
            return;
        };
        let approved = self
            .devcontainers
            .review(workspace)
            .is_some_and(|review| review.approved);
        let detail = self.devcontainers.review_text(workspace);
        let actions = self.devcontainer_actions(workspace, discovery);
        self.publish_devcontainer_record(
            workspace.clone(),
            discovery,
            FeedbackKind::DevcontainerOperation,
            FeedbackSeverity::Info,
            DevcontainerMessage::new(
                if approved {
                    "Dev container review approved".into()
                } else {
                    "Dev container review ready".into()
                },
                detail,
            ),
            actions,
        );
        self.refresh_devcontainer_views();
    }

    pub(super) fn open_devcontainer_review(&mut self, workspace: &WorkspaceKey) {
        let text = self.devcontainers.review_text(workspace);
        let existing = self.devcontainers.ui.review;
        if let Some(document) = existing.filter(|document| self.docs.get(*document).is_some()) {
            if let Err(error) = self.replace_system(document, &text) {
                self.message = format!("could not refresh dev container review: {error}");
                return;
            }
            self.switch_to(document);
            self.set_head(0);
        } else {
            let mut buffer = strop_core::Buffer::from_text(&text);
            buffer.name = Some("dev container trust review".into());
            let Some(document) = self.open_temporary_output(buffer) else {
                let _ = self.devcontainers.close_review(workspace);
                return;
            };
            self.devcontainers.ui.review = Some(document);
            self.devcontainers.ui.views.insert(
                document,
                ViewBinding {
                    workspace: workspace.clone(),
                    kind: ViewKind::Review,
                },
            );
        }
        self.publish_devcontainer_review(workspace);
    }

    pub(super) fn open_devcontainer_output(&mut self, workspace: &WorkspaceKey) {
        let Some(text) = self.devcontainers.operation_text(workspace) else {
            self.message = "dev container output is no longer current; reopen actions".into();
            return;
        };
        let existing = self
            .devcontainers
            .ui
            .views
            .iter()
            .find(|(document, binding)| {
                self.docs.get(**document).is_some()
                    && &binding.workspace == workspace
                    && matches!(binding.kind, ViewKind::Output)
            })
            .map(|(document, _)| *document);
        if let Some(document) = existing.filter(|document| self.docs.get(*document).is_some()) {
            if let Err(error) = self.replace_system(document, &text) {
                self.message = format!("could not refresh dev container output: {error}");
                return;
            }
            self.switch_to(document);
            self.set_head(0);
        } else {
            let mut buffer = strop_core::Buffer::from_text(&text);
            buffer.name = Some("dev container operation output".into());
            let Some(document) = self.open_temporary_output(buffer) else {
                return;
            };
            self.devcontainers.ui.views.insert(
                document,
                ViewBinding {
                    workspace: workspace.clone(),
                    kind: ViewKind::Output,
                },
            );
        }
    }

    fn open_devcontainer_configuration(&mut self, workspace: &WorkspaceKey, merged: bool) {
        let text = match self.devcontainers.configuration_text(workspace, merged) {
            Ok(text) => text,
            Err(error) => {
                self.message = error.to_string();
                return;
            }
        };
        let existing = self
            .devcontainers
            .ui
            .views
            .iter()
            .find(|(document, binding)| {
                self.docs.get(**document).is_some()
                    && &binding.workspace == workspace
                    && matches!(binding.kind, ViewKind::Configuration { merged: current } if current == merged)
            })
            .map(|(document, _)| *document);
        if let Some(document) = existing.filter(|document| self.docs.get(*document).is_some()) {
            if let Err(error) = self.replace_system(document, &text) {
                self.message = format!("could not refresh dev container configuration: {error}");
                return;
            }
            self.switch_to(document);
            self.set_head(0);
        } else {
            let mut buffer = strop_core::Buffer::from_text(&text);
            buffer.name = Some(format!(
                "dev container {} configuration",
                if merged { "merged" } else { "plain" }
            ));
            let Some(document) = self.open_temporary_output(buffer) else {
                return;
            };
            self.devcontainers.ui.views.insert(
                document,
                ViewBinding {
                    workspace: workspace.clone(),
                    kind: ViewKind::Configuration { merged },
                },
            );
        }
    }

    fn refresh_devcontainer_views(&mut self) {
        let views: Vec<_> = self
            .devcontainers
            .ui
            .views
            .iter()
            .map(|(document, binding)| (*document, binding.clone()))
            .collect();
        for (document, binding) in views {
            if self.docs.get(document).is_none() {
                continue;
            }
            let text = match binding.kind {
                ViewKind::Review => self.devcontainers.review_text(&binding.workspace),
                ViewKind::Output => match self.devcontainers.operation_text(&binding.workspace) {
                    Some(text) => text,
                    None => "This operation belongs to a stale discovery incarnation.\n".into(),
                },
                ViewKind::Configuration { merged } => match self
                    .devcontainers
                    .configuration_text(&binding.workspace, merged)
                {
                    Ok(text) => text,
                    Err(error) => format!("Configuration is no longer available: {error}\n"),
                },
            };
            if *self.doc(document).buf.text() != text {
                if let Err(error) = self.replace_system(document, &text) {
                    self.message = format!("could not refresh dev container view: {error}");
                }
            }
        }
    }

    fn dirty_local_documents(&self) -> Vec<(DocumentId, BufferRevision)> {
        self.docs
            .iter()
            .filter(|(_, document)| {
                document.buf.dirty
                    && matches!(
                        &document.source,
                        DocumentSource::File | DocumentSource::Scratch
                    )
            })
            .map(|(id, document)| (id, document.buf.revision()))
            .collect()
    }

    fn begin_devcontainer_transition(
        &mut self,
        workspace: &WorkspaceKey,
        context: strop_containers::devcontainer::EnvironmentContext,
    ) {
        let dirty = self.dirty_local_documents();
        match self
            .devcontainers
            .begin_transition(workspace, context, !dirty.is_empty())
        {
            Ok(TransitionRequest::Ready(permit)) => self.complete_devcontainer_transition(*permit),
            Ok(TransitionRequest::Dirty(prompt)) => self.publish_devcontainer_dirty(prompt, dirty),
            Err(error) => self.devcontainer_action_failed(
                workspace.clone(),
                self.devcontainers
                    .discovery_incarnation(workspace)
                    .unwrap_or(0),
                error,
            ),
        }
    }

    fn publish_devcontainer_dirty(
        &mut self,
        prompt: DirtyTransitionPrompt,
        dirty: Vec<(DocumentId, BufferRevision)>,
    ) {
        let discovery = self
            .devcontainers
            .discovery_incarnation(&prompt.workspace)
            .unwrap_or(0);
        let root = self
            .devcontainers
            .pending_transition()
            .filter(|permit| permit.id == prompt.id)
            .map(|permit| permit.root.label())
            .unwrap_or_else(|| "stale transition".into());
        let documents = dirty
            .iter()
            .filter_map(|(id, _)| self.docs.get(*id).map(|document| document.label(&self.cwd)))
            .collect::<Vec<_>>()
            .join(", ");
        let detail = format!(
            "Verified container workspace: {root}\nDirty local buffers: {documents}\nSave uses the ordinary protected save path for every buffer and enters only after every save succeeds. Stay and Cancel preserve the original workspace and all unsaved work."
        );
        let actions = self.devcontainer_actions(&prompt.workspace, discovery);
        self.publish_devcontainer_record(
            prompt.workspace,
            discovery,
            FeedbackKind::DevcontainerOperation,
            FeedbackSeverity::Warning,
            DevcontainerMessage::new(
                "Dev container workspace ready — dirty buffers require a choice".into(),
                detail,
            ),
            actions,
        );
    }

    pub(super) fn resolve_devcontainer_dirty(&mut self, id: u64, choice: DirtyTransitionChoice) {
        let Some(permit) = self.devcontainers.pending_transition().cloned() else {
            self.message = "dev container transition is no longer pending".into();
            return;
        };
        if permit.id != id {
            self.message = "dev container dirty choice is stale".into();
            return;
        }
        let workspace = permit.workspace.clone();
        let discovery = self
            .devcontainers
            .discovery_incarnation(&workspace)
            .unwrap_or(0);
        match self.devcontainers.resolve_dirty_transition(choice) {
            Ok(DirtyTransitionResolution::SaveRequired(id)) => {
                self.save_devcontainer_transition(id, workspace, discovery)
            }
            Ok(DirtyTransitionResolution::Stayed | DirtyTransitionResolution::Cancelled) => {
                self.message = "original workspace and unsaved work preserved".into();
                let actions = self.devcontainer_actions(&workspace, discovery);
                self.publish_devcontainer_record(
                    workspace,
                    discovery,
                    FeedbackKind::DevcontainerOperation,
                    FeedbackSeverity::Info,
                    DevcontainerMessage::new(
                        "Dev container transition not taken".into(),
                        "The original workspace remains active. No buffer was saved, closed or retargeted; the verified container was not stopped or removed.".into(),
                    ),
                    actions,
                );
            }
            Err(error) => self.devcontainer_action_failed(workspace, discovery, error),
        }
    }

    fn save_devcontainer_transition(&mut self, id: u64, workspace: WorkspaceKey, discovery: u64) {
        let documents: Vec<_> = self
            .dirty_local_documents()
            .into_iter()
            .map(|(id, revision)| SavingDocument { id, revision })
            .collect();
        if documents.is_empty() {
            match self.devcontainers.complete_transition_save(id, Ok(())) {
                Ok(permit) => self.complete_devcontainer_transition(permit),
                Err(error) => self.devcontainer_action_failed(workspace, discovery, error),
            }
            return;
        }
        let mut failed = None;
        for document in &documents {
            if !self.request_save_document(document.id, None, false, false) {
                failed = Some(format!(
                    "save was refused for {}",
                    self.docs
                        .get(document.id)
                        .map(|document| document.label(&self.cwd))
                        .unwrap_or_else(|| format!("closed document {:?}", document.id))
                ));
                break;
            }
        }
        if let Some(error) = failed {
            let _ = self
                .devcontainers
                .complete_transition_save(id, Err(error.clone()));
            self.devcontainer_action_failed(
                workspace,
                discovery,
                DevcontainerStateError::SaveFailed(error),
            );
            return;
        }
        self.devcontainers.ui.saving = Some(TransitionSave { id, documents });
        self.message = "saving all dirty local buffers before entering the container".into();
    }

    fn poll_devcontainer_transition_save(&mut self) {
        let Some(saving) = self.devcontainers.ui.saving.clone() else {
            return;
        };
        if self
            .devcontainers
            .pending_transition()
            .map(|permit| permit.id)
            != Some(saving.id)
        {
            self.devcontainers.ui.saving = None;
            return;
        }
        let mut waiting = false;
        let mut failure = None;
        for saved in &saving.documents {
            let Some(document) = self.docs.get(saved.id) else {
                failure = Some("a buffer closed before its save completed".into());
                break;
            };
            if self.io.save_pending_for(saved.id)
                || matches!(self.lsp_state.after_format.as_ref(),
                    Some(crate::editor::lsp::state::AfterFormat::Save { document, .. }) if *document == saved.id)
            {
                waiting = true;
                continue;
            }
            if document.buf.dirty {
                failure = Some(if document.buf.revision() == saved.revision {
                    format!("save failed for {}", document.label(&self.cwd))
                } else {
                    format!(
                        "{} changed while saving; preserve and save the newer edits explicitly",
                        document.label(&self.cwd)
                    )
                });
                break;
            }
        }
        if waiting {
            return;
        }
        let workspace = self
            .devcontainers
            .pending_transition()
            .map(|permit| permit.workspace.clone());
        let Some(workspace) = workspace else {
            self.devcontainers.ui.saving = None;
            return;
        };
        let discovery = self
            .devcontainers
            .discovery_incarnation(&workspace)
            .unwrap_or(0);
        self.devcontainers.ui.saving = None;
        match failure {
            Some(error) => {
                let _ = self
                    .devcontainers
                    .complete_transition_save(saving.id, Err(error.clone()));
                self.devcontainer_action_failed(
                    workspace,
                    discovery,
                    DevcontainerStateError::SaveFailed(error),
                );
            }
            None => match self
                .devcontainers
                .complete_transition_save(saving.id, Ok(()))
            {
                Ok(permit) => self.complete_devcontainer_transition(permit),
                Err(error) => self.devcontainer_action_failed(workspace, discovery, error),
            },
        }
    }

    fn complete_devcontainer_transition(&mut self, permit: TransitionPermit) {
        let workspace = permit.workspace.clone();
        let discovery = self
            .devcontainers
            .discovery_incarnation(&workspace)
            .unwrap_or(0);
        if let Err(error) = self.select_root(permit.root.clone()) {
            let _ = self.devcontainers.abort_transition(permit.id);
            self.publish_devcontainer_record(
                workspace,
                discovery,
                FeedbackKind::DevcontainerOperation,
                FeedbackSeverity::Failure,
                DevcontainerMessage::new(
                    "Dev container workspace selection failed".into(),
                    error.to_string(),
                ),
                Vec::new(),
            );
            return;
        }
        let active = match self.devcontainers.commit_transition(permit.clone()) {
            Ok(active) => active,
            Err(error) => {
                self.devcontainer_action_failed(workspace, discovery, error);
                return;
            }
        };
        let Filesystem::Container(container) = &active.root.filesystem else {
            self.message = "verified dev container root was not a container namespace".into();
            return;
        };
        self.attach_container_target(
            container.clone(),
            active.root.path.clone(),
            OpenIntent::Browse,
        );
        if let Some(previous) = &permit.retire_previous {
            self.retire_devcontainer_environment(&previous.container_id);
        }
        let detail = format!(
            "Entered verified container {} at exact root {}.\nDeferred lifecycle stages: {:?}.\nThe previous owned incarnation had only its editor services, write permits and terminals retired; no container or volume was stopped or removed.",
            permit.environment.container_id,
            active.root.label(),
            permit.environment.readiness.deferred_stages
        );
        let actions = self.devcontainer_actions(&workspace, discovery);
        self.publish_devcontainer_record(
            workspace,
            discovery,
            FeedbackKind::DevcontainerOperation,
            FeedbackSeverity::Info,
            DevcontainerMessage::new("Dev container workspace ready".into(), detail),
            actions,
        );
        self.message = "dev container workspace selected; attaching its exact verified root".into();
    }

    fn retire_devcontainer_environment(&mut self, container: &str) {
        let Ok(container_id) = ContainerId::canonical(container.to_owned()) else {
            return;
        };
        let filesystem = Filesystem::Container(container_id.clone());
        let terminals: Vec<_> = self
            .docs
            .iter()
            .filter_map(|(document, _)| {
                self.terminal_document(document).and_then(|terminal| {
                    (terminal.launch.filesystem == filesystem).then_some(terminal.session)
                })
            })
            .collect();
        for terminal in terminals {
            self.stop_terminal_session(terminal);
        }
        let documents: Vec<_> = self
            .docs
            .iter()
            .filter(|(_, document)| {
                matches!(&document.source, DocumentSource::Container { container, .. } if container == &container_id)
            })
            .map(|(document, _)| document)
            .collect();
        for document in documents {
            self.revoke_container_write(document);
        }
        self.containers.attached.remove(container);
        self.containers.workers.close_container(container);
        self.workspaces.note_disconnect(&filesystem);
    }
}
