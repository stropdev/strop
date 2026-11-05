use super::jobs::{self, InternalEvent};
use super::model::*;
use super::{DevcontainerState, WorkspaceKey};
use strop_containers::devcontainer::{
    discover, CliEngine, ConfigIdentity, DiscoveredConfiguration, DiscoveryReport, TrustReview,
};
use strop_core::worker::{CancelReason, CancelToken};

impl DevcontainerState {
    /// Idempotent project-open/frame entry. Use `rediscover` only for an explicit
    /// new root-selection or configuration-change boundary.
    pub fn discover(
        &mut self,
        workspace: WorkspaceKey,
    ) -> Result<DiscoveryDisposition, DevcontainerStateError> {
        Self::require_local(&workspace)?;
        if self
            .workspaces
            .get(&workspace)
            .is_some_and(|state| !matches!(state.discovery, DiscoveryState::Idle))
        {
            return Ok(DiscoveryDisposition::AlreadySettledOrRunning);
        }
        self.start_discovery(workspace)
    }

    /// Explicitly inspect again. The new incarnation invalidates stale worker
    /// completions; exact digest matches retain selection and review decisions.
    pub fn rediscover(
        &mut self,
        workspace: WorkspaceKey,
    ) -> Result<DiscoveryDisposition, DevcontainerStateError> {
        Self::require_local(&workspace)?;
        self.start_discovery(workspace)
    }

    fn start_discovery(
        &mut self,
        workspace: WorkspaceKey,
    ) -> Result<DiscoveryDisposition, DevcontainerStateError> {
        let incarnation = self.allocate_id()?;
        if self
            .transition
            .as_ref()
            .is_some_and(|pending| pending.permit.workspace == workspace)
        {
            self.transition = None;
        }
        let state = self
            .workspaces
            .entry(workspace.clone())
            .or_insert_with(WorkspaceState::new);
        if let Some(cancel) = state.discovery_cancel.take() {
            cancel.cancel(CancelReason::Superseded);
        }
        let (token, cancel) = CancelToken::standalone();
        state.discovery = DiscoveryState::Running { id: incarnation };
        state.discovery_cancel = Some(cancel);
        state.last = None;
        if matches!(state.read, ConfigurationReadState::Running { .. }) {
            state.read = ConfigurationReadState::NotRequested;
        }
        let root = workspace.root.clone();
        let sender = self.tx.clone();
        let event_workspace = workspace.clone();
        let spawn = jobs::spawn(
            "devcontainer-discovery",
            sender,
            self.wake_tx.clone(),
            token,
            move |result| InternalEvent::Discovery {
                workspace: event_workspace,
                incarnation,
                result,
            },
            move |_| discover(&root).map_err(OperationError::from),
        );
        if let Err(error) = spawn {
            self.finish_discovery(workspace, incarnation, Err(error));
        }
        Ok(DiscoveryDisposition::Started(incarnation))
    }

    pub(crate) fn finish_discovery(
        &mut self,
        key: WorkspaceKey,
        incarnation: u64,
        result: Result<DiscoveryReport, OperationError>,
    ) {
        let Some(state) = self.workspaces.get_mut(&key) else {
            return;
        };
        if !matches!(state.discovery, DiscoveryState::Running { id, .. } if id == incarnation) {
            return;
        }
        state.discovery_cancel = None;
        let public_result = match result {
            Ok(report) => {
                let summary = DiscoverySummary {
                    configurations: report.configurations.len(),
                    issues: report.issues.len(),
                };
                state.canonical_root = report
                    .preferred()
                    .map(|configuration| configuration.identity.workspace_root().to_path_buf());
                let previous = state.selected.take();
                let selected = previous
                    .clone()
                    .filter(|previous| {
                        report
                            .configurations
                            .iter()
                            .any(|configuration| &configuration.identity == previous)
                    })
                    .or_else(|| {
                        report
                            .preferred()
                            .map(|configuration| configuration.identity.clone())
                    });
                if selected != previous {
                    state.approved = None;
                    state.read = ConfigurationReadState::NotRequested;
                    state.configuration = None;
                }
                state.selected = selected;
                let current = report
                    .configurations
                    .iter()
                    .map(|configuration| configuration.identity.clone())
                    .collect::<std::collections::HashSet<_>>();
                state
                    .decisions
                    .later
                    .retain(|identity| current.contains(identity));
                state
                    .decisions
                    .workspace_suppressed
                    .retain(|identity| current.contains(identity));
                state.report = Some(report);
                state.discovery = DiscoveryState::Ready { id: incarnation };
                Ok(summary)
            }
            Err(error) => {
                state.discovery = if error == OperationError::Cancelled {
                    DiscoveryState::Cancelled
                } else {
                    DiscoveryState::Failed
                };
                Err(error)
            }
        };
        let offer = if public_result.is_ok() {
            self.offer(&key)
        } else {
            None
        };
        self.outbox.push(DevcontainerEvent::DiscoverySettled {
            workspace: key,
            incarnation,
            result: public_result,
            offer: offer.map(Box::new),
        });
    }

    pub fn configurations(&self, key: &WorkspaceKey) -> Option<&[DiscoveredConfiguration]> {
        Some(
            self.workspaces
                .get(key)?
                .report
                .as_ref()?
                .configurations
                .as_slice(),
        )
    }

