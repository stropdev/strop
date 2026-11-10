//! Editor-side remote workspace ownership. Transport actors live in
//! strop-remote; this module owns explicit connections, browsing and
//! saves. A connected host is editable: the SSH login is the one consent.
mod chooser;
mod commands;
mod controls;
mod history;
pub(crate) mod save;
#[cfg(test)]
mod tests;

use super::document::DocumentSource;
use super::Editor;
use std::collections::HashMap;
use strop_core::id::DocumentId;
use strop_core::worker::{Completion, Ticket, WorkerId};
use strop_remote::{ConnectionLease, RemoteClient};
use strop_workspace::{RemoteEndpoint, RemoteFile};

pub(crate) struct RemoteState {
    client: RemoteClient,
    controls: HashMap<WorkerId, ControlKey>,
    pins: HashMap<RemoteEndpoint, ConnectionLease>,
    choices: Option<Ticket<super::picker::PickerId>>,
    destination_write: Option<Ticket<RemoteFile>>,
    destination_queue: Vec<RemoteFile>,
    pub(crate) writes: save::WriteState,
}
impl Default for RemoteState {
    fn default() -> Self {
        Self {
            client: RemoteClient::new(),
            controls: HashMap::new(),
            pins: HashMap::new(),
            choices: None,
            destination_write: None,
            destination_queue: Vec::new(),
            writes: save::WriteState::default(),
        }
    }
}

impl RemoteState {
    pub(crate) fn control_pending(&self) -> bool {
        !self.controls.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RemoteControl {
    Connect(RemoteEndpoint),
    Disconnect(RemoteEndpoint),
    DisconnectAll,
    Connections,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ControlKey {
    pub document: DocumentId,
    pub focus: u64,
    pub operation: RemoteControl,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub enum ControlResult {
    Connected {
        endpoint: RemoteEndpoint,
        #[serde(skip)]
        lease: Option<ConnectionLease>,
    },
    Disconnected,
    Listing(String),
}
#[derive(serde::Serialize, serde::Deserialize)]
pub enum RemoteEvent {
    Control(Completion<ControlKey, ControlResult>),
    Choices(Completion<super::picker::PickerId, chooser::RemoteChoices>),
    DestinationWritten(Completion<RemoteFile, ()>),
    Write(Box<Completion<save::RemoteWriteKey, strop_remote::Stamp>>),
}

impl Editor {
    pub(crate) fn remote_client(&self) -> RemoteClient {
        self.remote.client.clone()
    }
    pub fn remote_file(&self) -> Option<&RemoteFile> {
        match &self.cur().source {
            DocumentSource::Remote(source) => Some(&source.file),
            _ => None,
        }
    }
    pub fn remote_endpoint(&self) -> Option<&RemoteEndpoint> {
        self.remote_file()
            .map(RemoteFile::endpoint)
            .or_else(|| {
                self.directory()
                    .and_then(|source| source.location.filesystem.endpoint())
            })
            .or_else(|| {
                self.cur()
                    .git_context()
                    .and_then(strop_git::GitContext::endpoint)
            })
    }
    pub(crate) fn remote_work_pending(&self) -> bool {
        !self.remote.controls.is_empty()
            || self.remote.choices.is_some()
            || self.remote.destination_write.is_some()
            || !self.remote.destination_queue.is_empty()
            || self.remote.writes.pending()
    }
    pub(crate) fn handle_remote_event(&mut self, event: RemoteEvent) {
        match event {
            RemoteEvent::Write(completion) => self.remote_write_done(*completion),
            RemoteEvent::Control(completion) => self.remote_control_done(completion),
            RemoteEvent::Choices(completion) => self.remote_choices_done(completion),
            RemoteEvent::DestinationWritten(completion) => {
                self.remote_destination_written(completion)
            }
        }
    }
    pub(crate) fn stop_remote_work(&mut self) {
        self.remote.pins.clear();
    }
}
