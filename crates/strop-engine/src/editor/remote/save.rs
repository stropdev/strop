//! Saving a remote document (plan 0090 §5): the whole text goes over the
//! endpoint's SFTP session to a temporary name and is renamed over the
//! file. `:w` is refused when the file on the host changed since it was
//! read (vim's "changed since reading"); `:w!` replaces whatever is
//! there; `:w ssh://…` (or a path on the same host) writes a copy and
//! never replaces an existing file without the `!`.
use crate::editor::document::DocumentSource;
use crate::editor::io::IoEvent;
use crate::editor::Editor;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, CancelReason, Completion, FailureKind, Outcome, Ticket, WorkerId};
use strop_remote::{Expect, Stamp};
use strop_workspace::{RemoteFile, RemoteLocation};

use super::RemoteEvent;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteWriteKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    /// Where the bytes go: the document's own file, or a save-as copy.
    pub file: RemoteFile,
    pub copy: bool,
    pub expect: Expect,
    pub focus: u64,
    pub close: bool,
    pub force: bool,
    #[serde(with = "strop_core::path_serde::option")]
    pub target: Option<PathBuf>,
}

#[derive(Default)]
pub(crate) struct WriteState {
    pending: HashMap<DocumentId, Ticket<RemoteWriteKey>>,
}

impl WriteState {
    pub(super) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn pending_for(&self, document: DocumentId) -> bool {
        self.pending.contains_key(&document)
    }
}

/// The save-as destination on the document's own host: an `ssh://`
/// URI, an absolute path, or a path relative to the file's directory.
fn destination(file: &RemoteFile, target: &Path) -> Result<RemoteFile, String> {
    let text = target.to_string_lossy();
    if text.starts_with("ssh://") {
        let location = RemoteLocation::parse(&text).map_err(|error| error.to_string())?;
        return location
            .absolute_file()
            .cloned()
            .ok_or_else(|| "save-as needs an absolute ssh:// path".into());
    }
    let path = match file.path().parent() {
        Some(parent) if target.is_relative() => parent.join(target),
        _ => target.to_path_buf(),
    };
    file.with_path(path).map_err(|error| error.to_string())
}

impl Editor {
    pub(crate) fn request_remote_save(
        &mut self,
        document: DocumentId,
        target: Option<PathBuf>,
        force: bool,
        close: bool,
    ) -> bool {
        match self.start_remote_save(document, target.clone(), force, close) {
            Ok(()) => true,
            Err(error) => {
                self.message = error.clone();
                let revision = self
                    .docs
                    .get(document)
                    .map(|document| document.buf.revision())
                    .unwrap_or_default();
                self.note_save_failure(
                    document, true, revision, target, force, close, false, error,
                );
                false
            }
        }
    }

