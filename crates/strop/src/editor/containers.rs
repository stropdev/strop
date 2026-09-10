//! Existing-container attach and read-only browsing (0037 DC1a): the
//! local engine's running containers as a picker; attach binds a
//! `Filesystem::Container` workspace context and opens a real readonly
//! listing buffer. Listings and file reads are owned, ticketed jobs
//! through strop-containers — never a local path, never an SSH daemon,
//! never lifecycle ownership.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
#[cfg(test)]
mod tests;

use strop_core::id::DocumentId;
use strop_core::worker::{self, Completion, FailureKind, Outcome, Ticket, WorkerId};
use strop_core::Buffer;

use super::document::Document;
use super::Editor;

/// What one owned job was asked to do.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum ContainerJob {
    /// Probe the engine and list running containers.
    Discover,
    /// Attach: resolve the picker choice to canonical identity, then
    /// list the container root.
    Attach { id: String },
    /// List one directory inside an attached container.
    ListDir { id: String, path: String },
    /// Read one bounded file inside an attached container.
    ReadFile { id: String, path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ContainerKey {
    pub request: WorkerId,
    pub focus: u64,
    pub job: ContainerJob,
}

/// One job's terminal answer.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum ContainerResult {
    Containers(Vec<strop_containers::ContainerIdentity>),
    Listing {
        identity: strop_containers::ContainerIdentity,
        path: String,
        entries: Vec<strop_containers::DirEntry>,
    },
    File {
        identity: strop_containers::ContainerIdentity,
        path: String,
        text: String,
    },
}

pub(crate) type ContainerEvent = Completion<ContainerKey, ContainerResult>;

pub(crate) struct ContainerState {
    pub tx: Sender<ContainerEvent>,
    pub rx: Option<Receiver<ContainerEvent>>,
    pub pending: Option<Ticket<ContainerKey>>,
    /// Attached identities by canonical id (the registry owns contexts).
    pub attached: HashMap<String, strop_containers::ContainerIdentity>,
    /// Listing rows per listing buffer, for Enter resolution.
    pub entries: HashMap<DocumentId, Vec<strop_containers::DirEntry>>,
    /// The container each buffer belongs to (listing and file buffers).
    pub buffers: HashMap<DocumentId, (String, String)>, // id, path
}

impl Default for ContainerState {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx: Some(rx),
            pending: None,
            attached: HashMap::new(),
            entries: HashMap::new(),
            buffers: HashMap::new(),
        }
    }
}

impl ContainerState {
    pub fn take_rx(&mut self) -> Option<Receiver<ContainerEvent>> {
        self.rx.take()
    }
}

impl Editor {
    /// `:containers` — probe the local engine and offer running
    /// containers as a picker.
    pub(crate) fn request_containers(&mut self) {
        self.start_container_job(ContainerJob::Discover);
    }

