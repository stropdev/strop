//! The editor state: modes, pending keys, named registers, buffers.
//! One input path for TUI and headless — both call `feed`.
//!
//! Mode handlers live beside this file: `normal`, `visual`, `insert`.

mod blame;
pub mod block;
#[cfg(test)]
pub mod conformance;
#[cfg(test)]
pub mod contract_probes;
pub mod trace;

pub(crate) mod analysis;
mod cursor;
mod diagnostics;
pub(crate) mod resolution;
pub use diagnostics::DocumentDiagnostics;
mod dive;
mod document;
pub mod events;
mod explain;
mod git;
mod git_memory;
mod help;
mod input;
mod insert;
pub(crate) mod io;
mod jumps;
pub(crate) mod keys;
mod lsp;
pub mod macros;
#[cfg(test)]
mod multicursor_tests;
pub(crate) mod normal;
mod panes;
pub(crate) mod pending;
mod permalink;
mod picker;
mod registers;
pub(crate) mod remote;
mod remote_completion;
mod shell;
pub mod transact;
mod undo;
pub mod view;
mod visual;
mod workspaces;

pub use document::Document;
pub use document::{DiffRow, Surface};
pub use git_memory::{git_channel, BlameGutter, GitJob};
#[cfg(test)]
pub(crate) use git_memory::{CommitFiles, PreparedDiff, PreparedFiles};
pub use panes::{LayoutDir, Pane};
pub use picker::{PickerGlue, PreviewKey, PreviewResult, PreviewSource, Previews};
pub use registers::{ClipboardKey, ClipboardResult, Register};
pub use shell::{ShellIntent, ShellKey, ShellResult};

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use strop_core::{Buffer, Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    /// ctrl-v: the rectangle selection (0017).
    VisualBlock,
}

