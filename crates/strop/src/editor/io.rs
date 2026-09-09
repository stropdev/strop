//! File I/O is owned work. Only matching completions may publish into a view.
mod codec;
pub(super) mod native;
mod remote;
#[cfg(test)]
mod remote_tests;
use super::{Document, Editor};
use crate::files::FileTarget;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use strop_core::id::{BufferRevision, ByteColumn, DocumentId, LineIndex};
use strop_core::worker::{self, Completion, FailureKind, Outcome, Ticket, WorkerId};
use strop_core::{Buffer, SaveReceipt};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OpenIntent {
    Switch {
        readonly: bool,
    },
    Split {
        vertical: bool,
    },
    AtLine {
        line: LineIndex,
    },
    Refresh,
    Browse,
    DirectoryParent {
        child: strop_workspace::RemoteFile,
    },
    RemoteDestination,
    RemoteView {
        view: super::remote::RemoteView,
        line: Option<LineIndex>,
    },
    Grep {
        line: LineIndex,
        column: ByteColumn,
    },
    LspLocation {
        context: strop_lsp::ReplyContext,
        position: strop_lsp::ServerPosition,
    },
    Replace {
        hits: Vec<(usize, usize, usize, String)>,
        replacement: String,
    },
}
impl OpenIntent {
    fn requires_file(&self) -> bool {
        match self {
            Self::AtLine { .. } | Self::Grep { .. } | Self::LspLocation { .. } => true,
            Self::RemoteView { view, line } => {
                line.is_some() || *view != super::remote::RemoteView::default()
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenKey {
    pub path: FileTarget,
    pub origin: DocumentId,
    pub revision: BufferRevision,
    pub focus: u64,
    pub intent: OpenIntent,
    pub selection: strop_remote::ReadSelection,
}

pub struct Opened {
    pub document: Document,
    pub canonical: FileTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SaveKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub focus: u64,
    pub close: bool,
    #[serde(with = "strop_core::path_serde::option")]
    pub target: Option<PathBuf>,
    pub force: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum IoEvent {
    Open(Box<Completion<OpenKey, Opened>>),
    Save(Box<Completion<SaveKey, SaveReceipt>>),
    Native(Box<Completion<native::NativeKey, native::NativeResult>>),
    Remote(super::remote::RemoteEvent),
    Session {
        request: WorkerId,
        outcome: Outcome<()>,
    },
}

pub struct IoState {
    pub tx: Sender<IoEvent>,
    pub rx: Option<Receiver<IoEvent>>,
    open: HashMap<WorkerId, OpenKey>,
    navigation: Option<WorkerId>,
    saves: HashMap<DocumentId, Ticket<SaveKey>>,
    session: Option<WorkerId>,
    queued_session: Option<crate::session::SaveRequest>,
    native: HashMap<WorkerId, native::NativeKey>,
    pub session_error: Option<String>,
}

impl Default for IoState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx: Some(rx),
            open: HashMap::new(),
            navigation: None,
            saves: HashMap::new(),
            session: None,
            queued_session: None,
            native: HashMap::new(),
            session_error: None,
        }
    }
}

impl Editor {
    pub fn request_open(&mut self, path: PathBuf, intent: OpenIntent) {
        self.request_target(FileTarget::Local(path), intent);
    }

    pub fn request_target(&mut self, target: FileTarget, intent: OpenIntent) {
        if matches!(intent, OpenIntent::Refresh) && self.remote_write_blocks_refresh(self.current())
        {
            self.message =
                "remote save pending or unconfirmed; settle or :remote verify before refresh"
                    .into();
            return;
        }
        let selection = match &intent {
            OpenIntent::RemoteView { view, .. } => view.selection(),
            OpenIntent::Refresh => self
                .cur()
                .remote_metadata()
                .map_or(strop_remote::ReadSelection::Full, |source| source.selection),
            _ => strop_remote::ReadSelection::Full,
        };
        let requires_file = intent.requires_file();
        let browse = matches!(
            intent,
            OpenIntent::Browse | OpenIntent::DirectoryParent { .. }
        );
        if matches!(target, FileTarget::Local(_))
            && (browse
                || matches!(
                    intent,
                    OpenIntent::RemoteView { .. } | OpenIntent::RemoteDestination
                ))
        {
            self.message = "range/tail/follow views require a remote target".into();
            return;
        }
        let path = match target {
            FileTarget::Local(path) => FileTarget::Local(self.cwd.join(path)),
            remote => remote,
        };
        if !matches!(intent, OpenIntent::Replace { .. }) {
            self.cancel_open(worker::CancelReason::Superseded);
        }
        let existing = self.docs.iter().find_map(|(id, document)| {
            (document.matches_target(&path)
                && document
                    .remote_metadata()
                    .is_none_or(|source| source.selection == selection))
            .then_some(id)
        });
        if let Some(id) = existing.filter(|&id| {
            !matches!(intent, OpenIntent::Refresh)
                && (!matches!(intent, OpenIntent::Browse | OpenIntent::RemoteDestination)
                    || self.doc(id).directory_metadata_ref().is_none())
        }) {
            if (requires_file && self.doc(id).directory_metadata_ref().is_some())
                || (browse && self.doc(id).directory_metadata_ref().is_none())
            {
                self.message = if browse {
                    "browse requires a directory"
                } else {
                    "this view requires a regular file"
                }
                .into();
                return;
            }
            self.finish_open(id, intent);
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let key = OpenKey {
            path: path.clone(),
            origin: self.current(),
            revision: self.buf().revision(),
            focus: self.focus_epoch,
            intent,
            selection,
        };
        if !matches!(key.intent, OpenIntent::Replace { .. }) {
            self.io.navigation = Some(request);
        }
        self.io.open.insert(request, key.clone());
        self.message = format!("loading {path}");
        let tx = self.io.tx.clone();
        let ticket = Ticket { request, key };
        match self.tape.request("io.open", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_io(IoEvent::Open(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return;
            }
        }
        let client = self.remote_client();
        let handle = worker::spawn(
            "strop-open",
            move |outcome| {
                let _ = tx.send(IoEvent::Open(Box::new(Completion { ticket, outcome })));
            },
            move |cancel| match path {
                FileTarget::Local(path) => match Buffer::open(&path) {
                    Ok(buffer) => {
                        let canonical = buffer
                            .file_identity()
                            .map_or_else(|| path.clone(), ToOwned::to_owned);
                        Outcome::Success(Opened {
                            document: Document::new(buffer),
                            canonical: FileTarget::Local(canonical),
                        })
                    }
                    Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                },
                FileTarget::Remote(location) => match if browse {
                    client
                        .list(&location, &cancel)
                        .map(strop_remote::RemoteResource::Directory)
                } else {
                    client.open(&location, selection, &cancel)
                } {
                    Ok(strop_remote::RemoteResource::File(snapshot)) => {
                        let canonical = FileTarget::Remote(snapshot.file.clone().into());
                        Outcome::Success(Opened {
                            document: Document::remote_snapshot(*snapshot, selection),
                            canonical,
                        })
                    }
                    Ok(strop_remote::RemoteResource::Directory(snapshot)) => {
                        if requires_file {
                            return Outcome::failed(
                                FailureKind::InvalidInput,
                                "range/tail/follow requires a regular file",
                            );
                        }
                        let canonical = FileTarget::Remote(snapshot.directory.clone().into());
                        Outcome::Success(Opened {
                            document: Document::remote_directory(snapshot),
                            canonical,
                        })
                    }
                    Err(error) if error.is_cancellation() => {
                        Outcome::Cancelled(worker::CancelReason::OwnerClosed)
                    }
                    Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                },
            },
        );
        self.worker_handles.insert(request, handle);
    }

    fn open_fresh(&self, key: &OpenKey) -> bool {
        if self.finishing {
            return false;
        }
        if matches!(key.intent, OpenIntent::Replace { .. }) {
            return true;
        }
        if let OpenIntent::LspLocation { context, .. } = &key.intent {
            if !self.lsp_context_fresh(context) {
                return false;
            }
        }
        !self.docs.is_empty()
            && self.current() == key.origin
            && self.focus_epoch == key.focus
            && self.buf().revision() == key.revision
    }

    fn finish_open(&mut self, document: DocumentId, intent: OpenIntent) {
        match intent {
            OpenIntent::LspLocation { context, position } => {
                self.finish_lsp_jump(document, position, context)
            }
            OpenIntent::Replace { hits, replacement } => {
                let (_, applied, stale) = self.replace_in_buffer(document, &hits, &replacement);
                self.message = format!("replaced {applied}; {stale} stale matches skipped");
                if applied > 0 {
                    self.request_save_document(document, None, true, false);
                }
            }
            OpenIntent::Split { vertical } => self.split_document(vertical, document),
            intent => {
                self.switch_to(document);
                self.set_head(0);
                self.view_mut().view_top = 0;
                match intent {
                    OpenIntent::Switch { readonly: true } => self.buf_mut().readonly = true,
                    OpenIntent::DirectoryParent { child } => {
                        if let Some(line) = self
                            .remote_directory()
                            .and_then(|directory| directory.line_for(&child))
                        {
                            self.set_head(self.buf().line_start(line));
                        }
                    }
                    OpenIntent::AtLine { line } => {
                        self.set_head(
                            self.buf()
                                .line_start(line.get().min(self.buf().last_content_line())),
                        );
                        self.run_motion("^");
                    }
                    OpenIntent::RemoteView { view, line } => {
                        if let Some(line) = line {
                            self.set_head(
                                self.buf()
                                    .line_start(line.get().min(self.buf().last_content_line())),
                            );
                            self.run_motion("^");
                        } else if view.follow_limit().is_some() {
                            self.set_head(super::remote::follow::last_position(self.buf().text()));
                        }
                        if let Some(limit) = view.follow_limit() {
                            self.start_remote_follow(document, limit);
                        }
                    }
                    OpenIntent::Grep { line, column } => {
                        let line = line.get().min(self.buf().last_content_line());
                        let offset = self
                            .buf()
                            .line_start(line)
                            .saturating_add(column.get())
                            .min(self.buf().line_end(line));
                        self.set_head(self.buf().clamp_boundary(offset));
                    }
                    _ => {}
                }
                self.remember_remote_destination();
                self.discover_git();
                self.lsp_maybe_attach();
            }
        }
    }

    pub fn request_save(&mut self, target: Option<PathBuf>, force: bool, close: bool) {
        self.request_save_document(self.current(), target, force, close);
    }

    pub(crate) fn request_save_document(
        &mut self,
        document: DocumentId,
        target: Option<PathBuf>,
        force: bool,
        close: bool,
    ) {
        if self.docs.get(document).is_some_and(|doc| {
            matches!(
                doc.source,
                super::document::DocumentSource::Remote(_)
                    | super::document::DocumentSource::RemoteDirectory(_)
            )
        }) {
            self.request_remote_save(document, target, force, close);
            return;
        }
        if target
            .as_ref()
            .and_then(|path| path.to_str())
            .is_some_and(|path| path.starts_with("ssh://"))
        {
            self.message = "remote save-as is unsupported; no local fallback".into();
            return;
        }
        if self.io.saves.contains_key(&document) {
            self.message = "write already in progress".into();
            return;
        }
        let Some(buffer) = self.docs.get(document).map(|doc| &doc.buf) else {
            return;
        };
        let revision = buffer.revision();
        let target = target.map(|path| self.cwd.join(path));
        let work = match buffer.prepare_save(target.clone(), force) {
            Ok(work) => work,
            Err(error) => {
                self.message = format!("write failed: {error}");
                return;
            }
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: SaveKey {
                document,
                revision,
                focus: self.focus_epoch,
                close,
                target,
                force,
            },
        };
        self.io.saves.insert(document, ticket.clone());
        self.message = "saving".into();
        match self.tape.request("io.save", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_io(IoEvent::Save(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return;
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-save",
            move |outcome| {
                let _ = tx.send(IoEvent::Save(Box::new(Completion { ticket, outcome })));
            },
            move |_| match work.execute() {
                Ok(receipt) => Outcome::Success(receipt),
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub(crate) fn request_session_save(&mut self) {
        let Some(work) = crate::session::capture_save(self) else {
            return;
        };
        if self.io.session.is_some() {
            // Serialized writes; newest queued capture replaces an unwritten one.
            self.io.queued_session = Some(work);
        } else {
            self.start_session_save(work);
        }
    }

    fn start_session_save(&mut self, work: crate::session::SaveRequest) {
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        self.io.session = Some(request);
        match self.tape.request("io.session", &request) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_io(IoEvent::Session {
                    request,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-session",
            move |outcome| {
                let _ = tx.send(IoEvent::Session { request, outcome });
            },
            move |_| match work.persist() {
                Ok(()) => Outcome::Success(()),
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub fn handle_io(&mut self, event: IoEvent) {
        super::trace::services::io(&event);
        match event {
            IoEvent::Native(completion) => self.handle_native(*completion),
            IoEvent::Remote(event) => self.handle_remote_event(event),
            IoEvent::Open(completion) => {
                let request = completion.ticket.request;
                if self.io.open.get(&request) != Some(&completion.ticket.key) {
                    return;
                }
                let Some(key) = self.io.open.remove(&request) else {
                    return;
                };
                self.worker_handles.remove(&request);
                if self.io.navigation == Some(request) {
                    self.io.navigation = None;
                }
                if !self.open_fresh(&key) {
                    return;
                }
                match completion.outcome {
                    Outcome::Success(mut opened) => {
                        if matches!(key.intent, OpenIntent::Refresh) {
                            self.revoke_remote_write(key.origin);
                            self.finish_remote_refresh(key.origin, opened.document);
                            return;
                        }
                        opened
                            .document
                            .set_return_point(super::document::ReturnPoint {
                                buffer: key.origin,
                                cursor: self.head(),
                                view_top: self.view_top(),
                                hscroll: self.view().hscroll,
                            });
                        let existing = self.docs.iter().find_map(|(id, document)| {
                            (document.matches_target(&opened.canonical)
                                && document
                                    .remote_metadata()
                                    .is_none_or(|source| source.selection == key.selection))
                            .then_some(id)
                        });
                        let id = if let Some(id) = existing {
                            if matches!(
                                key.intent,
                                OpenIntent::Browse | OpenIntent::RemoteDestination
                            ) && self.doc(id).directory_metadata_ref().is_some()
                            {
                                if let Err(error) =
                                    self.publish_remote_snapshot(id, opened.document, false)
                                {
                                    self.message = error.to_string();
                                    return;
                                }
                            }
                            id
                        } else {
                            let id = self.docs.insert(opened.document);
                            self.drop_stale_scratch(id);
                            self.generation += 1;
                            self.mru.push(id);
                            id
                        };
                        self.message.clear();
                        self.finish_open(id, key.intent);
                    }
                    Outcome::Failed { failure, .. } => {
                        self.message = format!("open {}: {}", key.path, failure.message)
                    }
                    Outcome::Cancelled(_) => {}
                }
            }
            IoEvent::Save(completion) => {
                let request = completion.ticket.request;
                if self.io.saves.get(&completion.ticket.key.document) != Some(&completion.ticket) {
                    return;
                }
                let key = completion.ticket.key;
                self.io.saves.remove(&key.document);
                self.worker_handles.remove(&request);
                match completion.outcome {
                    Outcome::Success(receipt) => {
                        let Some(document) = self.docs.get_mut(key.document) else {
                            return;
                        };
                        let previous_path = document.buf.path.clone();
                        let saved = document.buf.accept_save(receipt);
                        let renamed = previous_path != document.buf.path;
                        if renamed {
                            self.lsp_close_document(key.document);
                            if !self.docs.is_empty() && self.current() == key.document {
                                self.lsp_maybe_attach();
                            }
                        }
                        self.message = if saved {
                            "written"
                        } else {
                            "snapshot written; newer edits remain unsaved"
                        }
                        .into();
                        self.request_session_save();
                        if saved
                            && key.close
                            && !self.docs.is_empty()
                            && self.current() == key.document
                            && self.focus_epoch == key.focus
                        {
                            self.close_pane_or_buffer(false);
                        }
                    }
                    Outcome::Failed { failure, .. } => {
                        self.message = format!("write failed: {}", failure.message)
                    }
                    Outcome::Cancelled(_) => self.message = "write cancelled".into(),
                }
            }
            IoEvent::Session { request, outcome } => {
                if self.io.session != Some(request) {
                    return;
                }
                self.io.session = None;
                self.worker_handles.remove(&request);
                if let Outcome::Failed { failure, .. } = outcome {
                    self.message = format!("session save failed: {}", failure.message);
                    self.io.session_error = Some(self.message.clone());
                }
                if let Some(work) = self.io.queued_session.take() {
                    self.start_session_save(work);
                }
            }
        }
    }

    pub fn io_pending(&self) -> bool {
        !self.io.open.is_empty()
            || !self.io.saves.is_empty()
            || self.io.session.is_some()
            || !self.io.native.is_empty()
            || self.remote_work_pending()
    }
}

impl Editor {
    pub(crate) fn io_write_pending(&self, request: WorkerId) -> bool {
        self.io.session == Some(request)
            || self.remote_write_pending(request)
            || self.destination_write_pending(request)
            || self
                .io
                .saves
                .values()
                .any(|ticket| ticket.request == request)
            || self.io.native.get(&request).is_some_and(|key| {
                matches!(
                    key.operation,
                    native::Operation::Trust { .. } | native::Operation::TrustRemote { .. }
                )
            })
    }
    pub(crate) fn io_status(&self) -> Option<&'static str> {
        if let Some(status) = self.remote_write_status() {
            return Some(status);
        }
        if !self.io.saves.is_empty() {
            Some("saving")
        } else if !self.io.open.is_empty() {
            Some("loading")
        } else {
            None
        }
    }

    pub(crate) fn remote_refresh_pending(&self, document: DocumentId) -> bool {
        self.io
            .open
            .values()
            .any(|key| key.origin == document && matches!(key.intent, OpenIntent::Refresh))
    }
}

impl IoState {
    /// In-flight native tickets — tests answer a tape-suppressed
    /// launch by feeding `handle_io` a crafted completion.
    #[cfg(test)]
    pub(crate) fn native_tickets(&self) -> Vec<Ticket<native::NativeKey>> {
        self.native
            .iter()
            .map(|(request, key)| Ticket {
                request: *request,
                key: key.clone(),
            })
            .collect()
    }
}
