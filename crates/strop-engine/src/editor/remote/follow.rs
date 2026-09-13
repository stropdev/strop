//! Follow is an owned clock plus one finite read at a time. User input never waits.
use super::*;
use crate::editor::{io::IoEvent, Mode};
use crate::files::FileTarget;
use ropey::Rope;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;
use strop_core::worker::{self, CancelReason, FailureKind, Outcome};
use strop_remote::{ReadSelection, RemoteWindow};

const FOLLOW_INTERVAL: Duration = Duration::from_millis(500);

impl Editor {
    pub(crate) fn start_remote_follow(&mut self, document: DocumentId, limit: ReadLimit) {
        if self.remote_write_blocks_refresh(document)
            || self.docs.get(document).is_some_and(|doc| {
                doc.buf.dirty
                    || doc
                        .remote_metadata()
                        .is_some_and(|source| source.write.is_some())
            })
        {
            self.message = "follow requires a separate read-only snapshot".into();
            return;
        }
        if self
            .docs
            .get(document)
            .and_then(Document::remote_metadata)
            .is_none()
        {
            return;
        }
        self.stop_remote_follow(document);
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: FollowKey { document },
        };
        self.remote.following.insert(
            document,
            FollowOwner {
                ticket: ticket.clone(),
                read: None,
                limit,
            },
        );
        self.lsp_close_document(document);
        match self.tape.request("remote.follow.clock", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.remote.following.remove(&document);
                self.message = error.to_string();
                return;
            }
        }
        let tx = self.io.tx.clone();
        let ticks = tx.clone();
        let clock = ticket.clone();
        let handle = worker::spawn(
            "remote-follow-clock",
            move |outcome| {
                let _ = tx.send(IoEvent::Remote(RemoteEvent::Timer(Completion {
                    ticket,
                    outcome,
                })));
            },
            move |token| {
                let (wake, waiting) = mpsc::channel();
                if let Err(failure) = token.register_cancel_resource(move || {
                    let _ = wake.send(());
                    Ok(())
                }) {
                    return Outcome::Failed {
                        failure,
                        partial: None,
                    };
                }
                let outcome = loop {
                    if token.is_cancelled() {
                        break Outcome::Cancelled(CancelReason::Dismissed);
                    }
                    match waiting.recv_timeout(FOLLOW_INTERVAL) {
                        Err(RecvTimeoutError::Timeout) => {
                            if ticks
                                .send(IoEvent::Remote(RemoteEvent::Tick(clock.clone())))
                                .is_err()
                            {
                                break Outcome::failed(
                                    FailureKind::Disconnected,
                                    "follow event receiver closed",
                                );
                            }
                        }
                        _ => break Outcome::Cancelled(CancelReason::Dismissed),
                    }
                };
                token.clear_cancel_resource();
                outcome
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub(crate) fn stop_remote_follow(&mut self, document: DocumentId) -> bool {
        let Some(owner) = self.remote.following.remove(&document) else {
            return false;
        };
        for request in [Some(owner.ticket.request), owner.read]
            .into_iter()
            .flatten()
        {
            if let Some(handle) = self.worker_handles.remove(&request) {
                handle.cancel(CancelReason::Dismissed);
            }
        }
        true
    }

    pub(super) fn remote_follow_tick(&mut self, ticket: Ticket<FollowKey>) {
        let document = ticket.key.document;
        let Some(owner) = self.remote.following.get(&document) else {
            return;
        };
        if owner.ticket != ticket || owner.read.is_some() || self.finishing {
            return;
        }
        if self
            .pending
            .prompt()
            .is_some_and(|prompt| prompt.origin().pane.doc == document)
        {
            return;
        }
        if !self.docs.is_empty() && self.current() == document && self.mode != Mode::Normal {
            return;
        }
        let limit = owner.limit;
        let Some(doc) = self.docs.get(document) else {
            self.stop_remote_follow(document);
            return;
        };
        let Some(source) = doc.remote_metadata() else {
            self.stop_remote_follow(document);
            return;
        };
        let file = source.file.clone();
        let before = doc.buf.snapshot();
        let window = source.window;
        let revision = doc.buf.revision();
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                self.stop_remote_follow(document);
                return;
            }
        };
        let key = FollowReadKey {
            document,
            owner: ticket.request,
            revision,
        };
        let ticket = Ticket { request, key };
        if let Some(owner) = self.remote.following.get_mut(&document) {
            owner.read = Some(request);
        }
        match self.tape.request("remote.follow.read", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.remote_follow_read(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let client = self.remote_client();
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "remote-follow-read",
            move |outcome| {
                let _ = tx.send(IoEvent::Remote(RemoteEvent::Read(Box::new(Completion {
                    ticket,
                    outcome,
                }))));
            },
            move |cancel| match client.read(&file.into(), ReadSelection::Tail(limit), &cancel) {
                Ok(snapshot) => {
                    if window == snapshot.window && &before == snapshot.buffer.text() {
                        return Outcome::Success(FollowUpdate::Unchanged);
                    }
                    let change = classify(&before, window, snapshot.buffer.text(), snapshot.window);
                    let canonical = FileTarget::Remote(snapshot.file.clone().into());
                    let document = Document::remote_snapshot(snapshot, ReadSelection::Tail(limit));
                    Outcome::Success(FollowUpdate::Window {
                        opened: Box::new(Opened {
                            document,
                            canonical,
                        }),
                        change,
                    })
                }
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub(super) fn remote_follow_timer_done(&mut self, completion: Completion<FollowKey, ()>) {
        self.worker_handles.remove(&completion.ticket.request);
        if self
            .remote
            .following
            .get(&completion.ticket.key.document)
            .is_some_and(|owner| owner.ticket == completion.ticket)
        {
            self.stop_remote_follow(completion.ticket.key.document);
            if let Outcome::Failed { failure, .. } = completion.outcome {
                self.message = failure.message;
            }
        }
    }

    pub(super) fn remote_follow_read(
        &mut self,
        completion: Completion<FollowReadKey, FollowUpdate>,
    ) {
        let request = completion.ticket.request;
        let key = completion.ticket.key;
        let Some(owner) = self.remote.following.get_mut(&key.document) else {
            return;
        };
        if owner.ticket.request != key.owner || owner.read != Some(request) {
            return;
        }
        owner.read = None;
        self.worker_handles.remove(&request);
        if !self
            .docs
            .get(key.document)
            .is_some_and(|doc| doc.buf.revision() == key.revision)
        {
            return;
        }
        if self
            .pending
            .prompt()
            .is_some_and(|prompt| prompt.origin().pane.doc == key.document)
        {
            return;
        }
        match completion.outcome {
            Outcome::Success(FollowUpdate::Unchanged) => {}
            Outcome::Success(FollowUpdate::Window { opened, change }) => {
                if let Err(error) =
                    self.publish_source_snapshot(key.document, opened.document, true)
                {
                    self.stop_remote_follow(key.document);
                    self.message = error.to_string();
                    return;
                }
                self.message = match change {
                    FollowChange::Appended => "following remote log",
                    FollowChange::Reset => "remote log reset: content/window changed",
                    FollowChange::Shrank => "remote log reset: file shrank",
                }
                .into();
            }
            Outcome::Failed { failure, .. } => {
                self.stop_remote_follow(key.document);
                self.message = format!("follow paused: {} — :follow to resume", failure.message);
            }
            Outcome::Cancelled(_) => {}
        }
    }
}

fn classify(before: &Rope, old: RemoteWindow, after: &Rope, new: RemoteWindow) -> FollowChange {
    if new.file_size() < old.file_size() {
        return FollowChange::Shrank;
    }
    let start = old.start().get().max(new.start().get());
    let end = (old.start().get() + old.length().get()).min(new.start().get() + new.length().get());
    if new.file_size() > old.file_size() && start < end {
        let old_range = (start - old.start().get()) as usize..(end - old.start().get()) as usize;
        let new_range = (start - new.start().get()) as usize..(end - new.start().get()) as usize;
        let boundary = |rope: &Rope, offset| rope.char_to_byte(rope.byte_to_char(offset)) == offset;
        if boundary(before, old_range.start)
            && boundary(before, old_range.end)
            && boundary(after, new_range.start)
            && boundary(after, new_range.end)
            && before.byte_slice(old_range) == after.byte_slice(new_range)
        {
            return FollowChange::Appended;
        }
    }
    FollowChange::Reset
}