impl Mode {
    pub fn chip(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "V-LINE",
            Mode::VisualBlock => "V-BLOCK",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Key {
    Char(char),
    Esc,
    Enter,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Tab,
    Backtab,
    /// Replace picker: exclude the row's whole file (vscode's toggle).
    CtrlD,
    CtrlR,
    /// vim's jump-back (ctrl-i forward is Tab in a terminal).
    CtrlO,
    CtrlW,
    /// Replace picker: exclude/include the selected match (0007 §2).
    CtrlX,
    /// vim ctrl-u/ctrl-f/ctrl-b: half/full page up.
    CtrlU,
    CtrlF,
    CtrlB,
    /// vim ctrl-^: alternate buffer.
    CtrlCaret,
    /// vim ctrl-v: visual block mode.
    CtrlV,
    /// vim ctrl-l: force a full terminal repaint (desync recovery).
    CtrlL,
}

pub const FLASH_FOR: Duration = Duration::from_millis(280);

pub struct Editor {
    pub docs: strop_core::id::Arena<strop_core::id::DocumentKind, Document>,
    pub(crate) trace_documents:
        HashMap<strop_core::id::DocumentId, strop_core::diagnostics::BufferTraceId>,
    pub(crate) io: io::IoState,
    pub(crate) remote: remote::RemoteState,
    pub(crate) remote_completion: remote_completion::RemoteCompletionState,
    pub(crate) worker_ids: strop_core::worker::WorkerIds,
    pub(crate) worker_handles:
        HashMap<strop_core::worker::WorkerId, strop_core::worker::CancelHandle>,
    pub(crate) focus_epoch: u64,
    pub(crate) finishing: bool,
    pub(crate) tape: std::rc::Rc<strop_trace::replay::Tape>,
    pub git_view: strop_core::worker::WorkerId,
    pub git_discovery: strop_core::worker::Load<git_memory::ContextKey>,
    pub hunk_load: strop_core::worker::Load<git_memory::HunkKey>,
    pub hunks_untracked: bool,
    pub log_requests:
        HashMap<strop_core::id::DocumentId, strop_core::worker::Ticket<git_memory::LogKey>>,
    pub card_request: Option<strop_core::worker::Ticket<git_memory::CardKey>>,
    pub dive_requests:
        HashMap<strop_core::id::DocumentId, strop_core::worker::Ticket<git_memory::DiveKey>>,
    pub git_mutations: std::collections::VecDeque<git_memory::GitMutation>,
    pub git_mutation: Option<strop_core::worker::Ticket<git_memory::MutationKey>>,
    /// vim's jumplist (ctrl-o/ctrl-i): past/future stacks of
    /// (document, byte offset) (jumps.rs).
    pub jumplist_past: Vec<(strop_core::id::DocumentId, usize)>,
    pub jumplist_future: Vec<(strop_core::id::DocumentId, usize)>,
    pub mode: Mode,
    pub(crate) pending: pending::PendingInput,
    /// The input walker (0008 stage 2): typed parser state for
    /// counts/registers/operators/prefixes — pending stays for the
    /// free-text lines only.
    pub walker: input::Walker,
    /// `Space u` browser state (editor/undo.rs); None when closed.
    pub undo_browser: Option<undo::UndoBrowser>,
    /// Last `f/F/t/T` find: (char, backward, till). `;` and `,` replay it.
    pub last_find: Option<(char, bool, bool)>,
    /// Armed by `/`/`?`/`*`/`#` searches. `n`/`N` replay it; the render
    /// highlights matches persistently (rootle: current match underlined).
    pub last_search: Option<LastSearch>,
    pub registers: HashMap<char, Register>,
    /// Marks: char → (document, byte offset). `m{a}` sets, `'{a}` jumps.
    pub marks: HashMap<char, (strop_core::id::DocumentId, usize)>,
    pub flash: Option<(Range, strop_trace::replay::Tick)>,
    pub message: String,
    pub should_quit: bool,
    /// ctrl-c is armed after the first warn (0015 quit policy).
    pub ctrl_c_armed: bool,
    /// Last visual range for `gv` (recorded per visual-mode key).
    pub last_visual: Option<(usize, usize)>,
    /// Where the last insert session was for `gi`.
    pub last_insert_pos: Option<usize>,
    /// g;/g, walk: (index, history depth it was taken at) — a new
    /// commit invalidates the walk.
    pub change_idx: Option<(usize, usize)>,
    /// Text area height in rows — the render loop feeds it via
    /// scroll_to_cursor; viewport motions read it.
    pub view_rows: usize,
    /// Macro recording (0016): the register being recorded into.
    pub recording: Option<char>,
    /// The app event channel (0018): set by connect_events; late LSP
    /// attaches forward through it.
    pub app_tx: Option<events::EventSender>,
    pub(crate) lsp_state: lsp::state::LspState,
    /// Recorded macros: register → key events.
    pub macros: std::collections::HashMap<char, Vec<Key>>,
    /// The last replayed macro register (@@).
    pub last_macro: Option<char>,
    /// The rows and cell edge owned by an ongoing block insert/change.
    pub(crate) block_insert_state: Option<block::BlockInsertState>,
    /// Macro self-replay depth guard.
    pub macro_depth: usize,
    pub picker: Option<PickerGlue>,
    pub(crate) picker_ranking: picker::ranking::State,
    pub(crate) analysis: analysis::AnalysisState,
    pub(crate) resolution: resolution::ResolutionState,
    pub cwd: PathBuf,
    /// Bound workspace contexts (0042 slice 2): one per filesystem in use.
    pub workspaces: workspaces::WorkspaceRegistry,
    /// MRU document order (most recent first); drives `Space b`.
    pub mru: Vec<strop_core::id::DocumentId>,
    /// Picker preview file cache.
    pub previews: Previews,
    /// Git working surface state (M2).
    pub git: Option<strop_git::GitContext>,
    /// Preview file reads run on worker threads (0001 §3); results and
    /// the in-flight set are drained in drain_picker.
    pub preview_tx: std::sync::mpsc::Sender<PreviewResult>,
    pub preview_rx: Option<std::sync::mpsc::Receiver<PreviewResult>>,
    pub(crate) preview_loads: HashMap<PathBuf, strop_core::worker::Load<PreviewKey>>,
    pub hunks: git_memory::HunkSet,
    /// HEAD↔index — the staged set (0014 wave 4); rendered in the
    /// gutter's committed-adjacent color.
    pub staged_hunks: git_memory::HunkSet,
    /// Git memory (M3): per-buffer surface kinds, blame card, job channel,
    /// OSC52 clipboard payload drained by the TUI.
    pub blame_card: Option<strop_git::memory::BlameCard>,
    /// Each full/range/tail document owns its own revision-checked gutter.
    pub blame_gutters: HashMap<strop_core::id::DocumentId, BlameGutter>,
    /// Bumped on every buffer-list mutation; git jobs carry the
    /// generation they were spawned under so results for dead
    /// surfaces are dropped (0011 §2).
    pub generation: u64,
    pub git_tx: std::sync::mpsc::Sender<GitJob>,
    pub git_rx: Option<std::sync::mpsc::Receiver<GitJob>>,
    pub osc52: Option<String>,
    pub(crate) terminal_output: Vec<String>,
    /// ctrl-l: the terminal desynced from the model — the draw loop
    /// answers with a full repaint (vim's redraw).
    pub needs_repaint: bool,
    /// System-clipboard reads (paste from `+`) run on a worker thread;
    /// `clip_paste_pending` remembers before/after AND the initiating
    /// document until the read lands (0023 §4).
    pub clip_tx: std::sync::mpsc::Sender<ClipboardResult>,
    pub clip_rx: Option<std::sync::mpsc::Receiver<ClipboardResult>>,
    pub clip_paste_pending: Option<(bool, strop_core::worker::Ticket<ClipboardKey>)>,
    /// LSP server pool (0014 wave 2): one client per (workspace root,
    /// server) — a rust file and a python file in one session get their
    /// own servers. Diagnostics by path, hover card, open bookkeeping.
    pub lsp_servers: Vec<crate::editor::lsp::LspServer>,
    pub diags: HashMap<strop_core::id::DocumentId, DocumentDiagnostics>,
    pub hover_card: Option<String>,
    /// Shell jobs (`:!cmd` output buffers, `|cmd` pipes): results land
    /// in drain_shell — never a subprocess on the input path (0001 §3).
    pub shell_tx: std::sync::mpsc::Sender<ShellResult>,
    pub shell_rx: Option<std::sync::mpsc::Receiver<ShellResult>>,
    pub(crate) shell_requests: HashMap<strop_core::worker::WorkerId, ShellIntent>,
    pub(crate) shell_focus: Option<strop_core::worker::WorkerId>,
    /// Splits: flat row/column of panes (v1; tree layout later).
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    pub layout: LayoutDir,
    /// User config (0005-lite: TOML, embedded defaults, never bricks).
    pub config: crate::config::Config,
    /// Shared state root for explicit trust and optional session persistence.
    pub state_dir: Option<PathBuf>,
    pub(crate) session_policy: crate::session::SessionPolicy,
    /// The last grammar-level change (dot-repeat's semantic form).
    pub(crate) last_change: Option<strop_grammar::Command>,
    /// Direct non-grammar commands (x, p, J…) replay their key string.
    pub(crate) last_cmd_keys: String,
    pub(crate) last_insert: Option<String>,
    pub(crate) recording_insert: Option<String>,
    /// vim insert counts: `3i…`/`2o` repeat the session's text (o/O
    /// repeat the opened line too — `insert_open` carries it).
    pub(crate) insert_count: usize,
    pub(crate) insert_open: Option<String>,
}

/// The compiled query owns matching semantics for repeat, preview and highlighting.
#[derive(Debug, Clone)]
pub struct LastSearch {
    pub query: strop_grammar::CompiledQuery,
    pub backward: bool,
}

/// A pending `f/F/t/T` awaiting its target char — the leap-style
/// candidate overlay's input. Named fields, not a naked `(u8, bool)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindPending {
    pub ch: char,
    pub backward: bool,
}

/// The ctrl-v rectangle (0017): line span by buffer index, cell span
/// by LineLayout columns — named fields, not a naked mixed-unit tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRect {
    pub first_line: usize,
    pub last_line: usize,
    pub left_cell: strop_core::id::DisplayColumn,
    pub right_cell: strop_core::id::DisplayColumn,
}

