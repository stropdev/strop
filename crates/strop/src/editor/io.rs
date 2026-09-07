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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenKey {
    pub path: FileTarget,
    pub origin: DocumentId,
    pub revision: BufferRevision,
    pub focus: u64,
    pub intent: OpenIntent,
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
    Native(Completion<native::NativeKey, native::NativeResult>),
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
        let path = match target {
            FileTarget::Local(path) => FileTarget::Local(self.cwd.join(path)),
            remote => remote,
        };
        if !matches!(intent, OpenIntent::Replace { .. }) {
            self.cancel_open(worker::CancelReason::Superseded);
        }
        let existing = self
            .docs
            .iter()
            .find_map(|(id, document)| document.matches_target(&path).then_some(id));
        if let Some(id) = existing.filter(|_| !matches!(intent, OpenIntent::Refresh)) {
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
                FileTarget::Remote(file) => match strop_remote::read(&file, &cancel) {
                    Ok(buffer) => Outcome::Success(Opened {
                        document: Document::remote(buffer, file.clone()),
                        canonical: FileTarget::Remote(file),
                    }),
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
            if !self.lsp_reply_fresh(context) {
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
                    OpenIntent::AtLine { line } => {
                        self.set_head(
                            self.buf()
                                .line_start(line.get().min(self.buf().last_content_line())),
                        );
                        self.run_motion("^");
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
        if self
            .docs
            .get(document)
            .is_some_and(|doc| matches!(doc.source, super::document::DocumentSource::Remote(_)))
        {
            self.message = "remote snapshots are read-only; remote writes are not supported".into();
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
            IoEvent::Native(completion) => self.handle_native(completion),
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
                    Outcome::Success(opened) => {
                        if matches!(key.intent, OpenIntent::Refresh) {
                            self.finish_remote_refresh(key.origin, opened.document);
                            return;
                        }
                        let existing = self.docs.iter().find_map(|(id, document)| {
                            document.matches_target(&opened.canonical).then_some(id)
                        });
                        let id = existing.unwrap_or_else(|| {
                            let id = self.docs.insert(opened.document);
                            self.drop_stale_scratch(id);
                            self.generation += 1;
                            self.mru.push(id);
                            id
                        });
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
    }
}

impl Editor {
    pub(crate) fn io_write_pending(&self, request: WorkerId) -> bool {
        self.io.session == Some(request)
            || self
                .io
                .saves
                .values()
                .any(|ticket| ticket.request == request)
            || self
                .io
                .native
                .get(&request)
                .is_some_and(|key| matches!(key.operation, native::Operation::Trust { .. }))
    }
    pub(crate) fn io_status(&self) -> Option<&'static str> {
        if !self.io.saves.is_empty() {
            Some("saving")
        } else if !self.io.open.is_empty() {
            Some("loading")
        } else {
            None
        }
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
