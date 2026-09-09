//! The existing modal picker chooses a destination; entering its address is a
//! second picker field, not another input machine. Enumeration never connects.
use super::{Editor, RemoteEvent};
use crate::editor::{
    io::{IoEvent, OpenIntent},
    picker::{PickerGlue, PickerId},
};
use crate::files::FileTarget;
use strop_core::worker::{self, CancelReason, Completion, FailureKind, Outcome, Ticket};
use strop_picker::{Catalog, Item, Kind, Payload, Picker};
use strop_workspace::{RemoteEndpoint, RemoteFile, RemoteLocation};

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct RemoteChoices {
    items: Catalog,
    notes: Vec<String>,
}

impl Editor {
    pub(crate) fn open_remote_picker(&mut self) {
        self.set_picker(PickerGlue::diagnostics(Picker::new(
            Kind::RemoteHosts,
            vec![new_host_item()],
            true,
        )));
        let Some(picker) = self.picker.as_ref().map(|glue| glue.id) else {
            return;
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                if let Some(glue) = self.picker.as_mut() {
                    glue.picker.streaming = false;
                    glue.picker.error = Some(error.message);
                }
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: picker,
        };
        self.remote.choices = Some(ticket.clone());
        let state_dir = self.state_dir.clone();
        match self
            .tape
            .request("remote.destinations", &(ticket.clone(), &state_dir))
        {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.remote_choices_done(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let client = self.remote_client();
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "remote-destinations",
            move |outcome| {
                let _ = tx.send(IoEvent::Remote(RemoteEvent::Choices(Completion {
                    ticket,
                    outcome,
                })));
            },
            move |token| {
                let mut notes = Vec::new();
                let recent = match state_dir {
                    Some(root) => match crate::session::remotes::load(&root) {
                        Ok(recent) => recent,
                        Err(error) => {
                            notes.push(format!("remote history: {error}"));
                            Vec::new()
                        }
                    },
                    None => Vec::new(),
                };
                let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                let sources = strop_remote::HostSources::discover(home.as_deref());
                let enumeration = strop_remote::enumerate_hosts(&sources, &[]);
                if token.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::OwnerClosed);
                }
                notes.extend(enumeration.notes().iter().cloned());
                let mut items = Vec::new();
                let mut seen = std::collections::HashSet::new();
                for directory in recent {
                    seen.insert(directory.endpoint().clone());
                    items.push(destination_item(directory, "recent"));
                }
                for endpoint in client.connections() {
                    if seen.insert(endpoint.clone()) {
                        match RemoteFile::from_path(endpoint, "/".into()) {
                            Ok(directory) => items.push(destination_item(directory, "connected")),
                            Err(error) => notes.push(error.to_string()),
                        }
                    }
                }
                for candidate in enumeration.candidates() {
                    let endpoint =
                        match RemoteEndpoint::parse(&format!("ssh://{}", candidate.token())) {
                            Ok(endpoint) => endpoint,
                            Err(error) => {
                                notes.push(error.to_string());
                                continue;
                            }
                        };
                    if seen.insert(endpoint.clone()) {
                        match RemoteFile::from_path(endpoint, "/".into()) {
                            Ok(directory) => {
                                items.push(destination_item(directory, "SSH config/known hosts"))
                            }
                            Err(error) => notes.push(error.to_string()),
                        }
                    }
                }
                items.push(new_host_item());
                Outcome::Success(RemoteChoices {
                    items: items.into(),
                    notes,
                })
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub(super) fn remote_choices_done(&mut self, completion: Completion<PickerId, RemoteChoices>) {
        if self.remote.choices.as_ref() != Some(&completion.ticket) {
            return;
        }
        self.remote.choices = None;
        self.worker_handles.remove(&completion.ticket.request);
        let Some(glue) = self
            .picker
            .as_mut()
            .filter(|glue| glue.id == completion.ticket.key)
        else {
            return;
        };
        glue.picker.streaming = false;
        match completion.outcome {
            Outcome::Success(choices) => {
                glue.picker.items = choices.items;
                glue.picker.clear_results();
                if !choices.notes.is_empty() {
                    glue.picker.error = Some(choices.notes.join("; "));
                }
            }
            Outcome::Failed { failure, .. } => glue.picker.error = Some(failure.message),
            Outcome::Cancelled(_) => return,
        }
        self.request_picker_ranking();
    }

    pub(crate) fn revoke_remote_chooser(&mut self, picker: PickerId) {
        if !self
            .remote
            .choices
            .as_ref()
            .is_some_and(|ticket| ticket.key == picker)
        {
            return;
        }
        if let Some(ticket) = self.remote.choices.take() {
            if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                handle.cancel(CancelReason::OwnerClosed);
            }
        }
    }

    pub(crate) fn open_remote_address(&mut self) {
        self.set_picker(PickerGlue::diagnostics(Picker::new(
            Kind::RemoteAddress,
            Vec::new(),
            false,
        )));
    }

    pub(crate) fn accept_remote_address(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        let text = glue.picker.input.text.trim();
        let spelling = if text.starts_with("ssh://") {
            text.to_string()
        } else {
            format!("ssh://{text}")
        };
        let location = match RemoteEndpoint::parse(&spelling) {
            Ok(endpoint) => RemoteFile::from_path(endpoint, "/".into()).map(Into::into),
            Err(_) => RemoteLocation::parse(&spelling),
        };
        match location {
            Ok(location) => {
                self.close_picker();
                self.request_target(FileTarget::Remote(location), OpenIntent::RemoteDestination);
            }
            Err(error) => glue.picker.error = Some(error.to_string()),
        }
    }

    pub(crate) fn browse_remote_root(&mut self, home: bool) -> Result<(), String> {
        let endpoint = self
            .remote_endpoint()
            .ok_or("open a remote destination first")?;
        let location =
            RemoteLocation::parse(&format!("{endpoint}/{}", if home { "~/" } else { "" }))
                .map_err(|error| error.to_string())?;
        self.request_target(FileTarget::Remote(location), OpenIntent::Browse);
        Ok(())
    }
}

fn new_host_item() -> Item {
    Item {
        text: "Add a host…".into(),
        payload: Payload::RemoteConnect,
    }
}
fn destination_item(directory: RemoteFile, origin: &str) -> Item {
    Item {
        text: format!("{directory}  [{origin}]"),
        payload: Payload::RemoteDirectory(directory),
    }
}
