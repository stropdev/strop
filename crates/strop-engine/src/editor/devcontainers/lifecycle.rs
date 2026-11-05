use super::jobs::{self, InternalEvent, InternalOperationOutcome};
use super::model::*;
use super::{DevcontainerState, WorkspaceKey};
use std::path::PathBuf;
use std::sync::Arc;
use strop_containers::devcontainer::{
    AttachRequest, CliProcess, ConfigSelection, LockfilePolicy, MetadataResolutionConsent,
    RawConfiguration, RebuildRequest, ReferenceCli, StartRequest,
};
use strop_core::worker::{CancelReason, CancelToken};
use strop_workspace::{ContainerId, Filesystem, ResourceLocation};

impl DevcontainerState {
    pub fn probe_cli(
        &mut self,
        key: &WorkspaceKey,
        explicit_path: Option<PathBuf>,
    ) -> Result<OperationId, DevcontainerStateError> {
        self.probe_cli_inner(key, explicit_path, None)
    }

    /// Engine-owned/fake process boundary. No discovery or pure review path calls it.
    #[cfg(test)]
    pub fn probe_cli_with_runner(
        &mut self,
        key: &WorkspaceKey,
        explicit_path: Option<PathBuf>,
        runner: Arc<dyn CliProcess>,
    ) -> Result<OperationId, DevcontainerStateError> {
        self.probe_cli_inner(key, explicit_path, Some(runner))
    }

    fn probe_cli_inner(
        &mut self,
        key: &WorkspaceKey,
        explicit_path: Option<PathBuf>,
        runner: Option<Arc<dyn CliProcess>>,
    ) -> Result<OperationId, DevcontainerStateError> {
        let (identity, discovery) = self.operation_identity(key, false)?;
        let id = OperationId(self.allocate_id()?);
        let operation = OperationKey {
            id,
            workspace: key.clone(),
            identity,
            discovery,
            kind: OperationKind::ProbeCli,
        };
        let (token, cancel) = CancelToken::standalone();
        self.invalidate_transition(key);
        self.workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?
            .active = Some(ActiveOperation {
            key: operation.clone(),
            control: OperationControl::Cli(Some(cancel)),
            cancellation_requested: false,
            output: OutputBuffer::default(),
        });
        let sender = self.tx.clone();
        let event_key = operation.clone();
        let spawn = jobs::spawn(
            "devcontainer-cli-probe",
            sender,
            self.wake_tx.clone(),
            token,
            move |result| InternalEvent::Operation {
                key: event_key,
                result: Box::new(result.map(InternalOperationOutcome::Probe)),
            },
            move |token| {
                match runner {
                    Some(runner) => {
                        ReferenceCli::probe_with_runner(explicit_path.as_deref(), runner, token)
                    }
                    None => ReferenceCli::probe(explicit_path.as_deref(), token),
                }
                .map_err(OperationError::from)
            },
        );
        if let Err(error) = spawn {
            self.finish_operation(operation, Err(error));
        }
        Ok(id)
    }

    pub fn read_configuration(
        &mut self,
        key: &WorkspaceKey,
    ) -> Result<OperationId, DevcontainerStateError> {
        self.read_configuration_inner(key, None)
    }

    pub fn read_merged_configuration(
        &mut self,
        key: &WorkspaceKey,
        consent: MetadataResolutionConsent,
    ) -> Result<OperationId, DevcontainerStateError> {
        self.read_configuration_inner(key, Some(consent))
    }

