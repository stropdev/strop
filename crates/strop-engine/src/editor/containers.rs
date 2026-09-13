//! Explicit container discovery/inspection. Every subsequent resource read uses
//! the common opener and Directory model, with an incarnation-pinned identity.
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
#[cfg(test)]
mod tests;
use super::io::OpenIntent;
use super::Editor;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, Completion, FailureKind, Outcome, Ticket, WorkerId};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContainerJob {
    Discover,
    Attach {
        id: String,
        path: std::path::PathBuf,
        intent: OpenIntent,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContainerKey {
    pub request: WorkerId,
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub focus: u64,
    pub job: ContainerJob,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub enum ContainerResult {
    Containers(Vec<strop_containers::ContainerIdentity>),
    Attached(strop_containers::ContainerIdentity),
}
pub type ContainerEvent = Completion<ContainerKey, ContainerResult>;
pub(crate) struct ContainerState {
    pub tx: Sender<ContainerEvent>,
    pub rx: Option<Receiver<ContainerEvent>>,
    pub pending: Option<Ticket<ContainerKey>>,
    pub attached: HashMap<String, strop_containers::ContainerIdentity>,
}
impl Default for ContainerState {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx: Some(rx),
            pending: None,
            attached: HashMap::new(),
        }
    }
}
impl ContainerState {
    pub fn take_rx(&mut self) -> Option<Receiver<ContainerEvent>> {
        self.rx.take()
    }
}
impl Editor {
    pub(crate) fn request_containers(&mut self) {
        self.start_container_job(ContainerJob::Discover);
    }
    fn start_container_job(&mut self, job: ContainerJob) {
        if self.docs.is_empty() {
            return;
        }
        if let Some(old) = self.containers.pending.take() {
            if let Some(handle) = self.worker_handles.remove(&old.request) {
                handle.cancel(worker::CancelReason::Superseded);
            }
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: ContainerKey {
                request,
                document: self.current(),
                revision: self.buf().revision(),
                focus: self.focus_epoch,
                job: job.clone(),
            },
        };
        self.containers.pending = Some(ticket.clone());
        self.message = "inspecting container context".into();
        match self.tape.request("container.job", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_container_event(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let tx = self.containers.tx.clone();
        let handle = worker::spawn(
            "container-job",
            move |outcome| {
                let _ = tx.send(Completion { ticket, outcome });
            },
            move |token| match run_container_job(&job, &token) {
                Ok(result) => Outcome::Success(result),
                Err(strop_containers::ContainerError::Cancelled) => {
                    Outcome::Cancelled(worker::CancelReason::Dismissed)
                }
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
    }
    pub(crate) fn attach_container(&mut self, id: String) {
        self.start_container_job(ContainerJob::Attach {
            id,
            path: "/".into(),
            intent: OpenIntent::Browse,
        });
    }
    pub(crate) fn attach_container_target(
        &mut self,
        container: strop_workspace::ContainerId,
        path: std::path::PathBuf,
        intent: OpenIntent,
    ) {
        self.start_container_job(ContainerJob::Attach {
            id: container.to_string(),
            path,
            intent,
        });
    }
    pub(crate) fn handle_container_event(&mut self, event: ContainerEvent) {
        if self.containers.pending.as_ref() != Some(&event.ticket) {
            return;
        }
        self.containers.pending = None;
        self.worker_handles.remove(&event.ticket.request);
        let key = event.ticket.key;
        if self.finishing
            || self.docs.is_empty()
            || key.focus != self.focus_epoch
            || self.current() != key.document
            || self.buf().revision() != key.revision
        {
            return;
        }
        match event.outcome {
            Outcome::Success(ContainerResult::Containers(list)) => {
                if list.is_empty() {
                    self.message = "no running containers on the local engine".into();
                    return;
                }
                let mut items = Vec::with_capacity(list.len());
                for identity in list {
                    let reference = match strop_containers::ContainerRef::of(&identity) {
                        Ok(reference) => reference,
                        Err(error) => {
                            self.message = error.to_string();
                            return;
                        }
                    };
                    items.push(strop_picker::Item {
                        text: format!(
                            "{}  {}  {}",
                            identity.name,
                            &reference.id().as_str()[..12],
                            identity.image
                        ),
                        badge: None,
                        payload: strop_picker::Payload::Container(identity.id),
                    });
                }
                self.set_picker(super::picker::PickerGlue::diagnostics(
                    strop_picker::Picker::new(strop_picker::Kind::Containers, items, false),
                ));
            }
            Outcome::Success(ContainerResult::Attached(identity)) => {
                let reference = match strop_containers::ContainerRef::of(&identity) {
                    Ok(reference) => reference,
                    Err(error) => {
                        self.message = error.to_string();
                        return;
                    }
                };
                let ContainerJob::Attach { path, intent, .. } = key.job else {
                    self.message = "unexpected container attachment result".into();
                    return;
                };
                self.workspaces.bind(
                    strop_workspace::Filesystem::Container(reference.id().clone()),
                    None,
                );
                self.containers
                    .attached
                    .insert(identity.id.clone(), identity);
                self.request_target(
                    crate::files::FileTarget::Container {
                        container: reference.id().clone(),
                        path,
                    },
                    intent,
                );
            }
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => {}
        }
    }
}
fn run_container_job(
    job: &ContainerJob,
    token: &strop_core::worker::CancelToken,
) -> Result<ContainerResult, strop_containers::ContainerError> {
    let engine = strop_containers::engine(token)?;
    match job {
        ContainerJob::Discover => Ok(ContainerResult::Containers(strop_containers::list_running(
            &engine, token,
        )?)),
        ContainerJob::Attach { id, .. } => Ok(ContainerResult::Attached(
            strop_containers::inspect(&engine, id, token)?,
        )),
    }
}
