//! Picker glue: workers post onto the editor event loop, every stream
//! owned by an exact ticket (R9): registration precedes launch, every
//! request settles exactly once, and stale streams die at the handler
//! instead of against the model (0020 §2).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

use strop_core::worker::{CancelHandle, CancelReason, Load, Ticket, WorkerId};
use strop_picker::{spawn_files, GrepWorker, Item, Kind, Payload, Picker, PickerMsg};

use super::{Editor, Key};

mod accept;
mod drain;
mod preview;
pub(crate) mod ranking;
mod replace;
#[cfg(test)]
mod tests;

/// One picker instance's identity, allocated from the editor's worker
/// id pool when the picker opens. Every streaming request and preview
/// read binds to it — closing the picker invalidates them all at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct PickerId(pub WorkerId);

/// What one streaming picker request owns: which instance, against
/// which working directory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PickerKey {
    pub picker: PickerId,
    #[serde(with = "strop_core::path_serde")]
    pub cwd: PathBuf,
}

/// A worker message stamped with the request that produced it. Both
/// the TUI's forwarded events and the headless drain deliver these;
/// only the owning ticket may touch the model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PickerEvent {
    pub ticket: Ticket<PickerKey>,
    pub msg: PickerMsg,
}

/// One supervised preview read: the picker instance it serves and the
/// native path being read.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PreviewKey {
    pub picker: PickerId,
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
}

/// The terminal result of a preview request.
pub type PreviewResult = strop_core::worker::Completion<PreviewKey, preview::PreparedPreview>;

/// The live worker behind a streaming request.
pub(crate) enum PickerWorker {
    Files(CancelHandle),
    Grep(GrepWorker),
}

impl PickerWorker {
    pub(crate) fn cancel(self, reason: CancelReason) {
        match self {
            PickerWorker::Files(handle) => handle.cancel(reason),
            PickerWorker::Grep(worker) => worker.cancel(reason),
        }
    }
}

pub struct PickerGlue {
    pub picker: Picker,
    pub id: PickerId,
    /// The request owning the stream: set at launch, cleared by its
    /// terminal Finished event or by cancellation.
    pub(crate) active: Option<Ticket<PickerKey>>,
    /// Headless only: the active request's raw stream (the TUI gets a
    /// ticket-stamping bridge at launch instead).
    pub(crate) rx: Option<(Ticket<PickerKey>, Receiver<PickerMsg>)>,
    pub(crate) worker: Option<PickerWorker>,
    pub(crate) lsp_context: Option<strop_lsp::ReplyContext>,
    pub(crate) rank_worker: Option<strop_picker::RankingWorker<ranking::Key>>,
    pub rank_pending: Option<Ticket<ranking::Key>>,
    pub(crate) ranked_query: Option<String>,
    pub(crate) rank_alive: bool,
    pub(crate) accept_when_ranked: bool,
}

impl PickerGlue {
    /// A picker with no request yet: `Editor::set_picker` allocates the
    /// instance identity before the glue is installed; Files/grep
    /// requests are launched afterwards by `Editor::open_picker` and
    /// `picker_input_changed`. (LSP location lists never launch one.)
    pub fn diagnostics(picker: Picker) -> Self {
        Self {
            picker,
            id: PickerId(WorkerId::new(0)), // replaced on install
            active: None,
            rx: None,
            worker: None,
            lsp_context: None,
            rank_worker: None,
            rank_pending: None,
            ranked_query: None,
            rank_alive: false,
            accept_when_ranked: false,
        }
    }

    /// Revoke the active request without touching the model: cancel
    /// the worker (a queued terminal event is rejected later — it
    /// cannot regain authority) and drop the raw stream.
    fn revoke(&mut self, reason: CancelReason) {
        self.rx = None;
        if self.active.take().is_some() {
            if let Some(worker) = self.worker.take() {
                worker.cancel(reason);
            }
        }
    }
}