    fn read_configuration_inner(
        &mut self,
        key: &WorkspaceKey,
        consent: Option<MetadataResolutionConsent>,
    ) -> Result<OperationId, DevcontainerStateError> {
        let (identity, discovery, selection, cli) = self.operation_selection(key, false)?;
        let merged = consent.is_some();
        let id = OperationId(self.allocate_id()?);
        let operation = OperationKey {
            id,
            workspace: key.clone(),
            identity,
            discovery,
            kind: OperationKind::ReadConfiguration { merged },
        };
        let (token, cancel) = CancelToken::standalone();
        self.invalidate_transition(key);
        let state = self
            .workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        state.read = ConfigurationReadState::Running { merged };
        state.active = Some(ActiveOperation {
            key: operation.clone(),
            control: OperationControl::Cli(Some(cancel)),
            cancellation_requested: false,
            output: OutputBuffer::default(),
        });
        let sender = self.tx.clone();
        let event_key = operation.clone();
        let spawn = jobs::spawn(
            "devcontainer-read-configuration",
            sender,
            self.wake_tx.clone(),
            token,
            move |result| InternalEvent::Operation {
                key: event_key,
                result: Box::new(
                    result.map(|configuration| InternalOperationOutcome::Read { configuration }),
                ),
            },
            move |token| {
                match consent {
                    Some(consent) => cli.read_merged_configuration(&selection, consent, token),
                    None => cli.read_configuration(&selection, token),
                }
                .map_err(OperationError::from)
            },
        );
        if let Err(error) = spawn {
            self.finish_operation(operation, Err(error));
        }
        Ok(id)
    }

    pub fn attach(
        &mut self,
        key: &WorkspaceKey,
        expected_container: Option<&str>,
    ) -> Result<OperationId, DevcontainerStateError> {
        let (identity, discovery, selection, cli) = self.operation_selection(key, true)?;
        let mut request = AttachRequest::new(selection);
        if let Some(expected) = expected_container {
            request = request.expect_container(expected)?;
        }
        let job = cli.attach(request)?;
        self.start_owned(key, identity, discovery, OperationKind::Attach, job)
    }

    /// Reopen exactly the environment created by start/rebuild, including its
    /// private ownership label and canonical container id.
    pub fn attach_owned(
        &mut self,
        key: &WorkspaceKey,
    ) -> Result<OperationId, DevcontainerStateError> {
        let (identity, discovery, selection, cli) = self.operation_selection(key, true)?;
        let receipt = self
            .workspaces
            .get(key)
            .and_then(|state| state.ownership.clone())
            .ok_or(DevcontainerStateError::OwnershipRequired)?;
        let request = AttachRequest::new(selection).owned(&receipt)?;
        let job = cli.attach(request)?;
        self.start_owned(key, identity, discovery, OperationKind::Attach, job)
    }

    pub fn start(
        &mut self,
        key: &WorkspaceKey,
        lockfile: LockfilePolicy,
    ) -> Result<OperationId, DevcontainerStateError> {
        let (identity, discovery, selection, cli) = self.operation_selection(key, true)?;
        let job = cli.start(StartRequest::new(selection, lockfile))?;
        self.start_owned(key, identity, discovery, OperationKind::Start, job)
    }

    pub fn rebuild(
        &mut self,
        key: &WorkspaceKey,
        lockfile: LockfilePolicy,
    ) -> Result<OperationId, DevcontainerStateError> {
        let (identity, discovery, selection, cli) = self.operation_selection(key, true)?;
        let receipt = self
            .workspaces
            .get(key)
            .and_then(|state| state.ownership.clone())
            .ok_or(DevcontainerStateError::OwnershipRequired)?;
        let request = RebuildRequest::new(selection, lockfile, &receipt)?;
        let job = cli.rebuild(request)?;
        self.start_owned(key, identity, discovery, OperationKind::Rebuild, job)
    }