    fn start_remote_save(
        &mut self,
        document: DocumentId,
        target: Option<PathBuf>,
        force: bool,
        close: bool,
    ) -> Result<(), String> {
        if self.remote.writes.pending.contains_key(&document) {
            return Err("write already in progress".into());
        }
        if self.remote_refresh_pending(document) {
            return Err("a reload of this file is pending".into());
        }
        let doc = self.docs.get(document).ok_or("no such remote document")?;
        let source = doc
            .remote_metadata()
            .ok_or("remote directories cannot be saved")?;
        if doc.buf.readonly && !force {
            return Err("readonly buffer; :w! writes anyway".into());
        }
        let (file, copy, expect) = match &target {
            Some(target) => {
                let file = destination(&source.file, target)?;
                let own = file == source.file;
                let expect = match (force, own) {
                    (true, _) => Expect::Any,
                    (false, true) => Expect::Unchanged(source.stamp),
                    (false, false) => Expect::Absent,
                };
                (file, !own, expect)
            }
            None if force => (source.file.clone(), false, Expect::Any),
            None => (source.file.clone(), false, Expect::Unchanged(source.stamp)),
        };
        let mut bytes = Vec::with_capacity(doc.buf.len_bytes());
        for chunk in doc.buf.snapshot().chunks() {
            bytes.extend_from_slice(chunk.as_bytes());
        }
        let request = self.worker_ids.allocate().map_err(|error| error.message)?;
        let ticket = Ticket {
            request,
            key: RemoteWriteKey {
                document,
                revision: doc.buf.revision(),
                file,
                copy,
                expect,
                focus: self.focus_epoch,
                close,
                force,
                target,
            },
        };
        self.remote.writes.pending.insert(document, ticket.clone());
        self.message = "saving".into();
        match self.tape.request("remote.write", &(&ticket, bytes.len())) {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => {
                self.remote.writes.pending.remove(&document);
                return Err(format!("remote write replay diverged: {error}"));
            }
        }
        let sender = self.io.tx.clone();
        let client = self.remote_client();
        let destination = ticket.key.file.clone();
        let expect = ticket.key.expect;
        let handle = worker::spawn(
            "remote-write",
            move |outcome| {
                let _ = sender.send(IoEvent::Remote(RemoteEvent::Write(Box::new(Completion {
                    ticket,
                    outcome,
                }))));
            },
            move |token| match client.save(&destination, bytes, expect, &token) {
                Ok(stamp) => Outcome::Success(stamp),
                Err(error) if error.is_cancellation() => {
                    Outcome::Cancelled(CancelReason::Dismissed)
                }
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
        Ok(())
    }

    pub(in crate::editor::remote) fn remote_write_done(
        &mut self,
        completion: Completion<RemoteWriteKey, Stamp>,
    ) {
        let key = completion.ticket.key.clone();
        let document = key.document;
        if self.remote.writes.pending.get(&document) != Some(&completion.ticket) {
            return;
        }
        self.remote.writes.pending.remove(&document);
        self.worker_handles.remove(&completion.ticket.request);
        match completion.outcome {
            Outcome::Success(_) if key.copy => {
                self.resolve_save_feedback(document, true);
                self.message = format!("written to {}", key.file);
            }
            Outcome::Success(stamp) => {
                self.resolve_save_feedback(document, true);
                let current = match self.docs.get_mut(document) {
                    Some(doc) => match &mut doc.source {
                        DocumentSource::Remote(source) if source.file == key.file => {
                            source.stamp = stamp;
                            doc.external_change = false;
                            doc.buf.acknowledge_saved_revision(key.revision)
                        }
                        _ => false,
                    },
                    None => false,
                };
                self.message = if current {
                    "written"
                } else {
                    "snapshot written; newer edits remain unsaved"
                }
                .into();
                if current
                    && key.close
                    && !self.docs.is_empty()
                    && self.current() == document
                    && self.focus_epoch == key.focus
                {
                    self.close_pane_or_buffer(false);
                }
            }
            Outcome::Failed { failure, .. } => {
                self.message = failure.message.clone();
                self.note_save_failure(
                    document,
                    true,
                    key.revision,
                    key.target.clone(),
                    key.force,
                    key.close,
                    true,
                    failure.message,
                );
            }
            Outcome::Cancelled(_) => {
                self.message =
                    "save cancelled; the file on the host may already be replaced — :e! shows it"
                        .into();
            }
        }
        self.finish_save_feedback(document);
    }

    pub(crate) fn remote_write_blocks_refresh(&self, document: DocumentId) -> bool {
        self.remote.writes.pending.contains_key(&document)
    }

    pub(crate) fn cancel_remote_write(&mut self, document: DocumentId) {
        if let Some(ticket) = self.remote.writes.pending.get(&document) {
            if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                handle.cancel(CancelReason::Dismissed);
            }
        }
    }

    /// A closed document's save in flight is abandoned with it.
    pub(crate) fn drop_remote_write(&mut self, document: DocumentId) {
        if let Some(ticket) = self.remote.writes.pending.remove(&document) {
            if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                handle.cancel(CancelReason::OwnerClosed);
            }
        }
    }

    pub(crate) fn remote_write_pending(&self, request: WorkerId) -> bool {
        self.remote
            .writes
            .pending
            .values()
            .any(|ticket| ticket.request == request)
    }

    pub fn remote_write_status(&self) -> Option<&'static str> {
        if self.docs.is_empty() {
            return None;
        }
        self.remote
            .writes
            .pending
            .contains_key(&self.current())
            .then_some("saving")
    }
}

#[cfg(test)]
mod tests {
    use super::destination;
    use std::path::Path;
    use strop_workspace::RemoteFile;

    #[test]
    fn save_as_targets_stay_on_the_documents_host() {
        let file = RemoteFile::parse("ssh://dev@box/srv/app/main.rs").unwrap();
        assert_eq!(
            destination(&file, Path::new("copy.rs"))
                .unwrap()
                .to_string(),
            "ssh://dev@box/srv/app/copy.rs"
        );
        assert_eq!(
            destination(&file, Path::new("/tmp/x")).unwrap().to_string(),
            "ssh://dev@box/tmp/x"
        );
        assert_eq!(
            destination(&file, Path::new("ssh://other/etc/y"))
                .unwrap()
                .to_string(),
            "ssh://other/etc/y"
        );
    }
}
