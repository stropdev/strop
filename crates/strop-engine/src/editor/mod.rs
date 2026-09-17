//! The editor state: modes, pending keys, named registers, buffers.
//! One input path for TUI and headless — both call `feed`.
//!
//! Mode handlers live beside this file: `normal`, `visual`, `insert`.

mod api;
mod blame;
pub mod block;
#[cfg(test)]
pub mod conformance;
#[cfg(test)]
pub mod contract_probes;
pub mod trace;

pub(crate) mod analysis;
mod changes;
pub use changes::review::ReviewRow;
pub use dispatch::InputOwner;
pub mod collections;
mod containers;
mod cursor;
mod diagnostics;
mod directory;
mod dispatch;
mod filesystem;
pub mod resolution;
pub use diagnostics::DocumentDiagnostics;
mod dive;
mod document;
pub mod events;
mod explain;
pub(crate) mod field;
mod git;
pub mod git_memory;
mod help;
mod indent;
mod input;
mod insert;
pub mod io;
mod jumps;
pub mod keys;
mod lsp;
pub mod macros;
pub(crate) mod matching;
#[cfg(test)]
mod multicursor_tests;
pub(crate) mod normal;
mod occurrence;
#[cfg(test)]
mod occurrence_tests;
mod panes;
pub mod pending;
mod permalink;
mod picker;
pub mod prepare;
mod privacy;
pub mod recovery;
mod registers;
pub mod remote;
mod remote_completion;
mod shell;
pub(crate) mod terminal;
pub use terminal::TerminalDocument;
pub mod transact;
mod undo;
pub mod view;
mod visual;
mod workspaces;

pub use collections::{CollectionRow, CollectionRowInfo};
pub use document::Document;
pub use document::{DiffRow, Directory, DocumentSource, RemoteDocument, Surface};
pub use git_memory::{git_channel, BlameGutter, GitJob};
pub use git_memory::{CommitFiles, PreparedDiff, PreparedFiles, Sidebar, SidebarRow};
pub use panes::{LayoutDir, Pane};
pub use picker::{
    checked_hit_range, PickerGlue, PreviewKey, PreviewResult, PreviewSource, Previews,
    ReplacementHit, SearchScope,
};
pub use registers::{ClipboardKey, ClipboardResult, Register};
pub use shell::{ShellIntent, ShellKey, ShellResult};

pub use containers::{ContainerKey, ContainerResult};
pub use lsp::attach::AttachRecord;
pub use lsp::LspServer;
pub use permalink::PendingPermalink;
pub use picker::ranking::Event as RankingEvent;
pub use remote::{RemoteEvent, RemoteView};
pub use remote_completion::{RemoteCompletionKey, RemoteCompletionResult};
pub use resolution::{ResolutionEvent, ResolutionState};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
pub use workspaces::WorkspaceRegistry;

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
    /// ctrl-space: query suggestions in a query field (0051 R02).
    CtrlSpace,
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

/// The cursor fade-in window (0064 §2): one short, bounded interval
/// sharing the flash's 16 ms animation budget.
pub const CURSOR_FADE_MS: u64 = 160;

/// The injected frame renderer's shape (0046): editor, columns, rows,
/// record-action flag — the binary's implementation renders via TestBackend.
pub type FrameDraw = fn(&mut Editor, u16, u16, bool) -> std::io::Result<()>;

