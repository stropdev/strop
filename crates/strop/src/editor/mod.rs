//! The editor state: modes, pending keys, named registers, buffers.
//! One input path for TUI and headless — both call `feed`.
//!
//! Mode handlers live beside this file: `normal`, `visual`, `insert`.

mod blame;
pub mod block;
mod cursor;
mod diagnostics;
mod dive;
mod document;
pub mod events;
mod git;
mod git_memory;
mod help;
mod input;
mod insert;
mod jumps;
mod lsp;
pub mod macros;
#[cfg(test)]
mod multicursor_tests;
pub(crate) mod normal;
mod panes;
mod permalink;
mod picker;
mod registers;
mod shell;
mod undo;
pub mod view;
mod visual;

pub use document::Document;
pub use git_memory::{git_channel, BlameGutter, GitJob, Surface};
pub use panes::{LayoutDir, Pane};
pub use picker::{PickerGlue, PreviewSource, Previews};

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

pub const FLASH_FOR: Duration = Duration::from_millis(280);

/// One register cell: text + linewise flag (vim's unnamed register is `"`).
pub type Registers = HashMap<char, (String, bool)>;

pub struct Editor {
    pub docs: strop_core::id::Arena<strop_core::id::DocumentKind, Document>,
    /// Modal input on the `:`/`/`/`|` line (rootle's boxes): Esc once
    /// enters normal mode on the line, twice clears it.
    pub pending_normal: bool,
    pub pending_cursor: usize,
    /// vim's jumplist (ctrl-o/ctrl-i): past/future stacks of
    /// (document, byte offset) (jumps.rs).
    pub jumplist_past: Vec<(strop_core::id::DocumentId, usize)>,
    pub jumplist_future: Vec<(strop_core::id::DocumentId, usize)>,
    pub mode: Mode,
    pub pending: String,
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
    pub registers: Registers,
    /// Marks: char → (document, byte offset). `m{a}` sets, `'{a}` jumps.
    pub marks: HashMap<char, (strop_core::id::DocumentId, usize)>,
    pub flash: Option<(Range, Instant)>,
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
    pub app_tx: Option<std::sync::mpsc::Sender<events::AppEvent>>,
    /// The outstanding hover request's identity (doc, history depth) —
    /// a reply against another state is stale (0018).
    pub hover_request: Option<(strop_core::id::DocumentId, usize)>,
    /// Merged languages.toml per workspace root (0018 — the OnceLock
    /// used to pin the FIRST project's config process-wide).
    pub langs_by_root: std::collections::HashMap<PathBuf, &'static strop_lsp::languages::Languages>,
    /// Recorded macros: register → key events.
    pub macros: std::collections::HashMap<char, Vec<Key>>,
    /// The last replayed macro register (@@).
    pub last_macro: Option<char>,
    /// Block insert/change context (0017): (first line, last line,
    /// edge cell) — the typed text replicates per row at Esc.
    pub block_delete_pending: Option<(usize, usize, u16)>,
    /// Macro self-replay depth guard.
    pub macro_depth: usize,
    pub picker: Option<PickerGlue>,
    pub cwd: PathBuf,
    /// MRU document order (most recent first); drives `Space b`.
    pub mru: Vec<strop_core::id::DocumentId>,
    /// Picker preview file cache.
    pub previews: Previews,
    /// Git working surface state (M2).
    pub git: Option<strop_git::Repo>,
    /// Preview file reads run on worker threads (0001 §3); results and
    /// the in-flight set are drained in drain_picker.
    pub preview_tx: std::sync::mpsc::Sender<(PathBuf, Option<String>)>,
    pub preview_rx: Option<std::sync::mpsc::Receiver<(PathBuf, Option<String>)>>,
    pub preview_inflight: std::collections::HashSet<PathBuf>,
    pub hunks: Vec<strop_git::Hunk>,
    /// HEAD↔index — the staged set (0014 wave 4); rendered in the
    /// gutter's committed-adjacent color.
    pub staged_hunks: Vec<strop_git::Hunk>,
    pub hunks_epoch: u64,
    /// Git memory (M3): per-buffer surface kinds, blame card, job channel,
    /// OSC52 clipboard payload drained by the TUI.
    pub blame_card: Option<strop_git::memory::BlameCard>,
    /// Blame gutters by canonical path (0011 §3): per-buffer view
    /// state that outlives index churn and never persists to sessions.
    pub blame_gutters: HashMap<PathBuf, BlameGutter>,
    /// Bumped on every buffer-list mutation; git jobs carry the
    /// generation they were spawned under so results for dead
    /// surfaces are dropped (0011 §2).
    pub generation: u64,
    pub git_tx: std::sync::mpsc::Sender<GitJob>,
    pub git_rx: Option<std::sync::mpsc::Receiver<GitJob>>,
    pub osc52: Option<String>,
    /// System-clipboard reads (paste from `+`) run on a worker thread;
    /// `clip_paste_pending` remembers before/after until the read lands.
    pub clip_tx: std::sync::mpsc::Sender<Option<String>>,
    pub clip_rx: Option<std::sync::mpsc::Receiver<Option<String>>>,
    pub clip_paste_pending: Option<bool>,
    /// LSP server pool (0014 wave 2): one client per (workspace root,
    /// server) — a rust file and a python file in one session get their
    /// own servers. Diagnostics by path, hover card, open bookkeeping.
    pub lsp_servers: Vec<crate::editor::lsp::LspServer>,
    pub diags: std::collections::HashMap<PathBuf, Vec<strop_lsp::Diag>>,
    pub hover_card: Option<String>,
    pub lsp_opened: std::collections::HashSet<PathBuf>,
    pub lsp_sent_epochs: std::collections::HashMap<PathBuf, u64>,
    pub lsp_hints_shown: std::collections::HashSet<&'static str>,
    /// Shell jobs (`:!cmd` output buffers, `|cmd` pipes): results land
    /// in drain_shell — never a subprocess on the input path (0001 §3).
    pub shell_tx: std::sync::mpsc::Sender<ShellResult>,
    pub shell_rx: Option<std::sync::mpsc::Receiver<ShellResult>>,
    /// Splits: flat row/column of panes (v1; tree layout later).
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    pub layout: LayoutDir,
    /// User config (0005-lite: TOML, embedded defaults, never bricks).
    pub config: crate::config::Config,
    /// XDG state dir for sessions, resolved once at startup by main;
    /// None in tests/headless → session writes no-op (hermetic).
    pub state_dir: Option<PathBuf>,
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

/// A search to repeat and highlight: `/pat`, `?pat`, or `*`-style
/// whole-word (`whole_word` filters matches to word boundaries).
#[derive(Debug, Clone)]
pub struct LastSearch {
    pub pattern: String,
    pub backward: bool,
    pub whole_word: bool,
}

/// What a shell job produced (0009-adjacent plumbing): `:!` displays,
/// `|` pipes through and replaces.
pub enum ShellResult {
    /// `:!cmd`: show stdout+stderr in a readonly output buffer.
    Display { cmd: String, output: String },
    /// `|cmd`: replace a range with stdout (verified before applying).
    Pipe {
        buffer: strop_core::id::DocumentId,
        start: usize,
        end: usize,
        original: String,
        output: String,
        /// false when the command failed (nonzero exit or spawn error):
        /// the source text is NEVER replaced (0015).
        ok: bool,
        err: String,
    },
}

impl Editor {
    pub fn new(buf: Buffer) -> Self {
        // cwd is the process directory (project-wide): pickers walk it,
        // LSP/git resolve against it; a file's own dir is not the project.
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (preview_tx, preview_rx) = std::sync::mpsc::channel();
        let (shell_tx, shell_rx) = std::sync::mpsc::channel();
        let (clip_tx, clip_rx) = std::sync::mpsc::channel();
        let (git_tx, git_rx) = git_channel();
        let mut docs = strop_core::id::Arena::default();
        let current = docs.insert(Document::new(buf));
        let mut e = Self {
            docs,
            mru: vec![current],
            mode: Mode::Normal,
            pending: String::new(),
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
            langs_by_root: std::collections::HashMap::new(),
            hover_request: None,
            macros: std::collections::HashMap::new(),
            last_macro: None,
            block_delete_pending: None,
            macro_depth: 0,
            last_change: None,
            last_cmd_keys: String::new(),
            last_insert: None,
            recording_insert: None,
            insert_count: 1,
            insert_open: None,
            picker: None,
            cwd,
            blame_gutters: HashMap::new(),
            generation: 0,
            previews: HashMap::new(),
            shell_tx,
            shell_rx: Some(shell_rx),
            git: None,
            hunks: Vec::new(),
            staged_hunks: Vec::new(),
            hunks_epoch: u64::MAX,
            blame_card: None,
            git_tx,
            git_rx: Some(git_rx),
            osc52: None,
            preview_tx,
            preview_rx: Some(preview_rx),
            preview_inflight: std::collections::HashSet::new(),
            pending_normal: false,
            pending_cursor: 0,
            jumplist_past: Vec::new(),
            jumplist_future: Vec::new(),
            lsp_servers: Vec::new(),
            clip_tx,
            clip_rx: Some(clip_rx),
            clip_paste_pending: None,
            diags: HashMap::new(),
            hover_card: None,
            lsp_opened: std::collections::HashSet::new(),
            lsp_sent_epochs: HashMap::new(),
            lsp_hints_shown: std::collections::HashSet::new(),
            panes: vec![Pane {
                doc: current,
                sels: strop_core::selection::SelectionSet::default(),
                view_top: 0,
            }],
            active_pane: 0,
            layout: LayoutDir::Row,
            config: crate::config::Config::default(),
            state_dir: None,
        };
        e.discover_git();
        e
    }

