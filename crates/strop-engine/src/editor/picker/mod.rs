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
mod query;
#[cfg(test)]
mod query_tests;
pub(crate) mod ranking;
mod replace;
pub use replace::checked_hit_range;
pub use replace::ReplacementHit;
pub(crate) mod search;
pub use search::SearchScope;
#[cfg(test)]
mod search_tests;
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
    /// Coalesce catalog/query changes while one rank snapshot is in flight.
    pub(crate) rank_dirty: bool,
    pub(crate) ranked_query: Option<String>,
    pub(crate) rank_alive: bool,
    pub(crate) accept_when_ranked: bool,
    /// The manual query-suggestion list (0051 R02): ctrl-space opens
    /// it; it owns accept/cancel keys until dismissed.
    pub suggestions: Option<SuggestionList>,
    pub query_highlights: Vec<strop_picker::query::HighlightSpan>,
    pub query_summary: String,
    pub(crate) query: Option<std::sync::Arc<strop_picker::query::SearchQuery>>,
    file_scope: Option<std::sync::Arc<strop_picker::query::SearchQuery>>,
    pub(crate) indent_target: Option<strop_core::id::DocumentId>,
    pub(crate) search: Option<search::SearchContext>,
    preview_witness: Option<preview::WitnessCheck>,
}

