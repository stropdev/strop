use super::model::*;
use super::presentation::DevcontainerMessage;
use super::WorkspaceKey;
use crate::editor::feedback::{FeedbackKind, FeedbackSeverity};
use crate::editor::Editor;
use strop_containers::devcontainer::{LockfilePolicy, MetadataResolutionConsent};

impl Editor {
    pub(crate) fn devcontainer_action_valid(&self, action: &DevcontainerAction) -> bool {
        if let DevcontainerAction::Dirty { id, choice } = action {
            return self
                .devcontainers
                .pending_transition()
                .map(|permit| permit.id)
                == Some(*id)
                && (!matches!(choice, DirtyTransitionChoice::Save)
                    || !self.devcontainers.transition_save_in_progress());
        }
        let (workspace, discovery) = match action {
            DevcontainerAction::Review {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::Later {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::SuppressWorkspace {
                workspace,
                discovery,
            }
            | DevcontainerAction::SelectConfiguration {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::SelectEngine {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::Approve {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::CloseReview {
                workspace,
                discovery,
            }
            | DevcontainerAction::ProbeCli {
                workspace,
                discovery,
            }
            | DevcontainerAction::ReadConfiguration {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::Lifecycle {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::CancelOperation {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::InspectOutput {
                workspace,
                discovery,
                ..
            } => (workspace, *discovery),
            DevcontainerAction::Dirty { .. } => unreachable!(),
        };
        if self.devcontainers.discovery_incarnation(workspace) != Some(discovery) {
            return false;
        }
        let Some(state) = self.devcontainers.workspaces.get(workspace) else {
            return false;
        };
        let current_identity = |identity: &strop_containers::devcontainer::ConfigIdentity| {
            self.devcontainers
                .configurations(workspace)
                .is_some_and(|configurations| {
                    configurations
                        .iter()
                        .any(|configuration| &configuration.identity == identity)
                })
        };
        match action {
            DevcontainerAction::Review { identity, .. }
            | DevcontainerAction::Later { identity, .. } => current_identity(identity),
            DevcontainerAction::SuppressWorkspace { .. } => true,
            DevcontainerAction::SelectConfiguration { identity, .. } => {
                state.active.is_none() && current_identity(identity)
            }
            DevcontainerAction::SelectEngine { .. } | DevcontainerAction::ProbeCli { .. } => {
                state.active.is_none()
            }
            DevcontainerAction::Approve {
                identity, engine, ..
            } => self
                .devcontainers
                .review(workspace)
                .is_some_and(|review| &review.identity == identity && &review.engine == engine),
            DevcontainerAction::CloseReview { .. } => {
                self.devcontainers.review(workspace).is_some()
            }
            DevcontainerAction::ReadConfiguration {
                identity, engine, ..
            } => {
                state.active.is_none()
                    && self.devcontainers.review(workspace).is_some_and(|review| {
                        &review.identity == identity
                            && &review.engine == engine
                            && matches!(review.cli, CliAvailability::Ready { .. })
                    })
            }
            DevcontainerAction::Lifecycle {
                kind,
                identity,
                engine,
                ..
            } => {
                state.active.is_none()
                    && self.devcontainers.review(workspace).is_some_and(|review| {
                        &review.identity == identity
                            && &review.engine == engine
                            && review.approved
                            && matches!(review.cli, CliAvailability::Ready { .. })
                    })
                    && (!matches!(kind, OperationKind::Rebuild)
                        || self.devcontainers.has_ownership(workspace))
            }
            DevcontainerAction::CancelOperation { operation, .. } => state
                .active
                .as_ref()
                .is_some_and(|active| active.key.id == *operation),
            DevcontainerAction::InspectOutput { operation, .. } => {
                state
                    .active
                    .as_ref()
                    .is_some_and(|active| active.key.id == *operation)
                    || state
                        .last
                        .as_ref()
                        .is_some_and(|last| last.key.id == *operation)
            }
            DevcontainerAction::Dirty { .. } => unreachable!(),
        }
    }

    pub(crate) fn invoke_devcontainer_action(&mut self, action: DevcontainerAction) {
        if !self.devcontainer_action_valid(&action) {
            self.message = "dev container action is stale; reopen actions".into();
            return;
        }
        if let DevcontainerAction::Dirty { id, choice } = action {
            self.resolve_devcontainer_dirty(id, choice);
            return;
        }
        let (workspace, discovery) = match &action {
            DevcontainerAction::Review {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::Later {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::SuppressWorkspace {
                workspace,
                discovery,
            }
            | DevcontainerAction::SelectConfiguration {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::SelectEngine {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::Approve {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::CloseReview {
                workspace,
                discovery,
            }
            | DevcontainerAction::ProbeCli {
                workspace,
                discovery,
            }
            | DevcontainerAction::ReadConfiguration {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::Lifecycle {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::CancelOperation {
                workspace,
                discovery,
                ..
            }
            | DevcontainerAction::InspectOutput {
                workspace,
                discovery,
                ..
            } => (workspace.clone(), *discovery),
            DevcontainerAction::Dirty { .. } => unreachable!(),
        };
        let result = match action {
            DevcontainerAction::Review { identity, .. } => self
                .devcontainers
                .review_options(&workspace, &identity)
                .map(|_| {
                    self.feedback.resolve_owner(
                        &self.devcontainer_owner(&workspace, discovery),
                        FeedbackKind::DevcontainerOffer,
                    );
                    self.open_devcontainer_review(&workspace);
                }),
            DevcontainerAction::Later { identity, .. } => {
                self.devcontainers.later(&workspace, &identity).map(|_| {
                    self.feedback.resolve_owner(
                        &self.devcontainer_owner(&workspace, discovery),
                        FeedbackKind::DevcontainerOffer,
                    );
                    if let Some(offer) = self.devcontainers.offer(&workspace) {
                        self.publish_devcontainer_offer(offer);
                    }
                })
            }
            DevcontainerAction::SuppressWorkspace { .. } => {
                self.devcontainers.suppress_workspace(&workspace).map(|_| {
                    self.feedback.resolve_owner(
                        &self.devcontainer_owner(&workspace, discovery),
                        FeedbackKind::DevcontainerOffer,
                    );
                })
            }
            DevcontainerAction::SelectConfiguration { identity, .. } => self
                .devcontainers
                .select_configuration(&workspace, &identity)
                .map(|_| self.open_devcontainer_review(&workspace)),
            DevcontainerAction::SelectEngine { engine, .. } => self
                .devcontainers
                .select_engine(&workspace, engine)
                .map(|_| self.publish_devcontainer_review(&workspace)),
            DevcontainerAction::Approve { .. } => self
                .devcontainers
                .approve_review(&workspace)
                .map(|_| self.publish_devcontainer_review(&workspace)),
            DevcontainerAction::CloseReview { .. } => self
                .devcontainers
                .close_review(&workspace)
                .map(|_| self.publish_devcontainer_review(&workspace)),
            DevcontainerAction::ProbeCli { .. } => self
                .devcontainers
                .probe_cli(&workspace, None)
                .map(|_| self.publish_devcontainer_operation(&workspace)),
            DevcontainerAction::ReadConfiguration { merged, .. } => {
                let result = if merged {
                    self.devcontainers.read_merged_configuration(
                        &workspace,
                        MetadataResolutionConsent::explicit(),
                    )
                } else {
                    self.devcontainers.read_configuration(&workspace)
                };
                result.map(|operation| {
                    self.devcontainers.ui.read_focus = Some(operation);
                    self.publish_devcontainer_operation(&workspace);
                })
            }
            DevcontainerAction::Lifecycle { kind, .. } => {
                let result = match kind {
                    OperationKind::Attach if self.devcontainers.has_ownership(&workspace) => {
                        self.devcontainers.attach_owned(&workspace)
                    }
                    OperationKind::Attach => self.devcontainers.attach(&workspace, None),
                    OperationKind::Start => {
                        self.devcontainers.start(&workspace, LockfilePolicy::Frozen)
                    }
                    OperationKind::Rebuild => self
                        .devcontainers
                        .rebuild(&workspace, LockfilePolicy::Frozen),
                    _ => Err(DevcontainerStateError::NoConfiguration),
                };
                result.map(|_| self.publish_devcontainer_operation(&workspace))
            }
            DevcontainerAction::CancelOperation { .. } => self
                .devcontainers
                .cancel_operation(&workspace)
                .map(|_| self.publish_devcontainer_operation(&workspace)),
            DevcontainerAction::InspectOutput { .. } => {
                self.open_devcontainer_output(&workspace);
                Ok(())
            }
            DevcontainerAction::Dirty { .. } => unreachable!(),
        };
        if let Err(error) = result {
            self.devcontainer_action_failed(workspace, discovery, error);
        }
    }

    pub(super) fn devcontainer_action_failed(
        &mut self,
        workspace: WorkspaceKey,
        discovery: u64,
        error: DevcontainerStateError,
    ) {
        let actions = self.devcontainer_actions(&workspace, discovery);
        self.publish_devcontainer_record(
            workspace.clone(),
            discovery,
            FeedbackKind::DevcontainerOperation,
            FeedbackSeverity::Failure,
            DevcontainerMessage::new(
                "Dev container action failed".into(),
                format!("Workspace {}\n{error}", workspace.root.display()),
            ),
            actions,
        );
        self.message = error.to_string();
    }
}