    pub fn feed_text(&mut self, s: &str) {
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            let key = match c {
                '\x1b' => Key::Esc,
                '\r' | '\n' => Key::Enter,
                '\x7f' => Key::Backspace,
                '<' => {
                    // token form: <esc> <cr> <bs>
                    let rest: String = chars.by_ref().take_while(|&c| c != '>').collect();
                    match rest.to_ascii_lowercase().as_str() {
                        "esc" => Key::Esc,
                        "cr" | "enter" => Key::Enter,
                        "bs" => Key::Backspace,
                        "space" => Key::Char(' '),
                        "up" => Key::Up,
                        "down" => Key::Down,
                        "tab" => Key::Tab,
                        "s-tab" => Key::Backtab,
                        "left" => Key::Left,
                        "right" => Key::Right,
                        "c-r" => Key::CtrlR,
                        "c-x" => Key::CtrlX,
                        "c-d" => Key::CtrlD,
                        "c-u" => Key::CtrlU,
                        "c-f" => Key::CtrlF,
                        "c-b" => Key::CtrlB,
                        "c-^" => Key::CtrlCaret,
                        "c-v" => Key::CtrlV,
                        "c-w" => Key::CtrlW,
                        "c-o" => Key::CtrlO,
                        _ => {
                            self.feed(Key::Char('<'));
                            for c in rest.chars().chain(std::iter::once('>')) {
                                self.feed(Key::Char(c));
                            }
                            continue;
                        }
                    }
                }
                c => Key::Char(c),
            };
            self.feed(key);
        }
    }

