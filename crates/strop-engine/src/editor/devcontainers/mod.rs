//! Editor-side Dev Container discovery, review and explicitly invoked lifecycle.
//! Passive discovery and trust review never probe a CLI or an engine. Parent
//! Editor wiring owns presentation, root selection and actual buffer saves.
mod actions;
mod discovery;
mod jobs;
mod lifecycle;
mod model;
mod presentation;
#[cfg(test)]
mod tests;
mod ui;
#[cfg(test)]
mod ui_tests;

pub use model::*;

use super::WorkspaceKey;
use jobs::{InternalEvent, InternalOperationOutcome};
#[cfg(test)]
use model::OutputBuffer;
use model::{
    ActiveEnvironment, CliState, ConfigurationReadState, DevcontainerStateError, DiscoveryState,
    OperationControl, OperationError, OperationKey, OperationKind, OperationOutcome,
    OperationPhase, OperationResult, OperationStatus, PendingTransition, WorkspaceState,
};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use strop_containers::devcontainer::ConfigIdentity;
#[cfg(test)]
use strop_containers::devcontainer::LifecycleStage;
use strop_core::worker::CancelReason;
use strop_workspace::Filesystem;

pub struct DevcontainerState {
    pub(crate) tx: Sender<InternalEvent>,
    pub(crate) rx: Receiver<InternalEvent>,
    pub(crate) wake_tx: Sender<()>,
    pub(crate) wake_rx: Option<Receiver<()>>,
    pub(crate) workspaces: HashMap<WorkspaceKey, WorkspaceState>,
    pub(crate) next_id: u64,
    pub(crate) outbox: Vec<DevcontainerEvent>,
    pub(crate) transition: Option<PendingTransition>,
    pub(crate) environment: Option<ActiveEnvironment>,
    pub(crate) ui: ui::DevcontainerUiState,
}
impl Default for DevcontainerState {
    fn default() -> Self {
        let (tx, rx) = channel();
        let (wake_tx, wake_rx) = channel();
        Self {
            tx,
            rx,
            wake_tx,
            wake_rx: Some(wake_rx),
            workspaces: HashMap::new(),
            next_id: 0,
            outbox: Vec::new(),
            transition: None,
            environment: None,
            ui: ui::DevcontainerUiState::default(),
        }
    }
}
impl DevcontainerState {
    pub fn take_wake_rx(&mut self) -> Option<Receiver<()>> {
        self.wake_rx.take()
    }

    pub fn pending(&self) -> bool {
        self.workspaces
            .values()
            .any(|workspace| workspace.discovery_cancel.is_some() || workspace.active.is_some())
    }

