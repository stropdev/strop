//! Editor-side remote workspace ownership. Transport actors live in strop-remote;
//! this module owns view intent, periodic reads, explicit connections and browsing.
mod chooser;
mod commands;
mod controls;
mod directory;
pub(super) mod follow;
mod history;
pub(crate) mod save;
#[cfg(test)]
mod tests;
pub(crate) mod view;

use super::document::DocumentSource;
use super::io::Opened;
use super::{Document, Editor};
use std::collections::HashMap;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{Completion, Ticket, WorkerId};
use strop_remote::{ConnectionLease, ReadLimit, RemoteClient};
use strop_workspace::{RemoteEndpoint, RemoteFile};
pub(crate) use view::RemoteView;

pub(crate) struct RemoteState {
    client: RemoteClient,
    following: HashMap<DocumentId, FollowOwner>,
    controls: HashMap<WorkerId, ControlKey>,
    pins: HashMap<RemoteEndpoint, ConnectionLease>,
    filters: HashMap<WorkerId, DirectoryFilterKey>,
    choices: Option<Ticket<super::picker::PickerId>>,
    destination_write: Option<Ticket<RemoteFile>>,
    destination_queue: Vec<RemoteFile>,
    writes: save::WriteState,
}
impl Default for RemoteState {
    fn default() -> Self {
        Self {
            client: RemoteClient::new(),
            following: HashMap::new(),
            controls: HashMap::new(),
            pins: HashMap::new(),
            filters: HashMap::new(),
            choices: None,
            destination_write: None,
            destination_queue: Vec::new(),
            writes: save::WriteState::default(),
        }
    }
}
struct FollowOwner {
    ticket: Ticket<FollowKey>,
    read: Option<WorkerId>,
    limit: ReadLimit,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FollowKey {
    pub document: DocumentId,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FollowReadKey {
    pub document: DocumentId,
    pub owner: WorkerId,
    pub revision: BufferRevision,
}
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) enum FollowChange {
    Appended,
    Reset,
    Shrank,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum FollowUpdate {
    Unchanged,
    Window {
        opened: Box<Opened>,
        change: FollowChange,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum RemoteControl {
    Connect(RemoteEndpoint),
    Disconnect(RemoteEndpoint),
    DisconnectAll,
    Connections,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ControlKey {
    pub document: DocumentId,
    pub focus: u64,
    pub operation: RemoteControl,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum ControlResult {
    Connected {
        endpoint: RemoteEndpoint,
        #[serde(skip)]
        lease: Option<ConnectionLease>,
    },
    Disconnected,
    Listing(String),
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DirectoryFilterKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub query: String,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum RemoteEvent {
    Tick(Ticket<FollowKey>),
    Timer(Completion<FollowKey, ()>),
    Read(Box<Completion<FollowReadKey, FollowUpdate>>),
    Control(Completion<ControlKey, ControlResult>),
    Filter(Box<Completion<DirectoryFilterKey, Opened>>),
    Choices(Completion<super::picker::PickerId, chooser::RemoteChoices>),
    DestinationWritten(Completion<RemoteFile, ()>),
    Write(Box<Completion<save::RemoteWriteKey, save::RemoteWriteResult>>),
}

impl Editor {
    pub(crate) fn remote_client(&self) -> RemoteClient {
        self.remote.client.clone()
    }
    pub(crate) fn remote_file(&self) -> Option<&RemoteFile> {
        match &self.cur().source {
            DocumentSource::Remote(source) => Some(&source.file),
            DocumentSource::RemoteDirectory(source) => Some(&source.directory),
            _ => None,
        }
    }
    pub(crate) fn remote_endpoint(&self) -> Option<&RemoteEndpoint> {
        self.remote_file().map(RemoteFile::endpoint).or_else(|| {
            self.cur()
                .git_context()
                .and_then(strop_git::GitContext::endpoint)
        })
    }
    pub(crate) fn remote_window_complete(&self) -> bool {
        self.cur()
            .remote_metadata()
            .is_some_and(|source| source.window.is_complete())
            && !self.remote_following(self.current())
    }
    pub(crate) fn remote_directory(&self) -> Option<&super::document::RemoteDirectory> {
        self.cur().directory_metadata_ref()
    }
    pub(crate) fn remote_following(&self, document: DocumentId) -> bool {
        self.remote.following.contains_key(&document)
    }
    pub(crate) fn remote_work_pending(&self) -> bool {
        !self.remote.controls.is_empty()
            || !self.remote.filters.is_empty()
            || self.remote.choices.is_some()
            || self.remote.destination_write.is_some()
            || !self.remote.destination_queue.is_empty()
            || self.remote.writes.pending()
            || self
                .remote
                .following
                .values()
                .any(|owner| owner.read.is_some())
    }
    pub(crate) fn handle_remote_event(&mut self, event: RemoteEvent) {
        match event {
            RemoteEvent::Write(completion) => self.remote_write_done(*completion),
            RemoteEvent::Tick(ticket) => self.remote_follow_tick(ticket),
            RemoteEvent::Timer(completion) => self.remote_follow_timer_done(completion),
            RemoteEvent::Read(completion) => self.remote_follow_read(*completion),
            RemoteEvent::Control(completion) => self.remote_control_done(completion),
            RemoteEvent::Filter(completion) => self.remote_filter_done(*completion),
            RemoteEvent::Choices(completion) => self.remote_choices_done(completion),
            RemoteEvent::DestinationWritten(completion) => {
                self.remote_destination_written(completion)
            }
        }
    }
    pub(crate) fn stop_remote_work(&mut self) {
        let documents: Vec<_> = self.remote.following.keys().copied().collect();
        for document in documents {
            self.stop_remote_follow(document);
        }
        self.remote.pins.clear();
    }
}