    pub fn feed(&mut self, key: Key) {
        // the modal input line dies with the pending text (0003 §1)
        if self.pending.is_empty() {
            self.pending_normal = false;
        }
        self.message.clear();
        // macro recording (0016): q at ground stops and never reaches
        // the machine; everything else records BEFORE it runs, so
        // replay is exactly the live stream
        if let Some(reg) = self.recording {
            let at_ground = self.walker.display().is_empty() && self.pending.is_empty();
            if at_ground && key == Key::Char('q') {
                self.recording = None;
                self.message = format!("recorded @{}", reg);
                return;
            }
            if let Some(buf) = self.macros.get_mut(&reg) {
                buf.push(key);
            }
        }
        if self.hover_card.is_some() {
            self.hover_card = None;
            return;
        }
        if self.blame_card.is_some() {
            match key {
                Key::Enter => {
                    // dive into the browser *at* the card's commit,
                    // not the newest row (0011 §3)
                    let sha = self.blame_card.as_ref().map(|c| c.sha.clone());
                    self.blame_card = None;
                    match sha {
                        Some(sha) => self.open_log_at(&sha),
                        None => self.open_log(false),
                    }
                }
                _ => self.blame_card = None,
            }
            return;
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
        self.pending_normal
            || self
                .picker
                .as_ref()
                .is_some_and(|g| g.picker.input_normal())
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
mod tests;