impl Editor {
    pub fn new(buf: Buffer) -> Self {
        // cwd is the process directory (project-wide): pickers walk it,
        // LSP/git resolve against it; a file's own dir is not the project.
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new_in(buf, cwd)
    }

    /// Pure construction. Native services start explicitly after forensic seeding.
    pub fn new_in(buf: Buffer, cwd: PathBuf) -> Self {
        let (preview_tx, preview_rx) = std::sync::mpsc::channel();
        let (shell_tx, shell_rx) = std::sync::mpsc::channel();
        let (clip_tx, clip_rx) = std::sync::mpsc::channel();
        let (git_tx, git_rx) = git_channel();
        let mut docs = strop_core::id::Arena::default();
        // the source identity is set at construction, not by convention
        let doc = if buf.path.is_some() {
            Document::new(buf)
        } else {
            Document::scratch(buf)
        };
        let current = docs.insert(doc);
        Self {
            docs,
            trace_documents: HashMap::new(),
            io: io::IoState::default(),
            remote: remote::RemoteState::default(),
            remote_completion: remote_completion::RemoteCompletionState::default(),
            picker_ranking: picker::ranking::State::default(),
            analysis: analysis::AnalysisState::default(),
            resolution: resolution::ResolutionState::default(),
            worker_ids: strop_core::worker::WorkerIds::default(),
            worker_handles: HashMap::new(),
            focus_epoch: 0,
            finishing: false,
            tape: std::rc::Rc::new(strop_trace::replay::Tape::new()),
            git_view: strop_core::worker::WorkerId::new(0),
            git_discovery: strop_core::worker::Load::Idle,
            hunk_load: strop_core::worker::Load::Idle,
            hunks_untracked: false,
            log_requests: HashMap::new(),
            card_request: None,
            dive_requests: HashMap::new(),
            git_mutations: std::collections::VecDeque::new(),
            git_mutation: None,
            shell_requests: HashMap::new(),
            shell_focus: None,
            mru: vec![current],
            mode: Mode::Normal,
            pending: pending::PendingInput::default(),
            walker: input::Walker::new(),
            last_search: None,
            undo_browser: None,
            registers: HashMap::new(),
            marks: HashMap::new(),
            last_find: None,
            flash: None,
            message: String::new(),
            should_quit: false,
            ctrl_c_armed: false,
            last_visual: None,
            last_insert_pos: None,
            change_idx: None,
            view_rows: 24,
            recording: None,
            app_tx: None,
            lsp_state: lsp::state::LspState::default(),
            macros: std::collections::HashMap::new(),
            last_macro: None,
            block_insert_state: None,
            macro_depth: 0,
            last_change: None,
            last_cmd_keys: String::new(),
            last_insert: None,
            recording_insert: None,
            insert_count: 1,
            insert_open: None,
            picker: None,
            workspaces: {
                let mut registry = workspaces::WorkspaceRegistry::default();
                registry.bind(strop_workspace::Filesystem::Local, Some(cwd.clone()));
                registry
            },
            cwd,
            blame_gutters: HashMap::new(),
            generation: 0,
            previews: HashMap::new(),
            shell_tx,
            shell_rx: Some(shell_rx),
            git: None,
            hunks: git_memory::HunkSet::default(),
            staged_hunks: git_memory::HunkSet::default(),
            blame_card: None,
            git_tx,
            git_rx: Some(git_rx),
            needs_repaint: false,
            osc52: None,
            terminal_output: Vec::new(),
            preview_tx,
            preview_rx: Some(preview_rx),
            preview_loads: HashMap::new(),
            jumplist_past: Vec::new(),
            jumplist_future: Vec::new(),
            lsp_servers: Vec::new(),
            clip_tx,
            clip_rx: Some(clip_rx),
            clip_paste_pending: None,
            diags: HashMap::new(),
            hover_card: None,
            panes: vec![Pane {
                doc: current,
                sels: strop_core::selection::SelectionSet::default(),
                view_top: 0,
                hscroll: strop_core::id::DisplayColumn::new(0),
                desired_column: None,
            }],
            active_pane: 0,
            layout: LayoutDir::Row,
            config: crate::config::Config::default(),
            state_dir: None,
            session_policy: crate::session::SessionPolicy::Automatic,
        }
    }