    fn operation_identity(
        &self,
        key: &WorkspaceKey,
        approved: bool,
    ) -> Result<(strop_containers::devcontainer::ConfigIdentity, u64), DevcontainerStateError> {
        let state = self
            .workspaces
            .get(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        if state.active.is_some() {
            return Err(DevcontainerStateError::OperationInProgress);
        }
        let identity = Self::selected(state)?;
        let DiscoveryState::Ready { id: discovery } = &state.discovery else {
            return Err(DevcontainerStateError::StaleConfiguration);
        };
        if approved && state.approved.as_ref() != Some(&identity) {
            return Err(DevcontainerStateError::ReviewRequired);
        }
        Ok((identity, *discovery))
    }

    fn operation_selection(
        &self,
        key: &WorkspaceKey,
        approved: bool,
    ) -> Result<
        (
            strop_containers::devcontainer::ConfigIdentity,
            u64,
            ConfigSelection,
            ReferenceCli,
        ),
        DevcontainerStateError,
    > {
        let (identity, discovery) = self.operation_identity(key, approved)?;
        let state = self
            .workspaces
            .get(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        let configuration = state
            .report
            .as_ref()
            .and_then(|report| {
                report
                    .configurations
                    .iter()
                    .find(|configuration| configuration.identity == identity)
            })
            .ok_or(DevcontainerStateError::StaleConfiguration)?;
        let CliState::Ready(cli) = &state.cli else {
            return Err(DevcontainerStateError::CliUnavailable);
        };
        Ok((
            identity,
            discovery,
            ConfigSelection::new(configuration, state.engine.clone()),
            cli.clone(),
        ))
    }

    fn start_owned(
        &mut self,
        key: &WorkspaceKey,
        identity: strop_containers::devcontainer::ConfigIdentity,
        discovery: u64,
        kind: OperationKind,
        job: strop_containers::devcontainer::OwnedJob,
    ) -> Result<OperationId, DevcontainerStateError> {
        let id = OperationId(self.allocate_id()?);
        let operation = OperationKey {
            id,
            workspace: key.clone(),
            identity,
            discovery,
            kind,
        };
        let control = job.control();
        self.invalidate_transition(key);
        self.workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?
            .active = Some(ActiveOperation {
            key: operation.clone(),
            control: OperationControl::Lifecycle(control),
            cancellation_requested: false,
            output: OutputBuffer::default(),
        });
        if let Err(error) = jobs::spawn_owned(
            self.tx.clone(),
            self.wake_tx.clone(),
            operation.clone(),
            job,
        ) {
            self.finish_operation(operation, Err(error));
        }
        Ok(id)
    }

    fn invalidate_transition(&mut self, key: &WorkspaceKey) {
        if self
            .transition
            .as_ref()
            .is_some_and(|pending| &pending.permit.workspace == key)
        {
            self.transition = None;
        }
    }

    /// Returns true only for the transition to requested. Terminal state is
    /// published exclusively after OwnedJob/CLI work actually settles.
    pub fn cancel_operation(&mut self, key: &WorkspaceKey) -> Result<bool, DevcontainerStateError> {
        let state = self
            .workspaces
            .get_mut(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        let active = state
            .active
            .as_mut()
            .ok_or(DevcontainerStateError::NoActiveOperation)?;
        if active.cancellation_requested {
            return Ok(false);
        }
        active.cancellation_requested = true;
        match &mut active.control {
            OperationControl::Cli(cancel) => {
                if let Some(cancel) = cancel.take() {
                    cancel.cancel(CancelReason::Dismissed);
                }
            }
            OperationControl::Lifecycle(control) => control.cancel(),
        }
        Ok(true)
    }

    pub fn read_configuration_value(
        &self,
        key: &WorkspaceKey,
    ) -> Result<Option<RawConfiguration>, DevcontainerStateError> {
        let Some(state) = self.workspaces.get(key) else {
            return Err(DevcontainerStateError::UnknownWorkspace);
        };
        state
            .configuration
            .as_ref()
            .map(|configuration| {
                configuration
                    .configuration()
                    .map_err(DevcontainerStateError::from)
            })
            .transpose()
    }

    pub fn merged_configuration_value(
        &self,
        key: &WorkspaceKey,
    ) -> Result<Option<RawConfiguration>, DevcontainerStateError> {
        let Some(state) = self.workspaces.get(key) else {
            return Err(DevcontainerStateError::UnknownWorkspace);
        };
        state
            .configuration
            .as_ref()
            .and_then(|configuration| configuration.merged_configuration())
            .map(|configuration| configuration.map_err(DevcontainerStateError::from))
            .transpose()
    }

    /// Verify the successful current operation and its EnvironmentContext before
    /// constructing a container root. No host path is aliased by similarity.
    fn verified_environment(
        &self,
        key: &WorkspaceKey,
        candidate: &strop_containers::devcontainer::EnvironmentContext,
    ) -> Result<
        (
            strop_containers::devcontainer::EnvironmentContext,
            ResourceLocation,
        ),
        DevcontainerStateError,
    > {
        let state = self
            .workspaces
            .get(key)
            .ok_or(DevcontainerStateError::UnknownWorkspace)?;
        let Some(publication) = &state.last else {
            return Err(DevcontainerStateError::UnverifiedEnvironment);
        };
        if state.selected.as_ref() != Some(&candidate.identity)
            || !matches!(state.discovery, DiscoveryState::Ready { id } if id == publication.key.discovery)
        {
            return Err(DevcontainerStateError::UnverifiedEnvironment);
        }
        let OperationOutcome::Lifecycle(Ok(result)) = &publication.outcome else {
            return Err(DevcontainerStateError::UnverifiedEnvironment);
        };
        let Some(verified) = &result.context else {
            return Err(DevcontainerStateError::UnverifiedEnvironment);
        };
        if result.state != strop_containers::devcontainer::JobState::Succeeded
            || publication.key.kind.lifecycle() != Some(result.operation)
            || verified.job != candidate.job
            || verified.container_id != candidate.container_id
            || verified.operation != candidate.operation
            || verified.identity != candidate.identity
            || state.canonical_root.as_deref() != Some(verified.identity.workspace_root())
            || verified.workspace.host_root.as_path() != verified.identity.workspace_root()
        {
            return Err(DevcontainerStateError::UnverifiedEnvironment);
        }
        let remote = PathBuf::from(&verified.workspace.remote_folder);
        if !remote.is_absolute() {
            return Err(DevcontainerStateError::UnverifiedEnvironment);
        }
        let container = ContainerId::canonical(verified.container_id.clone())
            .map_err(|_| DevcontainerStateError::UnverifiedEnvironment)?;
        let root = ResourceLocation {
            filesystem: Filesystem::Container(container),
            path: remote,
        };
        Ok((verified.clone(), root))
    }

    pub fn begin_transition(
        &mut self,
        key: &WorkspaceKey,
        context: strop_containers::devcontainer::EnvironmentContext,
        dirty_local_buffers: bool,
    ) -> Result<TransitionRequest, DevcontainerStateError> {
        if self.transition.is_some() {
            return Err(DevcontainerStateError::OperationInProgress);
        }
        let (context, root) = self.verified_environment(key, &context)?;
        let retire_previous = self
            .environment
            .as_ref()
            .filter(|active| {
                context.supersedes.as_deref() == Some(active.identity.container_id.as_str())
            })
            .map(|active| active.identity.clone());
        let permit = TransitionPermit {
            id: self.allocate_id()?,
            workspace: key.clone(),
            retire_previous,
            environment: context,
            root,
        };
        let id = permit.id;
        self.transition = Some(PendingTransition {
            permit: permit.clone(),
            stage: if dirty_local_buffers {
                TransitionStage::AwaitingChoice
            } else {
                TransitionStage::Ready
            },
        });
        Ok(if dirty_local_buffers {
            TransitionRequest::Dirty(DirtyTransitionPrompt {
                id,
                workspace: key.clone(),
            })
        } else {
            TransitionRequest::Ready(Box::new(permit))
        })
    }

    pub fn resolve_dirty_transition(
        &mut self,
        choice: DirtyTransitionChoice,
    ) -> Result<DirtyTransitionResolution, DevcontainerStateError> {
        let pending = self
            .transition
            .as_mut()
            .ok_or(DevcontainerStateError::NoPendingTransition)?;
        if matches!(pending.stage, TransitionStage::Saving) {
            return Err(DevcontainerStateError::SaveInProgress);
        }
        if matches!(pending.stage, TransitionStage::Ready) {
            return Err(DevcontainerStateError::DirtyChoiceRequired);
        }
        match choice {
            DirtyTransitionChoice::Save => {
                let id = pending.permit.id;
                pending.stage = TransitionStage::Saving;
                Ok(DirtyTransitionResolution::SaveRequired(id))
            }
            DirtyTransitionChoice::Stay => {
                self.transition = None;
                Ok(DirtyTransitionResolution::Stayed)
            }
            DirtyTransitionChoice::Cancel => {
                self.transition = None;
                Ok(DirtyTransitionResolution::Cancelled)
            }
        }
    }

    /// Parent supplies the real save outcome. Failure leaves both the pending
    /// original workspace and its active environment untouched.
    pub fn complete_transition_save(
        &mut self,
        id: u64,
        result: Result<(), String>,
    ) -> Result<TransitionPermit, DevcontainerStateError> {
        let pending = self
            .transition
            .as_mut()
            .ok_or(DevcontainerStateError::NoPendingTransition)?;
        if pending.permit.id != id {
            return Err(DevcontainerStateError::NoPendingTransition);
        }
        if !matches!(pending.stage, TransitionStage::Saving) {
            return Err(DevcontainerStateError::DirtyChoiceRequired);
        }
        match result {
            Ok(()) => {
                pending.stage = TransitionStage::Ready;
                Ok(pending.permit.clone())
            }
            Err(error) => {
                pending.stage = TransitionStage::SaveFailed;
                Err(DevcontainerStateError::SaveFailed(error))
            }
        }
    }

    /// Commit only after parent root selection succeeds. A failed root selection
    /// calls `abort_transition`; no active environment has been changed yet.
    pub fn commit_transition(
        &mut self,
        permit: TransitionPermit,
    ) -> Result<ActiveEnvironment, DevcontainerStateError> {
        let pending = self
            .transition
            .as_ref()
            .ok_or(DevcontainerStateError::NoPendingTransition)?;
        if !matches!(pending.stage, TransitionStage::Ready)
            || pending.permit.id != permit.id
            || pending.permit.environment.job != permit.environment.job
            || pending.permit.environment.operation != permit.environment.operation
            || pending.permit.environment.identity != permit.environment.identity
            || pending.permit.environment.container_id != permit.environment.container_id
            || pending.permit.root != permit.root
        {
            return Err(DevcontainerStateError::NoPendingTransition);
        }
        self.verified_environment(&permit.workspace, &permit.environment)?;
        let pending = self
            .transition
            .take()
            .ok_or(DevcontainerStateError::NoPendingTransition)?;
        let active = ActiveEnvironment {
            identity: EnvironmentIdentity::of(&pending.permit.environment),
            root: pending.permit.root,
        };
        self.environment = Some(active.clone());
        Ok(active)
    }

    #[cfg(test)]
    pub fn verified_container_root(
        &self,
        key: &WorkspaceKey,
        context: &strop_containers::devcontainer::EnvironmentContext,
    ) -> Result<ResourceLocation, DevcontainerStateError> {
        self.verified_environment(key, context)
            .map(|(_, root)| root)
    }

    pub fn abort_transition(&mut self, id: u64) -> Result<(), DevcontainerStateError> {
        if self
            .transition
            .as_ref()
            .is_none_or(|pending| pending.permit.id != id)
        {
            return Err(DevcontainerStateError::NoPendingTransition);
        }
        self.transition = None;
        Ok(())
    }
}
