//! File I/O is owned work. Only matching completions may publish into a view.
mod admission;
mod codec;
mod indent;
pub(super) mod native;
mod navigation;
pub(crate) mod open;
#[cfg(test)]
mod remote_tests;
mod save;
mod session;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod worker_tests;
use super::{Document, Editor};
use crate::files::FileTarget;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use strop_core::id::{BufferRevision, ByteColumn, DocumentId, LineIndex};
use strop_core::worker::{Completion, Outcome, Ticket, WorkerId};
use strop_core::SaveReceipt;

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
        child: strop_workspace::ResourceLocation,
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
    SearchHit(super::picker::ReplacementHit),
    LspLocation {
        context: strop_lsp::ReplyContext,
        position: strop_lsp::ServerPosition,
    },
    /// Open without focus (0044 v2): collection builds load sources in
    /// the background; the picker keeps focus and focus never moves.
    CollectionSource {
        owner: WorkerId,
    },
}
impl OpenIntent {
    fn requires_file(&self) -> bool {
        match self {
            Self::AtLine { .. }
            | Self::Grep { .. }
            | Self::SearchHit(_)
            | Self::LspLocation { .. } => true,
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

/// A frozen unconfirmed document store (0058 WK09): the prepared
/// operation and its unconfirmed receipt, retained so the next `:w`
/// verifies against fresh evidence instead of rewriting blindly. An
/// uncertain outcome preserves dirty text and never grants an automatic
/// retry or rollback.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoreAttempt {
    pub revision: BufferRevision,
    #[serde(with = "strop_core::path_serde::option")]
    pub origin: Option<PathBuf>,
    #[serde(with = "strop_core::path_serde")]
    pub target: PathBuf,
    /// The destination as the kernel wrote it (plain `:w` resolves the
    /// canonical spelling; save-as keeps the given name).
    #[serde(with = "strop_core::path_serde")]
    pub write_target: PathBuf,
    pub close: bool,
    pub force: bool,
    pub focus: u64,
    pub namespace: strop_worker_protocol::NamespaceIdentity,
    pub receipt: strop_workspace::operation::StepReceipt,
}

/// The save completion payload (0058 WK09): a committed receipt as
/// before, an unconfirmed outcome with its frozen attempt, or a
/// verification result for a previously unconfirmed store.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum SaveOutcome {
    Written(SaveReceipt),
    Unconfirmed {
        detail: String,
        attempt: Box<StoreAttempt>,
    },
    Verified {
        attempt: Box<StoreAttempt>,
        verified: strop_workspace::operation::VerifiedOutcome,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum IoEvent {
    Open(Box<Completion<OpenKey, Opened>>),
    Save(Box<Completion<SaveKey, SaveOutcome>>),
    ContainerWrite(
        Box<Completion<save::container::ContainerWriteKey, save::container::ContainerWriteResult>>,
    ),
    Native(Box<Completion<native::NativeKey, native::NativeResult>>),
    Remote(super::remote::RemoteEvent),
    DirectoryFilter(Box<Completion<super::directory::FilterKey, Opened>>),
    Filesystem(Box<super::filesystem::FsEvent>),
    Review(
        Box<
            Completion<
                super::changes::review::prepare::PreparationKey,
                super::changes::review::prepare::PreparedReview,
            >,
        >,
    ),
    Session {
        request: WorkerId,
        outcome: Outcome<()>,
    },
    /// A guarded clean-buffer reload triggered by a filesystem
    /// notification hint (0058 S7): observation rides the worker lease;
    /// publication re-checks document/binding/revision at completion.
    NotifyReload(Box<Completion<super::notify::ReloadKey, Option<Opened>>>),
    /// Draft checkpoint/recovery publications and store reads (0056 AR04).
    Recovery {
        request: WorkerId,
        outcome: Outcome<super::recovery::RecoveryReport>,
    },
}

pub struct IoState {
    pub tx: Sender<IoEvent>,
    pub rx: Option<Receiver<IoEvent>>,
    pub open: HashMap<WorkerId, OpenKey>,
    navigation: Option<WorkerId>,
    /// Frozen unconfirmed document stores (0058 WK09): a document with an
    /// entry here verifies on its next `:w` before any rewrite.
    store_attempts: HashMap<DocumentId, StoreAttempt>,
    saves: HashMap<DocumentId, Ticket<SaveKey>>,
    pub(crate) container_writes: save::container::ContainerWriteState,
    session: Option<WorkerId>,
    queued_session: Option<crate::session::SaveRequest>,
    native: HashMap<WorkerId, native::NativeKey>,
    pub session_error: Option<String>,
    pub(crate) format_warnings: HashMap<DocumentId, String>,
    /// `strop -`: the scratch document stdin streams into, until EOF.
    stdin: Option<DocumentId>,
}

impl Default for IoState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx: Some(rx),
            open: HashMap::new(),
            navigation: None,
            store_attempts: HashMap::new(),
            saves: HashMap::new(),
            container_writes: save::container::ContainerWriteState::default(),
            session: None,
            queued_session: None,
            native: HashMap::new(),
            session_error: None,
            format_warnings: HashMap::new(),
            stdin: None,
        }
    }
}

