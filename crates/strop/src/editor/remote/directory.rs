//! Directory rows retain native targets; filtering publishes a fresh real buffer
//! without copying the entry tree on the input thread.
use super::*;
use crate::editor::{
    document::RemoteDirectory,
    io::{IoEvent, OpenIntent},
    Key,
};
use crate::files::FileTarget;
use strop_core::id::LineIndex;
use strop_core::worker::{self, CancelReason, FailureKind, Outcome};

impl Editor {
    pub(crate) fn remote_directory_key(&mut self, key: Key) -> bool {
        if !matches!(key, Key::Enter | Key::Backspace) {
            return false;
        }
        let Some(directory) = self.remote_directory() else {
            return false;
        };
        let line = self.buf().line_of(self.head());
        let target = if key == Key::Backspace || line == 0 {
            directory.parent().map_err(|error| error.to_string())
        } else {
            Ok(directory
                .entry(LineIndex::new(line))
                .map(|entry| entry.file.clone()))
        };
        match target {
            Ok(Some(file)) => self.request_target(
                FileTarget::Remote(file.into()),
                OpenIntent::Switch { readonly: true },
            ),
            Ok(None) => self.message = "no remote entry at this position".into(),
            Err(message) => self.message = message,
        }
        true
    }
    pub(super) fn filter_remote_directory(&mut self, query: String) -> Result<(), String> {
        let source = self
            .remote_directory()
            .ok_or(":filter requires a remote directory buffer")?;
        let mut replacement = RemoteDirectory {
            directory: source.directory.clone(),
            entries: source.entries.clone(),
            visible: Vec::new(),
            filter: query.clone(),
            connection: source.connection.clone(),
            return_to: source.return_to.clone(),
        };
        let document = self.current();
        self.cancel_remote_filter(document);
        let request = self.worker_ids.allocate().map_err(|error| error.message)?;
        let key = DirectoryFilterKey {
            document,
            revision: self.buf().revision(),
            query,
        };
        let ticket = Ticket {
            request,
            key: key.clone(),
        };
        self.remote.filters.insert(request, key);
        match self.tape.request("remote.directory.filter", &ticket) {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => {
                self.remote_filter_done(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return Ok(());
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "remote-directory-filter",
            move |outcome| {
                let _ = tx.send(IoEvent::Remote(RemoteEvent::Filter(Box::new(Completion {
                    ticket,
                    outcome,
                }))));
            },
            move |token| {
                for (index, entry) in replacement.entries.iter().enumerate() {
                    if token.is_cancelled() {
                        return Outcome::Cancelled(CancelReason::Superseded);
                    }
                    let name = entry
                        .file
                        .path()
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    if name.contains(&replacement.filter) {
                        replacement.visible.push(index);
                    }
                }
                let buffer = strop_core::Buffer::from_text(&replacement.text());
                let canonical = FileTarget::Remote(replacement.directory.clone().into());
                Outcome::Success(Opened {
                    document: Document::directory(buffer, replacement),
                    canonical,
                })
            },
        );
        self.worker_handles.insert(request, handle);
        Ok(())
    }
    pub(crate) fn cancel_remote_filter(&mut self, document: DocumentId) {
        let requests: Vec<_> = self
            .remote
            .filters
            .iter()
            .filter_map(|(&request, key)| (key.document == document).then_some(request))
            .collect();
        for request in requests {
            self.remote.filters.remove(&request);
            if let Some(handle) = self.worker_handles.remove(&request) {
                handle.cancel(CancelReason::Superseded);
            }
        }
    }
    pub(super) fn remote_filter_done(
        &mut self,
        completion: Completion<DirectoryFilterKey, Opened>,
    ) {
        let request = completion.ticket.request;
        if self.remote.filters.get(&request) != Some(&completion.ticket.key) {
            return;
        }
        self.remote.filters.remove(&request);
        self.worker_handles.remove(&request);
        let key = completion.ticket.key;
        if self.finishing
            || !self.docs.get(key.document).is_some_and(|doc| {
                doc.buf.revision() == key.revision && doc.directory_metadata_ref().is_some()
            })
        {
            return;
        }
        match completion.outcome {
            Outcome::Success(opened) => {
                if let Err(error) =
                    self.publish_remote_snapshot(key.document, opened.document, false)
                {
                    self.message = error.to_string();
                }
            }
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => {}
        }
    }
}