    /// The one non-focusing contextual offer. Later and workspace suppression
    /// are exact digest decisions, not global container preferences.
    pub fn offer(&self, key: &WorkspaceKey) -> Option<DevcontainerOffer> {
        let state = self.workspaces.get(key)?;
        if state.review_open || !matches!(state.discovery, DiscoveryState::Ready { .. }) {
            return None;
        }
        let report = state.report.as_ref()?;
        let configuration = report.configurations.iter().find(|configuration| {
            !state.decisions.later.contains(&configuration.identity)
                && !state
                    .decisions
                    .workspace_suppressed
                    .contains(&configuration.identity)
        })?;
        Some(DevcontainerOffer {
            workspace: key.clone(),
            identity: configuration.identity.clone(),
            name: configuration.configuration.name.clone(),
            configuration_count: report.configurations.len(),
        })
    }

    pub fn later(
        &mut self,
        key: &WorkspaceKey,
        identity: &ConfigIdentity,
    ) -> Result<(), DevcontainerStateError> {
        let state = self
            .workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        Self::ensure_current_configuration(state, identity)?;
        state.decisions.later.insert(identity.clone());
        Ok(())
    }

    pub fn suppress_workspace(&mut self, key: &WorkspaceKey) -> Result<(), DevcontainerStateError> {
        let state = self
            .workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        let report = state
            .report
            .as_ref()
            .ok_or(DevcontainerStateError::NoConfiguration)?;
        state.decisions.workspace_suppressed.extend(
            report
                .configurations
                .iter()
                .map(|configuration| configuration.identity.clone()),
        );
        Ok(())
    }

    pub fn review_options(
        &mut self,
        key: &WorkspaceKey,
        identity: &ConfigIdentity,
    ) -> Result<ReviewSnapshot, DevcontainerStateError> {
        self.select_configuration(key, identity)?;
        self.review(key)
            .ok_or(DevcontainerStateError::NoConfiguration)
    }

    pub fn select_configuration(
        &mut self,
        key: &WorkspaceKey,
        identity: &ConfigIdentity,
    ) -> Result<(), DevcontainerStateError> {
        let state = self
            .workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        if state.active.is_some() {
            return Err(DevcontainerStateError::OperationInProgress);
        }
        Self::ensure_current_configuration(state, identity)?;
        if state.selected.as_ref() != Some(identity) {
            state.selected = Some(identity.clone());
            state.approved = None;
            state.read = ConfigurationReadState::NotRequested;
            state.configuration = None;
            state.last = None;
            if self
                .transition
                .as_ref()
                .is_some_and(|pending| &pending.permit.workspace == key)
            {
                self.transition = None;
            }
        }
        state.review_open = true;
        Ok(())
    }

    fn ensure_current_configuration(
        state: &WorkspaceState,
        identity: &ConfigIdentity,
    ) -> Result<(), DevcontainerStateError> {
        let report = state
            .report
            .as_ref()
            .ok_or(DevcontainerStateError::NoConfiguration)?;
        if report
            .configurations
            .iter()
            .any(|configuration| &configuration.identity == identity)
        {
            Ok(())
        } else {
            Err(DevcontainerStateError::StaleConfiguration)
        }
    }

    pub fn select_engine(
        &mut self,
        key: &WorkspaceKey,
        engine: CliEngine,
    ) -> Result<(), DevcontainerStateError> {
        let state = self
            .workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        if state.active.is_some() {
            return Err(DevcontainerStateError::OperationInProgress);
        }
        if state.engine.identity() != engine.identity() {
            state.engine = engine;
            state.cli = CliState::NotProbed;
            state.approved = None;
            state.read = ConfigurationReadState::NotRequested;
            state.configuration = None;
            state.last = None;
            if self
                .transition
                .as_ref()
                .is_some_and(|pending| &pending.permit.workspace == key)
            {
                self.transition = None;
            }
        }
        Ok(())
    }

    /// Recording approval is explicit and digest scoped. TrustReview remains a
    /// pure preview; this method performs no CLI, engine, hook or network work.
    pub fn approve_review(
        &mut self,
        key: &WorkspaceKey,
    ) -> Result<ReviewSnapshot, DevcontainerStateError> {
        let state = self
            .workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        if !state.review_open {
            return Err(DevcontainerStateError::ReviewRequired);
        }
        let identity = Self::selected(state)?;
        state.approved = Some(identity);
        self.review(key)
            .ok_or(DevcontainerStateError::NoConfiguration)
    }

    pub fn close_review(&mut self, key: &WorkspaceKey) -> Result<(), DevcontainerStateError> {
        let state = self
            .workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        state.review_open = false;
        state.approved = None;
        Ok(())
    }

    pub fn review(&self, key: &WorkspaceKey) -> Option<ReviewSnapshot> {
        let state = self.workspaces.get(key)?;
        if !state.review_open {
            return None;
        }
        let identity = state.selected.clone()?;
        let configuration = state
            .report
            .as_ref()?
            .configurations
            .iter()
            .find(|configuration| configuration.identity == identity)?;
        Some(ReviewSnapshot {
            trust: TrustReview::new(identity.clone(), &configuration.configuration),
            identity,
            engine: state.engine.identity(),
            approved: state.approved.as_ref() == state.selected.as_ref(),
            cli: state.cli.availability(),
            read: state.read.availability(),
        })
    }
}