impl Editor {
    pub fn handle_io(&mut self, event: IoEvent) {
        super::trace::services::io(&event);
        match event {
            IoEvent::Native(completion) => self.handle_native(*completion),
            IoEvent::ContainerWrite(completion) => self.container_write_done(*completion),
            IoEvent::Remote(event) => self.handle_remote_event(event),
            IoEvent::Review(completion) => self.handle_review_prepared(*completion),
            IoEvent::DirectoryFilter(completion) => self.directory_filter_done(*completion),
            IoEvent::Filesystem(event) => self.handle_filesystem(*event),
            IoEvent::NotifyReload(completion) => self.notify_reload_done(*completion),
            IoEvent::Open(completion) => self.open_done(*completion),
            IoEvent::Save(completion) => {
                let request = completion.ticket.request;
                if self.io.saves.get(&completion.ticket.key.document) != Some(&completion.ticket) {
                    return;
                }
                let key = completion.ticket.key;
                self.io.saves.remove(&key.document);
                self.worker_handles.remove(&request);
                match completion.outcome {
                    Outcome::Success(SaveOutcome::Written(receipt)) => {
                        self.accept_store_receipt(&key, receipt);
                    }
                    Outcome::Success(SaveOutcome::Unconfirmed { detail, attempt }) => {
                        // Frozen evidence, dirty text preserved; the next
                        // :w verifies against fresh evidence (0058 WK09).
                        let revision = attempt.revision;
                        self.io.store_attempts.insert(key.document, *attempt);
                        self.message = format!(
                            "write outcome unconfirmed: {detail}; :w verifies before any rewrite"
                        );
                        self.note_unconfirmed_save(
                            key.document,
                            false,
                            revision,
                            self.message.clone(),
                        );
                    }
                    Outcome::Success(SaveOutcome::Verified { attempt, verified }) => {
                        self.store_verified(&key, *attempt, verified);
                    }
                    Outcome::Failed { failure, .. } => {
                        self.message = format!("write failed: {}", failure.message);
                        if let Some(revision) = self
                            .io
                            .store_attempt(key.document)
                            .map(|attempt| attempt.revision)
                        {
                            self.note_unconfirmed_save(
                                key.document,
                                false,
                                revision,
                                format!(
                                    "verification failed: {}; the original write remains unconfirmed",
                                    failure.message
                                ),
                            );
                        } else {
                            self.note_save_failure(
                                key.document,
                                false,
                                key.revision,
                                key.target.clone(),
                                key.force,
                                key.close,
                                true,
                                self.message.clone(),
                            );
                        }
                    }
                    Outcome::Cancelled(_) => {
                        self.message = "write cancelled".into();
                    }
                }
                self.finish_save_feedback(key.document);
            }
            IoEvent::Recovery { request, outcome } => {
                self.handle_recovery(request, outcome);
            }
            IoEvent::Session { request, outcome } => self.handle_session(request, outcome),
        }
    }