    pub fn feed_text(&mut self, text: &str) {
        for key in keys::parse(text) {
            self.feed(key);
        }
    }

    pub fn feed(&mut self, key: Key) {
        let _trace_scope = trace::InputScope::enter(self, key);
        self.trace_state();
        let generated = self.resolution.in_action;
        if self.resolution.blocked()
            || (!generated && !self.resolution.queue.is_empty())
            || (generated && !self.resolution.staged.is_empty())
        {
            if generated {
                self.resolution
                    .staged
                    .push_back(resolution::DeferredInput::GeneratedKey {
                        key,
                        depth: self.macro_depth,
                    });
            } else {
                self.resolution
                    .queue
                    .push_back(resolution::DeferredInput::Key(key));
            }
            return;
        }
        self.run_input_action(|editor| {
            editor.feed_inner(key);
            editor.prepare_resolution_preview();
        });
        self.trace_state();
    }

    fn feed_inner(&mut self, key: Key) {
        self.revoke_shell_focus();
        self.message.clear();
        if key == Key::Esc
            && !self.pending.is_active()
            && self.cancel_open(strop_core::worker::CancelReason::Dismissed)
        {
            self.message = "open cancelled".into();
        }
        // macro recording (0016): q at ground stops and never reaches
        // the machine; everything else records BEFORE it runs, so
        // replay is exactly the live stream
        if let Some(reg) = self.recording {
            let at_ground = self.walker.is_ground() && !self.pending.is_active();
            if at_ground && key == Key::Char('q') {
                self.recording = None;
                self.message = format!("recorded @{}", reg);
                return;
            }
            if let Some(buf) = self.macros.get_mut(&reg) {
                buf.push(key);
            }
        }
        if self.pending.is_active() {
            return self.feed_pending(key);
        }
        if self.hover_card.is_some() {
            self.hover_card = None;
            return;
        }
        if self.blame_card.is_some() || self.card_request.is_some() {
            if let Some(card) = self.dismiss_card_authority() {
                if key == Key::Enter {
                    self.open_log_at(&card.sha);
                }
                return;
            }
        }
        if self.picker_open() {
            return self.feed_picker(key);
        }
        if self.feed_undo_browser(key) {
            return;
        }
        match self.mode {
            Mode::Insert => self.feed_insert(key),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => self.feed_visual(key),
            Mode::Normal => self.feed_normal(key),
        }
    }

