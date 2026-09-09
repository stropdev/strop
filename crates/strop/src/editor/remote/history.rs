//! Successful directory choices persist serially. The UI holds only a bounded
//! pending MRU; no contents or implicit reconnect permission enter this store.
use super::{Editor, RemoteEvent};
use crate::editor::io::IoEvent;
use strop_core::worker::{self, Completion, FailureKind, Outcome, Ticket, WorkerId};
use strop_remote::RemoteFile;

impl Editor {
    pub(crate) fn remember_remote_destination(&mut self) {
        let directory = if let Some(source) = self.cur().directory_metadata_ref() {
            source.directory.clone()
        } else if let Some(source) = self.cur().remote_metadata() {
            let Some(parent) = source.file.path().parent() else {
                return;
            };
            match source.file.with_path(parent.to_owned()) {
                Ok(directory) => directory,
                Err(error) => {
                    self.message = error.to_string();
                    return;
                }
            }
        } else {
            return;
        };
        if self.state_dir.is_none() {
            return;
        }
        if self
            .remote
            .destination_write
            .as_ref()
            .is_some_and(|ticket| ticket.key == directory)
        {
            return;
        }
        self.remote
            .destination_queue
            .retain(|item| item.endpoint() != directory.endpoint());
        self.remote.destination_queue.push(directory);
        if self.remote.destination_queue.len() > crate::session::remotes::RETAINED {
            self.remote.destination_queue.remove(0);
        }
        self.start_destination_write();
    }

    fn start_destination_write(&mut self) {
        if self.remote.destination_write.is_some() || self.remote.destination_queue.is_empty() {
            return;
        }
        let Some(root) = self.state_dir.clone() else {
            return;
        };
        let directory = self.remote.destination_queue.remove(0);
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: directory.clone(),
        };
        self.remote.destination_write = Some(ticket.clone());
        match self
            .tape
            .request("remote.destination.remember", &(ticket.clone(), &root))
        {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.remote_destination_written(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "remote-destination-write",
            move |outcome| {
                let _ = tx.send(IoEvent::Remote(RemoteEvent::DestinationWritten(
                    Completion { ticket, outcome },
                )));
            },
            move |_| match crate::session::remotes::remember(&root, directory) {
                Ok(()) => Outcome::Success(()),
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub(crate) fn destination_write_pending(&self, request: WorkerId) -> bool {
        self.remote
            .destination_write
            .as_ref()
            .is_some_and(|ticket| ticket.request == request)
    }

    pub(super) fn remote_destination_written(&mut self, completion: Completion<RemoteFile, ()>) {
        if self.remote.destination_write.as_ref() != Some(&completion.ticket) {
            return;
        }
        self.remote.destination_write = None;
        self.worker_handles.remove(&completion.ticket.request);
        if let Outcome::Failed { failure, .. } = completion.outcome {
            self.message = format!("remote destination was not remembered: {}", failure.message);
        }
        self.start_destination_write();
    }
}