impl Editor {
    /// Install a picker: tears down any previous instance (revoking
    /// its streams and previews) and allocates a fresh identity from
    /// the worker id pool.
    pub(crate) fn set_picker(&mut self, mut glue: PickerGlue) {
        self.cancel_pending();
        self.close_picker();
        let id = match self.worker_ids.allocate() {
            Ok(id) => id,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        glue.id = PickerId(id);
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"picker","id":id.get(),
                "kind":glue.picker.kind.title().trim(),"streaming":glue.picker.streaming,
            })
        });
        self.picker = Some(glue);
        if self
            .picker
            .as_ref()
            .is_some_and(|glue| glue.picker.kind != Kind::RemoteAddress)
        {
            self.start_picker_ranking();
        }
    }

    pub fn open_picker(&mut self, kind: Kind) {
        if kind == Kind::RemoteHosts {
            self.open_remote_picker();
            return;
        }
        if kind == Kind::Jumps {
            self.open_jumps_picker();
            return;
        }
        if kind == Kind::RemoteAddress {
            self.open_remote_address();
            return;
        }
        let items = match kind {
            Kind::Buffers => self
                .mru
                .iter()
                .map(|&i| {
                    let name = match self.doc(i).buf.path.as_ref() {
                        Some(path) => path.to_string_lossy().into_owned(),
                        None => "[scratch]".into(),
                    };
                    Item {
                        badge: None,
                        text: name,
                        payload: Payload::Buffer(i),
                    }
                })
                .collect(),
            // Grep/Replace stream only once input registers a request;
            // Files launches its walk right after install.
            Kind::Files
            | Kind::Grep
            | Kind::Replace
            | Kind::RemoteHosts
            | Kind::RemoteAddress
            | Kind::CodeActions
            | Kind::Containers => vec![],
            Kind::Jumps => unreachable!("the jumplist builds its own items"),
            Kind::Symbols => vec![],
            Kind::Diagnostics | Kind::Locations => {
                unreachable!("location lists use PickerGlue::diagnostics")
            }
        };
        self.set_picker(PickerGlue::diagnostics(Picker::new(kind, items, false)));
        if kind == Kind::Files {
            self.launch_files_request();
        }
    }

    /// The jumplist as a menu (0047 §2): past newest-first, the current
    /// position marked, then the future; dead documents are filtered.
    pub(crate) fn open_jumps_picker(&mut self) {
        let mut items = Vec::new();
        for &entry in self.jumplist_past.iter().rev() {
            items.extend(jump_row(self, entry, "  "));
        }
        items.extend(jump_row(self, (self.current(), self.head()), "> "));
        for &entry in self.jumplist_future.iter().rev() {
            items.extend(jump_row(self, entry, "  "));
        }
        self.set_picker(PickerGlue::diagnostics(Picker::new(
            Kind::Jumps,
            items,
            false,
        )));
    }

    /// The files walk as an owned request. Registration precedes
    /// launch: the worker can only post onto its stream, and nothing
    /// reaches the model until the ticket is the active owner. Replay
    /// mode stops after registration (Main's service seam).
    fn launch_files_request(&mut self) {
        let Some(picker) = self.picker.as_ref().map(|glue| glue.id) else {
            return;
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
            key: PickerKey {
                picker,
                cwd: self.cwd.clone(),
            },
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.active = Some(ticket.clone());
            glue.picker.streaming = true;
        }
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"picker","source":"files","id":picker.0.get(),
                "request":request.get(),"cwd":self.cwd.to_string_lossy(),
            })
        });
        match self
            .tape
            .request("picker-files", &serde_json::json!({"ticket":ticket}))
        {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_picker_event(PickerEvent {
                    ticket,
                    msg: PickerMsg::Finished(strop_core::worker::Outcome::failed(
                        strop_core::worker::FailureKind::Protocol,
                        error.to_string(),
                    )),
                });
                return;
            }
        }
        let (tx, rx) = channel();
        let worker = spawn_files(self.cwd.clone(), tx);
        if let Some(glue) = self.picker.as_mut() {
            glue.worker = Some(PickerWorker::Files(worker));
        }
        self.attach_picker_stream(ticket, rx);
    }

    /// Hand a launched request's stream to the app event loop (TUI) or
    /// keep it for the headless drain.
    fn attach_picker_stream(&mut self, ticket: Ticket<PickerKey>, rx: Receiver<PickerMsg>) {
        let Some(app_tx) = self.app_tx.clone() else {
            if let Some(glue) = self.picker.as_mut() {
                glue.rx = Some((ticket, rx));
            }
            return;
        };
        if let Err(error) = drain::forward_picker_stream(rx, ticket.clone(), app_tx) {
            // the bridge thread could not start: settle the request now
            self.handle_picker_event(PickerEvent {
                ticket,
                msg: PickerMsg::Finished(strop_core::worker::Outcome::failed(
                    strop_core::worker::FailureKind::ThreadStart,
                    format!("picker bridge: {error}"),
                )),
            });
        }
    }

    /// Connect-time: hand any already-registered headless stream to
    /// the app channel (normally requests attach at launch).
    pub(crate) fn connect_picker_stream(&mut self, tx: &super::events::EventSender) {
        if let Some(glue) = &mut self.picker {
            if let Some((ticket, rx)) = glue.rx.take() {
                let _ = drain::forward_picker_stream(rx, ticket, tx.clone());
            }
        }
    }

    /// Close the picker: revoke its active request, stop its worker,
    /// and cancel/forget the previews it owns (failed reads become
    /// retryable on reopen; successful caches survive).
    pub fn close_picker(&mut self) {
        let Some(mut glue) = self.picker.take() else {
            return;
        };
        glue.revoke(CancelReason::OwnerClosed);
        self.revoke_remote_chooser(glue.id);
        self.revoke_picker_previews(glue.id);
        if glue.rank_alive {
            self.picker_ranking.retiring.insert(glue.id);
        }
        if let Some(worker) = glue.rank_worker.take() {
            if let Err(error) = worker.retire(glue.picker) {
                self.message = format!("picker cleanup failed: {error}");
            }
        }
    }

    pub fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// Cancel/forget every preview this picker instance owns. Running
    /// requests are cancelled; Failed/Cancelled loads and their blank
    /// cache entries are removed so an explicit reopen retries; Ready
    /// caches stay (a successful read is still a successful read).
    fn revoke_picker_previews(&mut self, picker: PickerId) {
        let mut cancelled = Vec::new();
        let mut forgotten = Vec::new();
        self.preview_loads.retain(|path, load| match load {
            Load::Running(ticket) if ticket.key.picker == picker => {
                cancelled.push(ticket.request);
                false
            }
            Load::Failed { key, .. } | Load::Cancelled { key, .. } if key.picker == picker => {
                forgotten.push(path.clone());
                false
            }
            _ => true,
        });
        for request in cancelled {
            if let Some(handle) = self.worker_handles.remove(&request) {
                handle.cancel(CancelReason::OwnerClosed);
            }
        }
        for path in forgotten {
            self.previews.remove(&path);
            self.analysis
                .forget(super::analysis::AnalysisTarget::Preview(path));
        }
    }

    pub(crate) fn feed_picker(&mut self, key: Key) {
        let Some(glue) = &mut self.picker else {
            return;
        };
        if key != Key::Enter {
            glue.accept_when_ranked = false;
        }
        let replace = glue.picker.kind == Kind::Replace;
        match key {
            Key::Esc => {
                if glue.picker.input_normal() {
                    self.close_picker();
                } else {
                    glue.picker.enter_normal();
                }
            }
            Key::Enter => self.accept_current_picker(),
            Key::Tab | Key::Backtab if replace => glue.picker.toggle_field(),
            // ctrl-o: the listed hits become an editable collection (0044).
            Key::CtrlO => self.open_collection_from_picker(),
            Key::CtrlD if replace => glue.picker.toggle_file_excluded(),
            Key::CtrlD => {}
            Key::CtrlX => {}
            Key::Backspace => {
                if glue.picker.input_normal() {
                    glue.picker.normal_key('h');
                } else if replace && glue.picker.field == strop_picker::Field::Replace {
                    glue.picker.pop_replace_char();
                } else {
                    glue.picker.pop_char();
                    self.picker_input_changed();
                }
            }
            Key::CtrlL => self.needs_repaint = true,
            Key::CtrlR | Key::CtrlW => {}
            Key::CtrlU | Key::CtrlF | Key::CtrlB | Key::CtrlV | Key::CtrlCaret => {}
            Key::Up => glue.picker.move_by(-1),
            Key::Down => glue.picker.move_by(1),
            Key::Tab => glue.picker.move_by(1),
            Key::Backtab => glue.picker.move_by(-1),
            Key::Left => glue.picker.caret_left(),
            Key::Right => glue.picker.caret_right(),
            Key::Char('j') if glue.picker.input_normal() => glue.picker.move_by(1),
            Key::Char('k') if glue.picker.input_normal() => glue.picker.move_by(-1),
            Key::Char(c) => {
                if glue.picker.input_normal() {
                    if glue.picker.normal_key(c) {
                        self.picker_input_changed();
                    }
                } else if replace && glue.picker.field == strop_picker::Field::Replace {
                    glue.picker.push_replace_char(c);
                } else {
                    glue.picker.push_char(c);
                    self.picker_input_changed();
                }
            }
        }
    }

    /// Bracketed paste while a picker is open edits the focused field
    /// (query, replacement or remote address); it never reaches the
    /// document behind the card. Multi-line payloads are rejected with
    /// a message — a dropped keystroke with no feedback reads as a
    /// broken terminal, not as an editor decision.
    pub(crate) fn paste_picker(&mut self, text: &str) {
        let Some(glue) = &mut self.picker else {
            return;
        };
        if text.contains(['\r', '\n']) {
            self.message = "picker input cannot contain a newline".into();
            return;
        }
        if glue.picker.paste(text) {
            self.picker_input_changed();
        }
    }

    pub(crate) fn accept_current_picker(&mut self) {
        if self
            .picker
            .as_ref()
            .is_some_and(|glue| glue.picker.kind == Kind::RemoteAddress)
        {
            self.accept_remote_address();
            return;
        }
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        let replacing = glue.picker.kind == Kind::Replace;
        if (replacing || glue.picker.current().is_none())
            && (glue.rank_pending.is_some() || glue.picker.streaming)
        {
            glue.accept_when_ranked = true;
            return;
        }
        glue.accept_when_ranked = false;
        if replacing {
            self.apply_replace();
            return;
        }
        let payload = glue.picker.current().map(|item| item.payload.clone());
        // RemoteHosts: the pinned "Add a host…" row keeps one meaning —
        // open the address box. Typed filter text comes along as the
        // draft, so a hostname that matched no listed destination isn't
        // lost (0.21.0 field report), and typing "Add" can't connect to
        // a host literally named "add".
        if glue.picker.kind == Kind::RemoteHosts && matches!(payload, Some(Payload::RemoteConnect))
        {
            let draft = {
                let text = glue.picker.input.text.trim();
                // Filter text that matches the pinned row's own label was
                // aimed AT the row ("Add"); only text that matched
                // nothing — a bare hostname — becomes the address draft.
                let aimed_at_row = glue
                    .picker
                    .current()
                    .is_some_and(|item| strop_picker::fuzzy_score(text, &item.text).is_some());
                (!aimed_at_row).then(|| text.to_string())
            };
            self.close_picker();
            self.open_remote_address();
            if let Some(draft) = draft.filter(|draft| !draft.is_empty()) {
                if let Some(glue) = self.picker.as_mut() {
                    glue.picker.paste(&draft);
                }
            }
            return;
        }
        let Some(payload) = payload else {
            self.message = "no matching entries".into();
            return;
        };
        let context = glue.lsp_context;
        self.close_picker();
        self.accept_picker(payload, context);
    }

    pub(crate) fn finish_pending_picker_accept(&mut self) {
        if self.picker.as_ref().is_some_and(|glue| {
            glue.accept_when_ranked
                && glue.rank_pending.is_none()
                && (glue.picker.kind != Kind::Replace || !glue.picker.streaming)
        }) {
            self.accept_current_picker();
        }
    }

    /// Grep/Replace: every input change is a new owned request — the
    /// previous one is superseded, its items/rows/exclusions cleared,
    /// and a fresh ticket + worker launched. Other kinds just refilter.
    fn picker_input_changed(&mut self) {
        let (query, picker) = {
            let Some(glue) = &mut self.picker else {
                return;
            };
            if glue.picker.kind == Kind::RemoteAddress {
                glue.picker.error = None;
                return;
            }
            if !matches!(glue.picker.kind, Kind::Grep | Kind::Replace) {
                self.request_picker_ranking();
                return;
            }
            let query = glue.picker.input.text.clone();
            let picker = glue.id;
            glue.revoke(CancelReason::Superseded);
            glue.picker.error = None;
            glue.picker.clear_items();
            glue.ranked_query = None;
            glue.rank_pending = None;
            (query, picker)
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                // no identity: settle as not streaming; the next
                // keystroke retries with a fresh allocation
                if let Some(glue) = self.picker.as_mut() {
                    glue.picker.streaming = false;
                }
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: PickerKey {
                picker,
                cwd: self.cwd.clone(),
            },
        };
        // registration precedes launch (replay stops here)
        if let Some(glue) = self.picker.as_mut() {
            glue.active = Some(ticket.clone());
            glue.picker.streaming = true;
        }
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"picker","source":"grep","id":picker.0.get(),
                "request":request.get(),"query":query,"streaming":true,
            })
        });
        match self.tape.request(
            "picker-grep",
            &serde_json::json!({"ticket":ticket,"query":query}),
        ) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_picker_event(PickerEvent {
                    ticket,
                    msg: PickerMsg::Finished(strop_core::worker::Outcome::failed(
                        strop_core::worker::FailureKind::Protocol,
                        error.to_string(),
                    )),
                });
                return;
            }
        }
        let (tx, rx) = channel();
        let worker = GrepWorker::spawn(&query, &self.cwd, tx);
        if let Some(glue) = self.picker.as_mut() {
            glue.worker = Some(PickerWorker::Grep(worker));
        }
        self.attach_picker_stream(ticket, rx);
    }
}

pub struct PreviewEntry {
    pub rope: ropey::Rope,
}

pub enum PreviewSource {
    Buffer(strop_core::id::DocumentId),
    Cached(PathBuf),
    Loading,
    Failed(String),
    Cancelled(CancelReason),
}

pub type Previews = HashMap<PathBuf, PreviewEntry>;

/// One jumplist row; dead documents drop out (0047 §2).
fn jump_row(
    editor: &Editor,
    (document, offset): (strop_core::id::DocumentId, usize),
    marker: &str,
) -> Option<Item> {
    let doc = editor.docs.get(document)?;
    let name = doc
        .buf
        .path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "[scratch]".into());
    let line = doc.buf.line_of(offset);
    let text: String = doc.buf.line_text(line).trim().chars().take(48).collect();
    Some(Item {
        badge: None,
        text: format!("{marker}{name}:{}  {text}", line + 1),
        payload: Payload::Jump { document, offset },
    })
}