    /// True when a modal input field sits in normal mode (picker field
    /// or pending line) — the TUI draws the block cursor for it.
    pub fn input_normal(&self) -> bool {
        self.pending.normal()
            || self
                .picker
                .as_ref()
                .is_some_and(|g| g.picker.input_normal())
    }

    /// The modal line's sigil when a free-text line is open (`: / ? |`)
    /// — the ONE authority; the render card, the terminal's bar-cursor
    /// shape, and pending dispatch all ask here (a `|sed s/a/b/` body
    /// is a pipe, not a search).
    pub fn pending_sigil(&self) -> Option<char> {
        self.pending.sigil()
    }

    // ---- shared helpers -------------------------------------------------

    /// `m{a}`: set mark a at the cursor.
    pub(crate) fn set_mark(&mut self, mark: char) {
        self.marks.insert(mark, (self.current(), self.head()));
        self.message = format!("mark {mark} set");
    }

    /// `'{a}`: jump to mark a (switches buffer if the mark lives there).
    pub(crate) fn jump_mark(&mut self, mark: char) {
        self.push_jump(); // mark jumps are jumplist entries
        match self.marks.get(&mark).copied() {
            Some((buf, offset)) => {
                if self.docs.get(buf).is_some() {
                    if buf != self.current() {
                        self.switch_to(buf);
                        self.discover_git();
                    }
                    self.set_head(
                        self.buf()
                            .clamp_boundary(offset.min(self.buf().len_bytes())),
                    );
                    self.clamp_cursor();
                }
            }
            None => self.message = format!("mark {mark} not set"),
        }
    }
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transaction_conformance;

impl Drop for Editor {
    fn drop(&mut self) {
        // One shutdown boundary, including headless errors and terminal failures.
        for handle in std::mem::take(&mut self.worker_handles).into_values() {
            handle.cancel(strop_core::worker::CancelReason::Shutdown);
        }
        for server in std::mem::take(&mut self.lsp_servers) {
            if let Some(client) = server.client {
                client.shutdown();
                client.wait(Duration::from_millis(500));
            }
        }
    }
}