pub struct Editor {
    /// Document arena (0056 AR02): frontends read through
    /// [`Editor::document`]/[`Editor::has_documents`] and admit new
    /// documents through [`Editor::admit_document`].
    pub(crate) docs: strop_core::id::Arena<strop_core::id::DocumentKind, Document>,
    pub(crate) trace_documents:
        HashMap<strop_core::id::DocumentId, strop_core::diagnostics::BufferTraceId>,
    pub(crate) remote: remote::RemoteState,
    pub(crate) directories: directory::DirectoryState,
    pub(crate) filesystem: filesystem::FsState,
    pub(crate) io: io::IoState,
    pub(crate) remote_completion: remote_completion::RemoteCompletionState,
    pub(crate) worker_ids: strop_core::worker::WorkerIds,
    pub(crate) worker_handles:
        HashMap<strop_core::worker::WorkerId, strop_core::worker::CancelHandle>,
    pub(crate) focus_epoch: u64,
    pub(crate) finishing: bool,
    pub(crate) tape: std::rc::Rc<strop_trace::replay::Tape>,
    pub(crate) git_view: strop_core::worker::WorkerId,
    pub(crate) git_discovery: strop_core::worker::Load<git_memory::ContextKey>,
    pub(crate) hunk_load: strop_core::worker::Load<git_memory::HunkKey>,
    pub(crate) hunks_untracked: bool,
    pub(crate) log_requests:
        HashMap<strop_core::id::DocumentId, strop_core::worker::Ticket<git_memory::LogKey>>,
    pub(crate) card_request: Option<strop_core::worker::Ticket<git_memory::CardKey>>,
    pub(crate) dive_requests:
        HashMap<strop_core::id::DocumentId, strop_core::worker::Ticket<git_memory::DiveKey>>,
    pub(crate) git_mutations: std::collections::VecDeque<git_memory::GitMutation>,
    pub(crate) git_mutation: Option<strop_core::worker::Ticket<git_memory::MutationKey>>,
    /// vim's jumplist (ctrl-o/ctrl-i): past/future stacks of named
    /// navigation/view records — caret, selection, viewport and
    /// horizontal origin (0051 §7, jumps.rs).
    pub(crate) jumplist_past: Vec<jumps::JumpRecord>,
    pub(crate) jumplist_future: Vec<jumps::JumpRecord>,
    pub(crate) mode: Mode,
    pub(crate) pending: pending::PendingInput,
    /// The input walker (0008 stage 2): typed parser state for
    /// counts/registers/operators/prefixes — pending stays for the
    /// free-text lines only.
    pub(crate) walker: input::Walker,
    /// `Space u` browser state (editor/undo.rs); None when closed.
    pub(crate) undo_browser: Option<undo::UndoBrowser>,
    /// Last `f/F/t/T` find: (char, backward, till). `;` and `,` replay it.
    pub(crate) last_find: Option<(char, bool, bool)>,
    /// Armed by `/`/`?`/`*`/`#` searches. `n`/`N` replay it; the render
    /// highlights matches persistently (rootle: current match underlined).
    pub(crate) last_search: Option<LastSearch>,
    /// Live occurrence selection (0049 §7): needle + add-order ranges.
    pub(crate) occurrence: Option<occurrence::OccurrenceState>,
    pub(crate) registers: HashMap<char, Register>,
    /// Marks: char → (document, byte offset). `m{a}` sets, `'{a}` jumps.
    pub(crate) marks: HashMap<char, (strop_core::id::DocumentId, usize)>,
    pub(crate) flash: Option<(Range, strop_trace::replay::Tick)>,
    /// Cursor fade-in (0064 §2): the Normal-mode block cursor fades
    /// back in from this tick after a focus return, pane/buffer switch
    /// or a jump beyond half a screen. Presentation-only — never part
    /// of the recorded observation.
    pub(crate) cursor_fade: Option<strop_trace::replay::Tick>,
    /// Last cursor-fade track mark (pane, document, head line) for
    /// jump detection across recorded actions.
    pub(crate) fade_track: Option<(usize, strop_core::id::DocumentId, usize)>,
    pub(crate) message: String,
    pub(crate) should_quit: bool,
    /// ctrl-c is armed after the first warn (0015 quit policy).
    pub(crate) ctrl_c_armed: bool,
    /// Last visual range for `gv` (recorded per visual-mode key).
    pub(crate) last_visual: Option<(usize, usize)>,
    /// Where the last insert session was for `gi`.
    pub(crate) last_insert_pos: Option<usize>,
    /// g;/g, walk: (index, history depth it was taken at) — a new
    /// commit invalidates the walk.
    pub(crate) change_idx: Option<(usize, usize)>,
    /// Text area height in rows — the render loop feeds it via
    /// scroll_to_cursor; viewport motions read it.
    pub(crate) view_rows: usize,
    /// Macro recording (0016): the register being recorded into.
    pub(crate) recording: Option<char>,
    /// The app event channel (0018): set by connect_events; late LSP
    /// attaches forward through it.
    pub(crate) app_tx: Option<events::EventSender>,
    pub(crate) lsp_state: lsp::state::LspState,
    /// Recorded macros: register → key events.
    pub(crate) macros: std::collections::HashMap<char, Vec<Key>>,
    /// The last replayed macro register (@@).
    pub(crate) last_macro: Option<char>,
    /// The rows and cell edge owned by an ongoing block insert/change.
    pub(crate) block_insert_state: Option<block::BlockInsertState>,
    /// Macro self-replay depth guard.
    pub(crate) macro_depth: usize,
    pub(crate) picker: Option<PickerGlue>,
    pub(crate) retained_search: Option<PickerGlue>,
    pub(crate) picker_source: Option<strop_picker::SourceWorker>,
    pub(crate) picker_ranking: picker::ranking::State,
    pub(crate) analysis: analysis::AnalysisState,
    pub(crate) terminals: terminal::State,
    pub(crate) resolution: resolution::ResolutionState,
    pub(crate) cwd: PathBuf,
    /// Bound workspace contexts (0042 slice 2): one per filesystem in use.
    pub(crate) workspaces: workspaces::WorkspaceRegistry,
    /// Applied change plans and their receipts (0043); grouped undo reads it.
    pub(crate) changes: changes::ChangeState,
    pub(crate) review: changes::review::ReviewState,
    /// Open editable code collections by their buffer document (0044).
    pub(crate) collections: HashMap<strop_core::id::DocumentId, collections::Collection>,
    /// A build waiting on background source loads (0044 v2).
    pub(crate) collection_build: Option<collections::CollectionBuild>,
    /// Container attach/browse state (0037 DC1a).
    pub(crate) containers: containers::ContainerState,
    /// MRU document order (most recent first); drives `Space b`.
    pub(crate) mru: Vec<strop_core::id::DocumentId>,
    /// Picker preview file cache.
    pub(crate) previews: Previews,
    /// Git working surface state (M2).
    pub(crate) git: Option<strop_git::GitContext>,
    /// Preview file reads run on worker threads (0001 §3); results and
    /// the in-flight set are drained in drain_picker.
    pub(crate) preview_tx: std::sync::mpsc::Sender<PreviewResult>,
    pub(crate) preview_rx: Option<std::sync::mpsc::Receiver<PreviewResult>>,
    pub(crate) preview_loads:
        HashMap<strop_workspace::ResourceLocation, strop_core::worker::Load<PreviewKey>>,
    pub(crate) hunks: git_memory::HunkSet,
    /// HEAD↔index — the staged set (0014 wave 4); rendered in the
    /// gutter's committed-adjacent color.
    pub(crate) staged_hunks: git_memory::HunkSet,
    /// Git memory (M3): per-buffer surface kinds, blame card, job channel,
    /// OSC52 clipboard payload drained by the TUI.
    pub(crate) blame_card: Option<strop_git::memory::BlameCard>,
    /// Each full/range/tail document owns its own revision-checked gutter.
    pub(crate) blame_gutters: HashMap<strop_core::id::DocumentId, BlameGutter>,
    /// Bumped on every buffer-list mutation; git jobs carry the
    /// generation they were spawned under so results for dead
    /// surfaces are dropped (0011 §2).
    pub(crate) generation: u64,
    pub(crate) git_tx: std::sync::mpsc::Sender<GitJob>,
    pub(crate) git_rx: Option<std::sync::mpsc::Receiver<GitJob>>,
    pub(crate) osc52: Option<String>,
    pub(crate) terminal_output: Vec<String>,
    /// ctrl-l: the terminal desynced from the model — the draw loop
    /// answers with a full repaint (vim's redraw).
    pub(crate) needs_repaint: bool,
    /// Last view-preparation stamp (AR01): preparation runs only when the
    /// view-relevant state or geometry changed; unchanged repaints stay pure.
    /// `None` until the first [`Editor::prepare_view`].
    pub(crate) frame_stamp: Option<u64>,
    /// Accepted-preparation generation (AR03): bumps on every meaningful
    /// state/geometry change; the prepared view is keyed by it.
    pub(crate) view_generation: u64,
    /// The published prepared view (AR03), rebuilt by preparation and
    /// borrowed by the frontend per frame.
    pub(crate) prepared: prepare::PreparedView,
    /// Worker-ticket admission counters (AR01 evidence).
    pub(crate) admissions: prepare::AdmissionProbe,
    /// System-clipboard reads (paste from `+`) run on a worker thread;
    /// `clip_paste_pending` remembers before/after AND the initiating
    /// document until the read lands (0023 §4).
    pub(crate) clip_tx: std::sync::mpsc::Sender<ClipboardResult>,
    pub(crate) clip_rx: Option<std::sync::mpsc::Receiver<ClipboardResult>>,
    pub(crate) clip_paste_pending: Option<(bool, strop_core::worker::Ticket<ClipboardKey>)>,
    /// LSP server pool (0014 wave 2): one client per (workspace root,
    /// server) — a rust file and a python file in one session get their
    /// own servers. Diagnostics by path, hover card, open bookkeeping.
    pub(crate) lsp_servers: Vec<crate::editor::lsp::LspServer>,
    pub(crate) diags: HashMap<strop_core::id::DocumentId, DocumentDiagnostics>,
    pub(crate) hover_card: Option<String>,
    /// Shell jobs (`:!cmd` output buffers, `|cmd` pipes): results land
    /// in drain_shell — never a subprocess on the input path (0001 §3).
    pub(crate) shell_tx: std::sync::mpsc::Sender<ShellResult>,
    pub(crate) shell_rx: Option<std::sync::mpsc::Receiver<ShellResult>>,
    pub(crate) shell_focus: Option<strop_core::worker::WorkerId>,
    pub(crate) shell_requests: HashMap<strop_core::worker::WorkerId, ShellIntent>,
    /// Splits: flat row/column of panes (v1; tree layout later).
    /// Frontends read through [`Editor::panes`]/[`Editor::active_pane`]/
    /// [`Editor::layout`] and act through admitted actions (0056 AR02).
    pub(crate) panes: Vec<Pane>,
    pub(crate) active_pane: usize,
    pub(crate) layout: LayoutDir,
    /// User config (0005-lite: TOML, embedded defaults, never bricks).
    /// Frontends read [`Editor::config`] and replace it through
    /// [`Editor::set_config`].
    pub(crate) config: crate::config::Config,
    /// Shared state root for explicit trust and optional session persistence.
    pub(crate) state_dir: Option<PathBuf>,
    pub(crate) session_policy: crate::session::SessionPolicy,
    /// Draft checkpoint/recovery (0056 AR04): durable dirty/scratch draft
    /// bytes in private state storage — a different product from session.
    pub(crate) recovery: recovery::RecoveryState,

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
    /// Injected frame renderer (0046): cell-grid production belongs to the
    /// binary; replay of a recorded `Frame` action renders through this
    /// hook. The engine never renders on its own.
    pub(crate) frame_draw: Option<FrameDraw>,
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
        let Ok(current) = docs.try_insert(doc) else {
            unreachable!("a fresh arena cannot exhaust its identity space")
        };
        Self {
            docs,
            trace_documents: HashMap::new(),
            io: io::IoState::default(),
            remote: remote::RemoteState::default(),
            directories: directory::DirectoryState::default(),
            filesystem: filesystem::FsState::default(),
            remote_completion: remote_completion::RemoteCompletionState::default(),
            picker_ranking: picker::ranking::State::default(),
            analysis: analysis::AnalysisState::default(),
            terminals: terminal::State::default(),
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
            containers: containers::ContainerState::default(),
            mru: vec![current],
            changes: changes::ChangeState::default(),
            review: changes::review::ReviewState::default(),
            collections: HashMap::new(),
            collection_build: None,
            mode: Mode::Normal,
            occurrence: None,
            pending: pending::PendingInput::default(),
            walker: input::Walker::new(),
            last_search: None,
            undo_browser: None,
            registers: HashMap::new(),
            marks: HashMap::new(),
            last_find: None,
            flash: None,
            cursor_fade: None,
            fade_track: None,
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
            retained_search: None,
            picker_source: None,
            workspaces: {
                let mut registry = workspaces::WorkspaceRegistry::default();
                let Ok(_) = registry.bind(strop_workspace::Filesystem::Local, Some(cwd.clone()))
                else {
                    unreachable!("a fresh workspace registry cannot exhaust its identity space")
                };
                registry
            },
            cwd,
            blame_gutters: HashMap::new(),
            generation: 0,
            frame_stamp: None,
            view_generation: 0,
            prepared: prepare::PreparedView::empty(),
            admissions: prepare::AdmissionProbe::default(),
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
                terminal_input: false,
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
            recovery: recovery::RecoveryState::default(),
            frame_draw: None,
        }
    }

    pub fn feed_text(&mut self, text: &str) {
        for key in keys::parse(text) {
            self.handle_frontend_input(strop_core::frontend_input::Input::Key(key));
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
        self.recovery_after_event();
    }

    fn feed_inner(&mut self, key: Key) {
        self.lsp_state.hover = None;
        if let Some(build) = self.collection_build.as_mut() {
            build.focus_on_ready = false;
        }
        if let Some(preparing) = self.review.preparing.as_mut() {
            preparing.focus_ready = false;
        }
        self.revoke_shell_focus();
        self.message.clear();
        if key == Key::Esc
            && self.mode == Mode::Normal
            && !self.pending.is_active()
            && self.review.preparing.is_some()
        {
            self.review_cancel_pub();
            return;
        }
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
        self.dispatch_owned(key);
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

    /// The current document's indent (config default or detected at
    /// open — resolved eagerly, so reads never rescan).
    pub(crate) fn cur_indent(&self) -> document::Indent {
        self.indentation_at(self.current(), self.head())
    }

    // ---- shared helpers -------------------------------------------------

    /// `m{a}`: set mark a at the cursor.
    pub(crate) fn set_mark(&mut self, mark: char) {
        self.marks.insert(mark, (self.current(), self.head()));
        self.message = format!("mark {mark} set");
    }

    /// (name, 1-based line, trimmed line text) per set mark, name-sorted —
    /// the which-key mark cards' live rows (0047 §3).
    pub fn mark_rows(&self) -> Vec<(char, usize, String)> {
        let mut rows: Vec<_> = self
            .marks
            .iter()
            .filter_map(|(name, (document, offset))| {
                let doc = self.docs.get(*document)?;
                let line = doc.buf.line_of(*offset);
                let text: String = doc.buf.line_text(line).trim().chars().take(48).collect();
                Some((*name, line + 1, text))
            })
            .collect();
        rows.sort_by_key(|row| row.0);
        rows
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
                    // mark jumps use the 0051 §7 landing placement:
                    // center unless comfortably visible
                    self.place_jump_target();
                }
            }
            None => self.message = format!("mark {mark} not set"),
        }
    }
}