/// The visible suggestion list: static candidates from the query's own
/// parse position — never an LSP or a filesystem scan.
pub struct SuggestionList {
    pub items: Vec<strop_picker::query::suggest::Suggestion>,
    pub selected: usize,
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
            rank_dirty: false,
            ranked_query: None,
            rank_alive: false,
            accept_when_ranked: false,
            suggestions: None,
            query_highlights: Vec::new(),
            query_summary: String::new(),
            file_scope: None,
            query: None,
            indent_target: None,
            search: None,
            preview_witness: None,
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
        glue.preview_witness = None;
        glue.id = PickerId(id);
        if glue.picker.kind == Kind::Search && glue.search.is_none() {
            match self.new_search_context(SearchScope {
                root: strop_workspace::ResourceLocation::local(self.cwd.clone()),
            }) {
                Ok(context) => glue.search = Some(context),
                Err(error) => {
                    self.message = error;
                    return;
                }
            }
        }
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
        if kind == Kind::Search {
            self.open_search(false);
            return;
        }
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
        if kind == Kind::TabSize {
            self.open_tab_size_picker();
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
            | Kind::Search
            | Kind::RemoteHosts
            | Kind::RemoteAddress
            | Kind::CodeActions
            | Kind::Containers => vec![],
            Kind::Jumps => unreachable!("the jumplist builds its own items"),
            Kind::SearchOptions => unreachable!("search options build their own items"),
            Kind::TabSize => unreachable!("the tab-size selector builds its own items"),
            Kind::Symbols => vec![],
            Kind::Diagnostics | Kind::Locations => {
                unreachable!("location lists use PickerGlue::diagnostics")
            }
        };
        self.set_picker(PickerGlue::diagnostics(Picker::new(kind, items, false)));
        if kind == Kind::Files {
            self.picker_input_changed();
        }
    }

    /// `:search-options` (0051 R03): the hidden/ignore controls with
    /// their live values; Enter toggles and the row updates in place.
    pub(crate) fn open_search_options(&mut self) {
        let item = |setting: strop_picker::SearchSetting, on: bool| strop_picker::Item {
            badge: None,
            text: format!(
                "{}: {}",
                match setting {
                    strop_picker::SearchSetting::Hidden => "hidden (dotfiles)",
                    strop_picker::SearchSetting::RespectIgnore => "ignored entries",
                },
                match (setting, on) {
                    (strop_picker::SearchSetting::Hidden, true)
                    | (strop_picker::SearchSetting::RespectIgnore, false) => "include",
                    _ => "exclude",
                },
            ),
            payload: strop_picker::Payload::SearchOption(setting),
        };
        let items = vec![
            item(
                strop_picker::SearchSetting::Hidden,
                self.config.search_show_hidden,
            ),
            item(
                strop_picker::SearchSetting::RespectIgnore,
                self.config.search_respect_ignore,
            ),
        ];
        self.set_picker(PickerGlue::diagnostics(Picker::new(
            Kind::SearchOptions,
            items,
            false,
        )));
    }

    /// The jumplist as a menu (0047 §2): past newest-first, the current
    /// position marked, then the future; dead documents are filtered.
    pub(crate) fn open_jumps_picker(&mut self) {
        let mut items = Vec::new();
        for entry in self.jumplist_past.iter().rev() {
            items.extend(jump_row(self, entry, "  "));
        }
        items.extend(jump_row(self, &self.jump_record(), "> "));
        for entry in self.jumplist_future.iter().rev() {
            items.extend(jump_row(self, entry, "  "));
        }
        self.set_picker(PickerGlue::diagnostics(Picker::new(
            Kind::Jumps,
            items,
            false,
        )));
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
    /// retryable on reopen; retained successful bytes must be revalidated).
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
        if glue.picker.kind == Kind::Search {
            drop(glue.rank_worker.take());
            self.retain_search(glue);
        } else if let Some(worker) = glue.rank_worker.take() {
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
    /// bytes stay within the cache bound but are not current in a new picker.
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
        let search = glue.picker.kind == Kind::Search;
        let replace = search && glue.picker.replacement_visible;
        // the suggestion list owns accept/cancel while open (0051 R02)
        if glue.suggestions.is_some() {
            match key {
                Key::Up => {
                    let list = glue.suggestions.as_mut().unwrap();
                    list.selected = list.selected.saturating_sub(1);
                    return;
                }
                Key::Down | Key::Tab => {
                    let list = glue.suggestions.as_mut().unwrap();
                    list.selected = (list.selected + 1).min(list.items.len().saturating_sub(1));
                    return;
                }
                Key::Enter => {
                    self.accept_suggestion();
                    return;
                }
                Key::Esc => {
                    glue.suggestions = None;
                    return;
                }
                _ => {
                    glue.suggestions = None;
                }
            }
        }
        match key {
            Key::CtrlSpace => {
                self.open_suggestions();
            }
            Key::Esc => {
                if glue.picker.input_normal() {
                    let origin = glue.search.as_ref().map(|context| context.origin.clone());
                    self.close_picker();
                    if let Some(origin) =
                        origin.filter(|origin| self.docs.get(origin.document).is_some())
                    {
                        self.jump_to(origin);
                    }
                } else {
                    glue.picker.enter_normal();
                }
            }
            Key::Enter => self.accept_current_picker(),
            Key::Tab | Key::Backtab if replace => glue.picker.toggle_field(),
            // ctrl-o: the listed hits become an editable collection (0044).
            Key::CtrlO => self.open_collection_from_picker(),
            Key::CtrlD if search => {
                if glue.picker.toggle_file_excluded() {
                    self.search_intent_changed();
                } else {
                    self.message = "no source match selected".into();
                }
            }
            Key::CtrlX if search => {
                if glue.picker.toggle_excluded() {
                    self.search_intent_changed();
                } else {
                    self.message = "no source match selected".into();
                }
            }
            Key::CtrlD | Key::CtrlX => {}
            Key::Backspace => {
                if glue.picker.input_normal() {
                    glue.picker.normal_key('h');
                } else if replace && glue.picker.field == strop_picker::Field::Replace {
                    glue.picker.pop_replace_char();
                    self.search_intent_changed();
                } else {
                    glue.picker.pop_char();
                    self.picker_input_changed();
                }
            }
            Key::CtrlL => self.needs_repaint = true,
            Key::CtrlR if search => self.toggle_search_replacement(),
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
                        if replace && glue.picker.field == strop_picker::Field::Replace {
                            self.search_intent_changed();
                        } else {
                            self.picker_input_changed();
                        }
                    }
                } else if replace && glue.picker.field == strop_picker::Field::Replace {
                    glue.picker.push_replace_char(c);
                    self.search_intent_changed();
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
        if text.is_empty() {
            return;
        }
        let Some(glue) = &mut self.picker else {
            return;
        };
        if text.contains(['\r', '\n']) {
            self.message = "picker input cannot contain a newline".into();
            return;
        }
        if glue.picker.paste(text) {
            self.picker_input_changed();
        } else {
            self.search_intent_changed();
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
        if matches!(glue.picker.kind, Kind::Files | Kind::Search) {
            if let Some(error) = &glue.picker.error {
                self.message = error.clone();
                return;
            }
        }
        let replacing = glue.picker.kind == Kind::Search
            && glue.picker.replacement_visible
            && glue.picker.field == strop_picker::Field::Replace;
        if glue.rank_pending.is_some()
            || ((replacing || glue.picker.current().is_none()) && glue.picker.streaming)
        {
            glue.accept_when_ranked = true;
            return;
        }
        glue.accept_when_ranked = false;
        if replacing {
            self.prepare_search_review();
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
        // TabSize: every row is an IndentChoice; the typed filter text
        // rides along so the pinned custom row can validate it as a
        // width (the RemoteHosts draft pattern, 0.21.0).
        if glue.picker.kind == Kind::TabSize {
            let draft = glue.picker.input.text.trim().to_string();
            let Some(Payload::IndentChoice(choice)) = payload else {
                self.message = "no matching entries".into();
                return;
            };
            let Some(document) = glue.indent_target else {
                self.message = "indentation selector lost its source — reopen :tab-size".into();
                return;
            };
            self.close_picker();
            self.accept_indent_choice(document, choice, &draft);
            return;
        }
        let Some(payload) = payload else {
            self.message = "no matching entries".into();
            return;
        };
        if glue.picker.kind == Kind::Search {
            self.open_search_hit(payload);
            return;
        }
        let context = glue.lsp_context;
        self.close_picker();
        self.accept_picker(payload, context);
    }

    pub(crate) fn finish_pending_picker_accept(&mut self) {
        if self.picker.as_ref().is_some_and(|glue| {
            glue.accept_when_ranked
                && glue.rank_pending.is_none()
                && (!(glue.picker.kind == Kind::Search
                    && glue.picker.replacement_visible
                    && glue.picker.field == strop_picker::Field::Replace)
                    || !glue.picker.streaming)
        }) {
            self.accept_current_picker();
        }
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

/// One jumplist row; dead documents drop out (0047 §2). The payload
/// stays a plain destination — accepting a menu entry is a NEW jump
/// landing (0051 §7), not a ctrl-o view restore.
fn jump_row(editor: &Editor, record: &super::jumps::JumpRecord, marker: &str) -> Option<Item> {
    let doc = editor.docs.get(record.document)?;
    let name = doc
        .buf
        .path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "[scratch]".into());
    let line = doc.buf.line_of(record.offset.min(doc.buf.len_bytes()));
    let text: String = doc.buf.line_text(line).trim().chars().take(48).collect();
    Some(Item {
        badge: None,
        text: format!("{marker}{name}:{}  {text}", line + 1),
        payload: Payload::Jump {
            document: record.document,
            offset: record.offset,
        },
    })
}
