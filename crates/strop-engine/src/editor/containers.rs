//! Explicit container discovery/inspection. Every subsequent resource read uses
//! the common opener and Directory model, with an incarnation-pinned identity.
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
mod container_worker;
#[cfg(test)]
mod tests;
use super::io::OpenIntent;
use super::Editor;
pub(crate) use container_worker::{BoundWorker, ContainerWorkers};
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, Completion, FailureKind, Outcome, Ticket, WorkerId};
use strop_workspace::operation::{FsFailure, FsFailureKind};
use strop_workspace::Filesystem;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContainerJob {
    Discover,
    Attach {
        id: String,
        path: std::path::PathBuf,
        intent: OpenIntent,
    },
    /// Explicit consent to provision/reuse a verified worker for this
    /// already attached incarnation. Browsing alone never deploys.
    EnableWorker {
        id: String,
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
    WorkerReady { id: String, target: String },
}
pub type ContainerEvent = Completion<ContainerKey, ContainerResult>;
pub(crate) struct ContainerState {
    pub tx: Sender<ContainerEvent>,
    pub rx: Option<Receiver<ContainerEvent>>,
    pub pending: Option<Ticket<ContainerKey>>,
    pub attached: HashMap<String, strop_containers::ContainerIdentity>,
    pub(crate) workers: ContainerWorkers,
}
impl Default for ContainerState {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx: Some(rx),
            pending: None,
            attached: HashMap::new(),
            workers: ContainerWorkers::default(),
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
        self.message = if matches!(job, ContainerJob::EnableWorker { .. }) {
            "admitting verified container worker".into()
        } else {
            "inspecting container context".into()
        };
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
        let leases = self.containers.workers.clone();
        let selected = match &job {
            ContainerJob::EnableWorker { id } => self.containers.attached.get(id).cloned(),
            _ => None,
        };
        let handle = worker::spawn(
            "container-job",
            move |outcome| {
                let _ = tx.send(Completion { ticket, outcome });
            },
            move |token| match run_container_job(&job, selected.as_ref(), &leases, &token) {
                Ok(result) => Outcome::Success(result),
                Err(error) if error.kind == FsFailureKind::Cancelled => {
                    Outcome::Cancelled(worker::CancelReason::Dismissed)
                }
                Err(error) => Outcome::failed(
                    if matches!(
                        error.kind,
                        FsFailureKind::Unsupported | FsFailureKind::Permission
                    ) {
                        FailureKind::Unavailable
                    } else {
                        FailureKind::Io
                    },
                    error.to_string(),
                ),
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
    pub(crate) fn request_container_worker(&mut self) {
        let Filesystem::Container(id) = self.directory_context().filesystem else {
            self.message = "open an attached container buffer before enabling its worker".into();
            return;
        };
        let Some(identity) = self.containers.attached.get(id.as_str()) else {
            self.message = "attach the container before enabling its worker".into();
            return;
        };
        if self.containers.workers.get(identity).is_some() {
            self.message = "this container already has an admitted worker".into();
            return;
        }
        self.start_container_job(ContainerJob::EnableWorker { id: id.to_string() });
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
                // Incarnation guard (0056 AR07): while this container is
                // attached, a same-id answer carrying a *different*
                // StartedAt means the container was recycled underneath
                // us. The held attachment is the authority views and
                // completions compare against — silently rebinding the
                // new incarnation would let stale state address a fresh
                // container, so the rebind is refused with both
                // incarnations named.
                if let Some(held) = self.containers.attached.get(&identity.id) {
                    if held != &identity {
                        self.message = format!(
                            "container {} was recycled (incarnation {} → {}); refusing to rebind a stale attachment",
                            identity.name, held.started_at, identity.started_at
                        );
                        return;
                    }
                }
                let ContainerJob::Attach { path, intent, .. } = key.job else {
                    self.message = "unexpected container attachment result".into();
                    return;
                };
                if self
                    .workspaces
                    .bind(
                        strop_workspace::Filesystem::Container(reference.id().clone()),
                        None,
                    )
                    .is_err()
                {
                    self.message = "workspace identity space exhausted".into();
                    return;
                }
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
            Outcome::Success(ContainerResult::WorkerReady { id, target }) => {
                if !matches!(&key.job, ContainerJob::EnableWorker { id: requested } if requested == &id)
                {
                    self.message = "container worker result belongs to another request".into();
                    return;
                }
                let lease = self
                    .containers
                    .attached
                    .get(&id)
                    .and_then(|identity| self.containers.workers.get(identity));
                let Some(lease) = lease.filter(|lease| lease.target() == target) else {
                    self.message = "container worker result belongs to a stale incarnation".into();
                    return;
                };
                self.message = format!(
                    "container worker {} ready for {} at {}",
                    env!("CARGO_PKG_VERSION"),
                    target,
                    strop_core::layout::printable_text(lease.artifact_path())
                );
            }
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => {}
        }
    }
}
fn container_failure(error: strop_containers::ContainerError) -> FsFailure {
    let kind = if matches!(error, strop_containers::ContainerError::Cancelled) {
        FsFailureKind::Cancelled
    } else {
        FsFailureKind::Io
    };
    FsFailure::new(kind, error.to_string())
}

fn run_container_job(
    job: &ContainerJob,
    selected: Option<&strop_containers::ContainerIdentity>,
    leases: &ContainerWorkers,
    token: &strop_core::worker::CancelToken,
) -> Result<ContainerResult, FsFailure> {
    match job {
        ContainerJob::Discover => {
            let engine = strop_containers::engine(token).map_err(container_failure)?;
            strop_containers::list_running(&engine, token)
                .map(ContainerResult::Containers)
                .map_err(container_failure)
        }
        ContainerJob::Attach { id, .. } => {
            let engine = strop_containers::engine(token).map_err(container_failure)?;
            let identity =
                strop_containers::inspect(&engine, id, token).map_err(container_failure)?;
            leases.note_inspect(&identity, engine)?;
            Ok(ContainerResult::Attached(identity))
        }
        ContainerJob::EnableWorker { id } => {
            let Some(identity) = selected.filter(|identity| &identity.id == id) else {
                return Err(FsFailure::new(
                    FsFailureKind::InvalidPath,
                    "container worker request has no matching attached identity",
                ));
            };
            let lease = leases.admit(identity, "container worker enable", token)?;
            Ok(ContainerResult::WorkerReady {
                id: id.clone(),
                target: lease.target().to_string(),
            })
        }
    }
}