/// Local-channel delivery and fixture helpers for engine-consumer tests;
/// live drivers use the shared AppEvent channel.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
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

/// The headless `state` directive's logical observation (0006): mode,
/// cursor, pending input, picker and register state — no cells. The
/// binary's headless driver prints it; the forensic observation embeds it.
pub fn state_json(editor: &Editor) -> String {
    if editor.docs.is_empty() {
        return serde_json::json!({"should_quit":editor.should_quit,"documents":0,"message":editor.message}).to_string();
    }
    serde_json::json!({
        "mode": if editor.terminal_input_active() { "TERMINAL" } else { editor.mode.chip() },
        "cursor": editor.head(),
        "line": editor.buf().line_of(editor.head()) + 1,
        "col": editor.buf().col_of(editor.head()) + 1,
        "pending": editor.pending.text(),
        "message": editor.message,
        "extra_cursors": editor.extra_selections().iter().map(|s| s.head).collect::<Vec<_>>(),
        "panes": editor.panes.len(),
        "active_pane": editor.active_pane,
        "picker": editor.picker_open(),
        "picker_input": editor.picker.as_ref().map(|g| g.picker.input.text().to_string()),
        "picker_items": editor.picker.as_ref().map(|g| g.picker.items.len()),
        "picker_streaming": editor.picker.as_ref().map(|g| g.picker.streaming),
        "register": editor.register(None).text,
        "dirty": editor.buf().dirty,
        "terminal":editor.terminal_status(editor.current()),
    })
    .to_string()
}
