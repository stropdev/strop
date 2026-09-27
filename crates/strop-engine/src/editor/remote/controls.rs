//! Explicit user connection pins outlive focus, not the editor. Results still
//! carry their own request identity; a stale result cannot take over the view.
use super::*;
use crate::editor::io::IoEvent;
use strop_core::worker::{self, FailureKind, Outcome};

impl Editor {
    pub(super) fn request_remote_control(&mut self, operation: RemoteControl) {
        if !self.remote.controls.is_empty() {
            self.message = "a remote connection command is still pending".into();
            return;
        }
        if matches!(
            operation,
            RemoteControl::Disconnect(_) | RemoteControl::DisconnectAll
        ) {
            let documents: Vec<_> = self
                .remote
                .following
                .keys()
                .copied()
                .filter(|&id| {
                    let endpoint = self
                        .doc(id)
                        .remote_metadata()
                        .map(|source| source.file.endpoint());
                    match &operation {
                        RemoteControl::Disconnect(target) => endpoint == Some(target),
                        _ => true,
                    }
                })
                .collect();
            for id in documents {
                self.stop_remote_follow(id);
            }
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let key = ControlKey {
            document: self.current(),
            focus: self.focus_epoch,
            operation: operation.clone(),
        };
        let ticket = Ticket {
            request,
            key: key.clone(),
        };
        self.remote.controls.insert(request, key);
        self.message = if matches!(&operation, RemoteControl::AdmitWorker(_)) {
            "admitting verified remote worker".into()
        } else {
            "remote connection command pending".into()
        };
        match self.tape.request("remote.control", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.remote_control_done(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let client = self.remote_client();
        let workers = self.remote.workers.clone();
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "remote-control",
            move |outcome| {
                let _ = tx.send(IoEvent::Remote(RemoteEvent::Control(Completion {
                    ticket,
                    outcome,
                })));
            },
            move |token| {
                let result = match operation {
                    RemoteControl::AdmitWorker(endpoint) => {
                        return match workers.admit(&endpoint, "remote worker enable", &token) {
                            Ok(_) => Outcome::Success(ControlResult::WorkerReady(endpoint)),
                            Err(error)
                                if error.kind
                                    == strop_workspace::operation::FsFailureKind::Cancelled =>
                            {
                                Outcome::Cancelled(worker::CancelReason::Dismissed)
                            }
                            Err(error) => {
                                Outcome::failed(FailureKind::Unavailable, error.to_string())
                            }
                        };
                    }
                    RemoteControl::Connect(endpoint) => {
                        client
                            .connect(&endpoint, &token)
                            .map(|lease| ControlResult::Connected {
                                endpoint,
                                lease: Some(lease),
                            })
                    }
                    RemoteControl::Disconnect(endpoint) => client
                        .disconnect(&endpoint)
                        .map(|()| ControlResult::Disconnected),
                    RemoteControl::DisconnectAll => client
                        .disconnect_all()
                        .map(|()| ControlResult::Disconnected),
                    RemoteControl::Connections => {
                        let mut endpoints = client.connections();
                        endpoints.sort_by_key(ToString::to_string);
                        let mut text =
                            String::from("Authenticated SFTP connections (observed snapshot)\n\n");
                        if endpoints.is_empty() {
                            text.push_str("No authenticated connections.\n");
                        }
                        for endpoint in endpoints {
                            text.push_str(&format!("{endpoint}\n"));
                        }
                        Ok(ControlResult::Listing(text))
                    }
                };
                match result {
                    Ok(result) => Outcome::Success(result),
                    Err(error) if error.is_cancellation() => {
                        Outcome::Cancelled(worker::CancelReason::Dismissed)
                    }
                    Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                }
            },
        );
        self.worker_handles.insert(request, handle);
    }
    pub(super) fn remote_control_done(
        &mut self,
        completion: Completion<ControlKey, ControlResult>,
    ) {
        let request = completion.ticket.request;
        if self.remote.controls.get(&request) != Some(&completion.ticket.key) {
            return;
        }
        self.remote.controls.remove(&request);
        self.worker_handles.remove(&request);
        let key = completion.ticket.key;
        let focused = !self.docs.is_empty()
            && self.current() == key.document
            && self.focus_epoch == key.focus
            && !self.finishing;
        match completion.outcome {
            Outcome::Success(ControlResult::Connected { endpoint, lease }) => {
                if self
                    .workspaces
                    .bind(strop_workspace::Filesystem::Remote(endpoint.clone()), None)
                    .is_err()
                {
                    self.message = "workspace identity space exhausted".into();
                    return;
                }
                if let Some(lease) = lease.filter(|_| !self.finishing) {
                    self.remote.pins.insert(endpoint, lease);
                }
                if focused {
                    self.message = "remote connection held".into();
                }
            }
            Outcome::Success(ControlResult::WorkerReady(endpoint)) if focused => {
                self.message = match self.remote.workers.get_ready(&endpoint) {
                    Some(ready) => format!(
                        "verified remote worker {} ready for {} ({}) at {}",
                        env!("CARGO_PKG_VERSION"),
                        endpoint,
                        ready.target,
                        strop_core::layout::printable_text(&ready.artifact.path)
                    ),
                    None => format!("remote worker for {endpoint} retired before readiness"),
                };
            }
            Outcome::Success(ControlResult::Disconnected) => {
                match &key.operation {
                    RemoteControl::Disconnect(endpoint) => self
                        .workspaces
                        .note_disconnect(&strop_workspace::Filesystem::Remote(endpoint.clone())),
                    RemoteControl::DisconnectAll => {
                        let filesystems: Vec<_> = self
                            .workspaces
                            .iter()
                            .filter(|(_, context)| context.filesystem.is_remote())
                            .map(|(_, context)| context.filesystem.clone())
                            .collect();
                        for filesystem in filesystems {
                            self.workspaces.note_disconnect(&filesystem);
                        }
                    }
                    _ => {}
                }
                match key.operation {
                    RemoteControl::Disconnect(endpoint) => {
                        self.remote.pins.remove(&endpoint);
                    }
                    RemoteControl::DisconnectAll => self.remote.pins.clear(),
                    _ => {}
                }
                if focused {
                    self.message = "remote connection closed".into();
                }
            }
            Outcome::Success(ControlResult::Listing(text)) if focused => {
                let mut buffer = strop_core::Buffer::from_text(&text);
                buffer.name = Some("remote connections".into());
                if self.open_temporary_output(buffer).is_none() {
                    return; // message set
                }
                self.generation += 1;
                self.message.clear();
            }
            Outcome::Failed { failure, .. } if focused => self.message = failure.message,
            _ => {}
        }
    }
}