    fn start_container_job(&mut self, job: ContainerJob) {
        if self.containers.pending.is_some() {
            self.message = "a container request is already running".into();
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let key = ContainerKey {
            request,
            focus: self.focus_epoch,
            job: job.clone(),
        };
        let pending = Ticket {
            request,
            key: key.clone(),
        };
        let tx = self.containers.tx.clone();
        let handle = worker::spawn(
            "container-job",
            move |outcome| {
                let _ = tx.send(Completion {
                    ticket: pending,
                    outcome,
                });
            },
            move |token| match run_container_job(&job, &token) {
                Ok(result) => Outcome::Success(result),
                Err(strop_containers::ContainerError::Cancelled) => {
                    Outcome::Cancelled(worker::CancelReason::Dismissed)
                }
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.containers.pending = Some(Ticket { request, key });
        self.worker_handles.insert(request, handle);
    }

    /// Picker acceptance: attach to the chosen container (canonicalize
    /// identity, bind the workspace context, list its root).
    pub(crate) fn attach_container(&mut self, id: String) {
        self.start_container_job(ContainerJob::Attach { id });
    }

    /// Enter on a container listing row: descend into directories, read
    /// files. Rows resolve through the buffer's recorded entries — never
    /// by re-parsing display text against the local disk.
    pub(crate) fn container_key(&mut self, key: super::Key) -> bool {
        if key != super::Key::Enter {
            return false;
        }
        let Some((id, base)) = self.containers.buffers.get(&self.current()).cloned() else {
            return false;
        };
        let Some(entries) = self.containers.entries.get(&self.current()) else {
            return false; // a file buffer: Enter is not navigation
        };
        let line = self.buf().line_of(self.head());
        let Some(entry) = line.checked_sub(1).and_then(|row| entries.get(row)) else {
            self.message = "no container entry at this position".into();
            return true;
        };
        let path = format!("{}/{}", base.trim_end_matches('/'), entry.name);
        let job = match entry.kind {
            strop_containers::DirEntryKind::Dir => ContainerJob::ListDir { id, path },
            _ => ContainerJob::ReadFile { id, path },
        };
        self.start_container_job(job);
        true
    }

    pub(crate) fn handle_container_event(&mut self, event: ContainerEvent) {
        let owned = self
            .containers
            .pending
            .as_ref()
            .is_some_and(|ticket| ticket.request == event.ticket.request);
        if !owned {
            return; // a superseded job's answer belongs to no one
        }
        self.containers.pending = None;
        self.worker_handles.remove(&event.ticket.request);
        let focused = event.ticket.key.focus == self.focus_epoch && !self.finishing;
        match event.outcome {
            Outcome::Success(ContainerResult::Containers(list)) if focused => {
                if list.is_empty() {
                    self.message = "no running containers on the local engine".into();
                    return;
                }
                let items = list
                    .into_iter()
                    .map(|identity| strop_picker::Item {
                        text: format!(
                            "{}  {}  {}",
                            &identity.id[..12],
                            identity.name,
                            identity.image
                        ),
                        payload: strop_picker::Payload::Container(identity.id.clone()),
                    })
                    .collect();
                self.set_picker(super::picker::PickerGlue::diagnostics(
                    strop_picker::Picker::new(strop_picker::Kind::Containers, items, false),
                ));
            }
            Outcome::Success(ContainerResult::Listing {
                identity,
                path,
                entries,
            }) if focused => {
                let Ok(reference) = strop_containers::ContainerRef::of(&identity) else {
                    self.message = "container identity failed validation".into();
                    return;
                };
                self.workspaces.bind(
                    strop_workspace::Filesystem::Container(reference.id().clone()),
                    None,
                );
                self.containers
                    .attached
                    .insert(identity.id.clone(), identity.clone());
                let short = &identity.id[..12];
                let mut text = format!(
                    "container {} ({}:{}) — {} entry(s); enter opens, q closes\n",
                    identity.name,
                    short,
                    path,
                    entries.len()
                );
                for entry in &entries {
                    let marker = match entry.kind {
                        strop_containers::DirEntryKind::Dir => "d",
                        strop_containers::DirEntryKind::Symlink => "l",
                        _ => " ",
                    };
                    let suffix = if entry.kind == strop_containers::DirEntryKind::Dir {
                        "/"
                    } else {
                        ""
                    };
                    text.push_str(&format!("{} {}{}\n", marker, entry.name, suffix));
                }
                let mut buffer = Buffer::from_text(&text);
                buffer.name = Some(format!("container:{short}:{path}"));
                let id = self.docs.insert(Document::output(buffer));
                self.drop_stale_scratch(id);
                self.containers
                    .buffers
                    .insert(id, (identity.id.clone(), path));
                self.containers.entries.insert(id, entries);
                self.switch_to(id);
                self.set_head(0);
            }
            Outcome::Success(ContainerResult::File {
                identity,
                path,
                text,
            }) if focused => {
                let short = identity.id[..12].to_string();
                let mut buffer = Buffer::from_text(&text);
                buffer.name = Some(format!("container:{short}:{path}"));
                let Ok(container) = strop_workspace::ContainerId::canonical(identity.id.clone())
                else {
                    self.message = "container identity failed validation".into();
                    return;
                };
                let id = self.docs.insert(Document::container_file(
                    buffer,
                    container,
                    std::path::PathBuf::from(&path),
                ));
                self.drop_stale_scratch(id);
                self.containers
                    .buffers
                    .insert(id, (identity.id.clone(), path));
                self.switch_to(id);
                self.set_head(0);
                // Container files get language services like any document
                // (DC1b); discovery runs on its own worker.
                self.lsp_maybe_attach();
            }
            Outcome::Success(_) => {}
            Outcome::Failed { failure, .. } if focused => {
                self.message = failure.message;
            }
            _ => {}
        }
    }
}

/// The worker body: engine probe, then the job's actual operation. All
/// docker work is supervised by strop-containers (deadlines, bounded
/// output, cancellation); nothing here touches the local filesystem.
fn run_container_job(
    job: &ContainerJob,
    token: &strop_core::worker::CancelToken,
) -> Result<ContainerResult, strop_containers::ContainerError> {
    let engine = strop_containers::engine(token)?;
    match job {
        ContainerJob::Discover => Ok(ContainerResult::Containers(strop_containers::list_running(
            &engine, token,
        )?)),
        ContainerJob::Attach { id } => {
            let identity = strop_containers::inspect(&engine, id, token)?;
            let entries = strop_containers::list_dir(
                &engine,
                &strop_containers::ContainerRef::of(&identity)?,
                "/",
                token,
            )?;
            Ok(ContainerResult::Listing {
                identity,
                path: "/".into(),
                entries,
            })
        }
        ContainerJob::ListDir { id, path } => {
            let identity = strop_containers::inspect(&engine, id, token)?;
            let entries = strop_containers::list_dir(
                &engine,
                &strop_containers::ContainerRef::of(&identity)?,
                path,
                token,
            )?;
            Ok(ContainerResult::Listing {
                identity,
                path: path.clone(),
                entries,
            })
        }
        ContainerJob::ReadFile { id, path } => {
            let identity = strop_containers::inspect(&engine, id, token)?;
            let text = strop_containers::read_file(
                &engine,
                &strop_containers::ContainerRef::of(&identity)?,
                path,
                8 * 1024 * 1024,
                token,
            )?;
            Ok(ContainerResult::File {
                identity,
                path: path.clone(),
                text: String::from_utf8_lossy(&text).into_owned(),
            })
        }
    }
}