    pub(crate) fn allocate_id(&mut self) -> Result<u64, DevcontainerStateError> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(DevcontainerStateError::IdentityExhausted)?;
        Ok(self.next_id)
    }

    pub(crate) fn require_local(workspace: &WorkspaceKey) -> Result<(), DevcontainerStateError> {
        if workspace.filesystem == Filesystem::Local {
            Ok(())
        } else {
            Err(DevcontainerStateError::NonLocalWorkspace)
        }
    }

    pub(crate) fn selected(
        workspace: &WorkspaceState,
    ) -> Result<ConfigIdentity, DevcontainerStateError> {
        workspace
            .selected
            .clone()
            .ok_or(DevcontainerStateError::NoConfiguration)
    }

    /// Drain actually settled workers and refresh the current redacted log
    /// snapshots. Stale workspace/config/operation incarnations never publish.
    pub fn poll(&mut self) -> Vec<DevcontainerEvent> {
        for workspace in self.workspaces.values_mut() {
            if let Some(active) = &mut workspace.active {
                active.refresh();
            }
        }
        while let Ok(event) = self.rx.try_recv() {
            self.handle_internal(event);
        }
        std::mem::take(&mut self.outbox)
    }

    pub(crate) fn handle_internal(&mut self, event: InternalEvent) {
        match event {
            InternalEvent::Discovery {
                workspace,
                incarnation,
                result,
            } => self.finish_discovery(workspace, incarnation, result),
            InternalEvent::Operation { key, result } => self.finish_operation(key, *result),
        }
    }

    fn finish_operation(
        &mut self,
        key: OperationKey,
        result: Result<InternalOperationOutcome, OperationError>,
    ) {
        let Some(workspace) = self.workspaces.get_mut(&key.workspace) else {
            return;
        };
        if workspace
            .active
            .as_ref()
            .is_none_or(|active| active.key.id != key.id)
        {
            return;
        }
        let Some(mut active) = workspace.active.take() else {
            panic!("active operation checked");
        };
        active.refresh();
        let current = workspace.selected.as_ref() == Some(&key.identity)
            && matches!(workspace.discovery, DiscoveryState::Ready { id } if id == key.discovery);
        let outcome = match (key.kind, result) {
            (OperationKind::ProbeCli, Ok(InternalOperationOutcome::Probe(cli))) => {
                let version = cli.version();
                if current {
                    workspace.cli = CliState::Ready(cli);
                }
                OperationOutcome::Probe(Ok(version))
            }
            (OperationKind::ProbeCli, Err(error)) => {
                if current && error != OperationError::Cancelled {
                    workspace.cli = CliState::Unavailable(error.clone());
                }
                OperationOutcome::Probe(Err(error))
            }
            (
                OperationKind::ReadConfiguration { merged },
                Ok(InternalOperationOutcome::Read { configuration }),
            ) => {
                if current {
                    workspace.configuration = Some(configuration);
                    workspace.read = ConfigurationReadState::Ready { merged };
                }
                OperationOutcome::Read {
                    merged,
                    result: Ok(()),
                }
            }
            (OperationKind::ReadConfiguration { merged }, Err(error)) => {
                if current {
                    workspace.read = if error == OperationError::Cancelled {
                        ConfigurationReadState::NotRequested
                    } else {
                        ConfigurationReadState::Failed
                    };
                }
                OperationOutcome::Read {
                    merged,
                    result: Err(error),
                }
            }
            (OperationKind::Attach | OperationKind::Start | OperationKind::Rebuild, result) => {
                let outcome = match result {
                    Ok(InternalOperationOutcome::Lifecycle(outcome)) => {
                        Ok(Box::new(OperationResult::from(*outcome)))
                    }
                    Ok(_) => Err(OperationError::Protocol(
                        "lifecycle returned a non-lifecycle result".into(),
                    )),
                    Err(error) => Err(error),
                };
                if current {
                    if let Ok(result) = &outcome {
                        if let Some(receipt) = &result.receipt {
                            workspace.ownership = Some(receipt.clone());
                        }
                    }
                }
                OperationOutcome::Lifecycle(outcome)
            }
            (kind, Ok(_)) => match kind {
                OperationKind::ProbeCli => OperationOutcome::Probe(Err(OperationError::Protocol(
                    "probe returned a non-probe result".into(),
                ))),
                OperationKind::ReadConfiguration { merged } => OperationOutcome::Read {
                    merged,
                    result: Err(OperationError::Protocol(
                        "read returned a non-read result".into(),
                    )),
                },
                OperationKind::Attach | OperationKind::Start | OperationKind::Rebuild => {
                    unreachable!()
                }
            },
        };
        if current {
            let publication = std::sync::Arc::new(active.into_publication(outcome));
            workspace.last = Some(publication.clone());
            self.outbox
                .push(DevcontainerEvent::OperationSettled(publication));
        }
    }

    pub fn operation_status(&mut self, key: &WorkspaceKey) -> Option<OperationStatus<'_>> {
        let workspace = self.workspaces.get_mut(key)?;
        if let Some(active) = &mut workspace.active {
            active.refresh();
            let current = matches!(workspace.discovery, DiscoveryState::Ready { id } if id == active.key.discovery);
            if current {
                return Some(OperationStatus {
                    key: &active.key,
                    phase: if active.cancellation_requested {
                        OperationPhase::CancellationRequested
                    } else {
                        OperationPhase::Running
                    },
                    cancellation_requested: active.cancellation_requested,
                    output: &active.output,
                    outcome: None,
                });
            }
        }
        workspace.last.as_ref().filter(|settled| {
            workspace.selected.as_ref() == Some(&settled.key.identity)
                && matches!(workspace.discovery, DiscoveryState::Ready { id } if id == settled.key.discovery)
        }).map(|settled| OperationStatus {
            key: &settled.key,
            phase: settled.phase,
            cancellation_requested: settled.cancellation_requested,
            output: &settled.output,
            outcome: Some(&settled.outcome),
        })
    }

    pub fn discovery_incarnation(&self, key: &WorkspaceKey) -> Option<u64> {
        let state = self.workspaces.get(key)?;
        match state.discovery {
            DiscoveryState::Ready { id } => Some(id),
            _ => None,
        }
    }

    pub fn selected_configuration(&self, key: &WorkspaceKey) -> Option<ConfigIdentity> {
        self.workspaces.get(key)?.selected.clone()
    }

    pub fn has_ownership(&self, key: &WorkspaceKey) -> bool {
        self.workspaces
            .get(key)
            .is_some_and(|state| state.ownership.is_some())
    }

    pub fn pending_transition(&self) -> Option<&TransitionPermit> {
        self.transition.as_ref().map(|pending| &pending.permit)
    }

    pub fn transition_save_in_progress(&self) -> bool {
        self.transition
            .as_ref()
            .is_some_and(|pending| matches!(pending.stage, TransitionStage::Saving))
    }

    #[cfg(test)]
    pub fn pending_deferred_stages(&self, key: &WorkspaceKey) -> Option<&[LifecycleStage]> {
        let state = self.workspaces.get(key)?;
        let publication = state.last.as_ref()?;
        if state.selected.as_ref() != Some(&publication.key.identity)
            || !matches!(state.discovery, DiscoveryState::Ready { id } if id == publication.key.discovery)
        {
            return None;
        }
        let OperationOutcome::Lifecycle(Ok(result)) = &publication.outcome else {
            return None;
        };
        Some(
            result
                .context
                .as_ref()?
                .readiness
                .deferred_stages
                .as_slice(),
        )
    }

    #[cfg(test)]
    pub fn output_buffer(&mut self, key: &WorkspaceKey) -> Option<&OutputBuffer> {
        self.operation_status(key).map(|status| status.output)
    }

    #[cfg(test)]
    pub fn active_environment(&self) -> Option<&ActiveEnvironment> {
        self.environment.as_ref()
    }

    pub fn shutdown(&mut self) {
        for workspace in self.workspaces.values_mut() {
            if let Some(cancel) = workspace.discovery_cancel.take() {
                cancel.cancel(CancelReason::Shutdown);
            }
            if let Some(active) = &mut workspace.active {
                match &mut active.control {
                    OperationControl::Cli(cancel) => {
                        if let Some(cancel) = cancel.take() {
                            cancel.cancel(CancelReason::Shutdown);
                        }
                    }
                    OperationControl::Lifecycle(control) => control.cancel(),
                }
            }
        }
    }
}
impl Drop for DevcontainerState {
    fn drop(&mut self) {
        self.shutdown();
    }
}