    pub fn io_pending(&self) -> bool {
        !self.io.open.is_empty()
            || self.review.preparing.is_some()
            || !self.directories.filters.is_empty()
            || self.filesystem.pending()
            || !self.io.saves.is_empty()
            || self.io.container_writes.pending()
            || self.io.session.is_some()
            || !self.io.native.is_empty()
            || self.remote_work_pending()
            || self.notify.pending()
            || self.recovery.pending()
    }
}

impl Editor {
    /// `strop -`: the current scratch buffer receives stdin. It reads as
    /// `stdin` and stays read-only while the pipe streams (a live
    /// `tail -f`); EOF hands it to the user like any scratch buffer.
    pub fn adopt_stdin(&mut self) {
        let document = self.current();
        {
            let mut doc = self.doc_mut(document);
            doc.buf.name = Some("stdin".into());
            doc.buf.set_readonly(strop_core::ReadonlyReason::Stdin);
        }
        self.io.stdin = Some(document);
    }

    /// Whether a stdin reader must be started (frontends ask once).
    pub fn stdin_streaming(&self) -> bool {
        self.io.stdin.is_some()
    }

    /// One read of stdin (`Some`) or its end (`None`). Text appends at
    /// the end as a system edit — never undo history, never dirty; a
    /// cursor on the last line follows the stream like `tail -f`.
    pub(crate) fn handle_stdin(&mut self, chunk: Option<String>) {
        let live = self.io.stdin.filter(|id| self.docs.get(*id).is_some());
        let Some(document) = live else {
            self.io.stdin = None;
            return;
        };
        let Some(text) = chunk else {
            self.io.stdin = None;
            let lines = {
                let mut doc = self.doc_mut(document);
                if doc.buf.readonly_reason == Some(strop_core::ReadonlyReason::Stdin) {
                    doc.buf.clear_readonly();
                }
                doc.buf.last_content_line() + 1
            };
            self.message = format!("stdin closed — {lines} lines, editable now");
            return;
        };
        let following = self.current() == document
            && self.buf().line_of(self.head()) >= self.buf().last_content_line();
        let end = self.doc(document).buf.len_bytes();
        let appended = self
            .doc_mut(document)
            .buf
            .system_edit()
            .replace(strop_core::Range::charwise(end, end), &text);
        if let Err(error) = appended {
            self.message = format!("stdin: {error}");
            return;
        }
        if following {
            let last = self.buf().last_content_line();
            self.set_head(self.buf().line_start(last));
            self.clamp_cursor();
        }
    }

    pub(crate) fn io_write_pending(&self, request: WorkerId) -> bool {
        self.io.session == Some(request)
            || self.recovery.write_pending(request)
            || self.remote_write_pending(request)
            || self.container_write_pending(request)
            || self.destination_write_pending(request)
            || self.filesystem.mutation_pending(request)
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
    pub fn io_status(&self) -> Option<&'static str> {
        if let Some(status) = self.remote_write_status() {
            return Some(status);
        }
        if let Some(status) = self.container_write_status() {
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
    pub(crate) fn save_pending_for(&self, document: DocumentId) -> bool {
        self.saves.contains_key(&document)
    }
    pub(crate) fn store_attempt(&self, document: DocumentId) -> Option<&StoreAttempt> {
        self.store_attempts.get(&document)
    }
    /// Frozen unconfirmed document stores (0058 WK09), for the shutdown
    /// report — in-memory save evidence is not persisted either.
    pub(crate) fn store_attempts(&self) -> impl Iterator<Item = (DocumentId, &StoreAttempt)> {
        self.store_attempts
            .iter()
            .map(|(document, attempt)| (*document, attempt))
    }
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
