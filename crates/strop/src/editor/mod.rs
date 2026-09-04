//! The editor state: modes, pending keys, named registers, buffers.
//! One input path for TUI and headless — both call `feed`.
//!
//! Mode handlers live beside this file: `normal`, `visual`, `insert`.

mod git;
mod git_memory;
mod help;
mod insert;
mod lsp;
#[cfg(test)]
mod multicursor_tests;
mod normal;
mod panes;
mod picker;
mod undo;
mod visual;

pub use git_memory::{git_channel, BlameGutter, GitJob, Surface};
pub use panes::{LayoutDir, Pane};
pub use picker::{PickerGlue, PreviewSource, Previews};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use strop_core::{Buffer, Range};
use strop_syntax::Highlighter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl Mode {
    pub fn chip(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "V-LINE",
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
    Tab,
    Backtab,
    CtrlR,
    CtrlW,
    /// Replace picker: exclude/include the selected match (0007 §2).
    CtrlX,
}

pub const FLASH_FOR: Duration = Duration::from_millis(280);

/// One register cell: text + linewise flag (vim's unnamed register is `"`).
pub type Registers = HashMap<char, (String, bool)>;

pub struct Editor {
    pub buffers: Vec<Buffer>,
    pub current: usize,
    /// Syntax highlighter per buffer (None: unsupported ext), aligned
    /// with `buffers` — previews and switches keep their highlighting.
    pub highlighters: Vec<Option<Highlighter>>,
    pub mode: Mode,
    pub cursor: usize,
    pub pending: String,
    /// Extra cursors beyond the primary `cursor` (`Q` toggles, normal
    /// Esc collapses; 0013). Invariant: sorted, deduped, none equal to
    /// `cursor`. Point cursors (byte offsets); visual mode is
    /// primary-only in v1 and collapses them.
    pub extra_cursors: Vec<usize>,
    /// `Space u` browser state (editor/undo.rs); None when closed.
    pub undo_browser: Option<undo::UndoBrowser>,
    /// Armed by `/`/`?` searches: (pattern, backward). `n`/`N` replay it.
    pub last_search: Option<(String, bool)>,
    pub anchor: usize,
    pub registers: Registers,
    /// Marks: char → (buffer index, byte offset). `m{a}` sets, `'{a}` jumps.
    pub marks: HashMap<char, (usize, usize)>,
    pub flash: Option<(Range, Instant)>,
    pub message: String,
    pub should_quit: bool,
    pub view_top: usize,
    pub picker: Option<PickerGlue>,
    pub cwd: PathBuf,
    /// MRU buffer order (most recent first); drives `Space b`.
    pub mru: Vec<usize>,
    /// Picker preview file cache.
    pub previews: Previews,
    /// Git working surface state (M2).
    pub git: Option<strop_git::Repo>,
    /// Preview file reads run on worker threads (0001 §3); results and
    /// the in-flight set are drained in drain_picker.
    pub preview_tx: std::sync::mpsc::Sender<(PathBuf, Option<String>)>,
    pub preview_rx: std::sync::mpsc::Receiver<(PathBuf, Option<String>)>,
    pub preview_inflight: std::collections::HashSet<PathBuf>,
    pub hunks: Vec<strop_git::Hunk>,
    pub hunks_epoch: u64,
    /// Git memory (M3): per-buffer surface kinds, blame card, job channel,
    /// OSC52 clipboard payload drained by the TUI.
    pub surfaces: Vec<Option<Surface>>,
    pub blame_card: Option<strop_git::memory::BlameCard>,
    /// Blame gutters by canonical path (0011 §3): per-buffer view
    /// state that outlives index churn and never persists to sessions.
    pub blame_gutters: HashMap<PathBuf, BlameGutter>,
    /// Bumped on every buffer-list mutation; git jobs carry the
    /// generation they were spawned under so results for dead
    /// surfaces are dropped (0011 §2).
    pub generation: u64,
    pub git_tx: std::sync::mpsc::Sender<GitJob>,
    pub git_rx: std::sync::mpsc::Receiver<GitJob>,
    pub osc52: Option<String>,
    /// System-clipboard reads (paste from `+`) run on a worker thread;
    /// `clip_paste_pending` remembers before/after until the read lands.
    pub clip_tx: std::sync::mpsc::Sender<Option<String>>,
    pub clip_rx: std::sync::mpsc::Receiver<Option<String>>,
    pub clip_paste_pending: Option<bool>,
    /// LSP: one client per workspace root, diagnostics by path,
    /// hover card text, open/sync bookkeeping.
    pub lsp: Option<strop_lsp::Client>,
    pub lsp_rx: Option<Receiver<strop_lsp::LspEvent>>,
    pub diags: std::collections::HashMap<PathBuf, Vec<strop_lsp::Diag>>,
    pub hover_card: Option<String>,
    pub lsp_opened: std::collections::HashSet<PathBuf>,
    pub lsp_sent_epochs: std::collections::HashMap<PathBuf, u64>,
    pub lsp_hints_shown: std::collections::HashSet<&'static str>,
    /// Splits: flat row/column of panes (v1; tree layout later).
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    pub layout: LayoutDir,
    /// User config (0005-lite: TOML, embedded defaults, never bricks).
    pub config: crate::config::Config,
    pub(crate) last_cmd_keys: String,
    pub(crate) last_insert: Option<String>,
    pub(crate) recording_insert: Option<String>,
}

impl Editor {
    pub fn new(buf: Buffer) -> Self {
        // cwd is the process directory (project-wide): pickers walk it,
        // LSP/git resolve against it; a file's own dir is not the project.
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (preview_tx, preview_rx) = std::sync::mpsc::channel();
        let (clip_tx, clip_rx) = std::sync::mpsc::channel();
        let (git_tx, git_rx) = git_channel();
        let mut e = Self {
            highlighters: vec![buf.path.as_deref().and_then(Highlighter::for_path)],
            buffers: vec![buf],
            current: 0,
            mode: Mode::Normal,
            cursor: 0,
            pending: String::new(),
            last_search: None,
            undo_browser: None,
            anchor: 0,
            registers: HashMap::new(),
            marks: HashMap::new(),
            extra_cursors: Vec::new(),
            flash: None,
            message: String::new(),
            should_quit: false,
            view_top: 0,
            last_cmd_keys: String::new(),
            last_insert: None,
            recording_insert: None,
            picker: None,
            cwd,
            blame_gutters: HashMap::new(),
            generation: 0,
            mru: vec![0],
            previews: HashMap::new(),
            git: None,
            hunks: Vec::new(),
            hunks_epoch: u64::MAX,
            surfaces: vec![None],
            blame_card: None,
            git_tx,
            git_rx,
            osc52: None,
            preview_tx,
            preview_rx,
            preview_inflight: std::collections::HashSet::new(),
            lsp: None,
            clip_tx,
            clip_rx,
            clip_paste_pending: None,
            lsp_rx: None,
            diags: HashMap::new(),
            hover_card: None,
            lsp_opened: std::collections::HashSet::new(),
            lsp_sent_epochs: HashMap::new(),
            lsp_hints_shown: std::collections::HashSet::new(),
            panes: vec![Pane {
                buffer: 0,
                cursor: 0,
                view_top: 0,
            }],
            active_pane: 0,
            layout: LayoutDir::Row,
            config: crate::config::Config::default(),
        };
        e.discover_git();
        e
    }

    /// Mark a buffer most-recently-used.
    pub fn touch_mru(&mut self, i: usize) {
        self.mru.retain(|&x| x != i);
        self.mru.insert(0, i);
    }

    pub fn buf(&self) -> &Buffer {
        &self.buffers[self.current]
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    /// Open a file into a new buffer and switch to it (`:e`).
    pub fn open_buffer(&mut self, path: &str) -> std::io::Result<()> {
        // vim semantics: :e on an open file switches to its buffer
        let canon = std::path::Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| self.cwd.join(path));
        if let Some(i) = self.buffers.iter().position(|b| {
            b.path
                .as_deref()
                .and_then(|p| std::path::Path::new(p).canonicalize().ok())
                == Some(canon.clone())
        }) {
            self.current = i;
            self.touch_mru(i);
            return Ok(());
        }
        let buf = Buffer::open(path)?;
        let hl = buf.path.as_deref().and_then(Highlighter::for_path);
        self.buffers.push(buf);
        self.surfaces.push(None);
        self.highlighters.push(hl);
        self.generation += 1; // buffer indices moved: old jobs are stale (0011 §2)
        self.current = self.buffers.len() - 1;
        self.touch_mru(self.current);
        self.cursor = 0;
        self.view_top = 0;
        self.discover_git();
        self.lsp_maybe_attach();
        Ok(())
    }

    /// Close the current buffer; quits when the last one closes.
    /// Returns false when unsaved changes block the close.
    pub fn close_buffer(&mut self, force: bool) -> bool {
        if self.buf().dirty && !force {
            self.message = "unsaved changes — :q! to force".into();
            return false;
        }
        let closed_surface = self.surfaces[self.current].take();
        self.buffers.remove(self.current);
        self.surfaces.remove(self.current);
        let closed = self.current;
        if self.buffers.is_empty() {
            crate::session::save(self);
            self.should_quit = true;
        } else {
            self.mru.retain(|&x| x != closed);
            for m in &mut self.mru {
                if *m > closed {
                    *m -= 1;
                }
            }
            self.highlighters.remove(closed);
            self.generation += 1; // buffer indices moved: old jobs are stale (0011 §2)
            self.current = self.current.min(self.buffers.len() - 1);
            self.touch_mru(self.current);
            self.cursor = 0;
            self.view_top = 0;
            // a closing surface hands the cursor and view back to the
            // buffer it opened from — unconditionally (0011 §1): when
            // the origin isn't what we'd land on next, we switch to it
            // (its index shifts down past `closed`)
            if let Some(surface) = closed_surface {
                if let Some(ret) = surface.return_point() {
                    let buffer = if ret.buffer > closed {
                        ret.buffer - 1
                    } else {
                        ret.buffer
                    };
                    if buffer != self.current {
                        self.current = buffer;
                        self.touch_mru(buffer);
                    }
                    self.cursor = ret.cursor.min(self.buf().len_bytes());
                    self.view_top = ret.view_top;
                }
            }
            self.discover_git();
        }
        true
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
                        "c-r" => Key::CtrlR,
                        "c-x" => Key::CtrlX,
                        "c-w" => Key::CtrlW,
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
        self.message.clear();
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
            Mode::Visual | Mode::VisualLine => self.feed_visual(key),
            Mode::Normal => self.feed_normal(key),
        }
    }

    // ---- shared helpers -------------------------------------------------

    pub(crate) fn flash(&mut self, range: Range) {
        self.flash = Some((range, Instant::now()));
    }

    pub fn flash_range(&self) -> Option<Range> {
        self.flash
            .and_then(|(r, at)| (at.elapsed() < FLASH_FOR).then_some(r))
    }

    pub fn clamp_cursor(&mut self) {
        let line = self.buf().line_of(self.cursor);
        let start = self.buf().line_start(line);
        let end = self.buf().line_end(line);
        let max = if self.mode == Mode::Insert {
            end
        } else {
            end.max(start + 1) - 1
        };
        self.cursor = self
            .buf()
            .clamp_boundary(self.cursor.clamp(start, max.max(start)));
    }

    /// Clamp one position the way clamp_cursor clamps the primary.
    fn clamp_pos(&self, pos: usize) -> usize {
        let line = self.buf().line_of(pos);
        let start = self.buf().line_start(line);
        let end = self.buf().line_end(line);
        let max = if self.mode == Mode::Insert {
            end
        } else {
            end.max(start + 1) - 1
        };
        self.buf().clamp_boundary(pos.clamp(start, max.max(start)))
    }

    /// Every cursor position, primary first (0013 §3).
    pub(crate) fn all_cursors(&self) -> Vec<usize> {
        std::iter::once(self.cursor)
            .chain(self.extra_cursors.iter().copied())
            .collect()
    }

    /// Restore the invariant after any cascade: sorted and deduped. An
    /// extra MAY sit on the primary (Q plants there, then you move) —
    /// edit cascades dedupe positions before applying.
    pub(crate) fn normalize_cursors(&mut self) {
        let mut extras = std::mem::take(&mut self.extra_cursors);
        let len = self.buf().len_bytes();
        for c in &mut extras {
            *c = self.buf().clamp_boundary((*c).min(len));
        }
        extras.sort_unstable();
        extras.dedup();
        self.extra_cursors = extras;
    }

    /// Remap cursors after a mirrored edit of `delta` bytes at each of
    /// `positions` (pre-edit, sorted, deduped): every cursor shifts by
    /// its own edit plus every edit below it (0013 §3).
    pub(crate) fn remap_after_mirrored_edit(&mut self, positions: &[usize], delta: isize) {
        let map = |old: usize| -> usize {
            let below = positions.partition_point(|&p| p < old);
            let own = usize::from(positions.contains(&old));
            (old as isize + delta * (below + own) as isize).max(0) as usize
        };
        self.cursor = map(self.cursor);
        self.extra_cursors = self.extra_cursors.iter().map(|&c| map(c)).collect();
        self.normalize_cursors();
    }

    /// `Q`: drop the cursor under point when one exists, else plant one.
    pub(crate) fn toggle_cursor(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        if let Some(i) = self.extra_cursors.iter().position(|&c| c == self.cursor) {
            self.extra_cursors.remove(i);
        } else {
            self.extra_cursors.push(self.cursor);
            self.normalize_cursors();
        }
        let n = self.extra_cursors.len() + 1;
        self.message = format!("{n} cursor{}", if n > 1 { "s" } else { "" });
    }

    /// `Space c` (helix's `C`): copy the primary cursor onto the same
    /// column of the next line — how vertical cursor stacks are built.
    pub(crate) fn add_cursor_next_line(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        // stack from the bottom-most cursor (helix C semantics: repeated
        // presses walk down the buffer)
        let base = self.extra_cursors.last().copied().unwrap_or(self.cursor);
        let line = self.buf().line_of(base);
        // the phantom line past a trailing newline is not a cursor home
        if line + 1 >= self.buf().len_lines()
            || self.buf().line_start(line + 1) >= self.buf().len_bytes()
        {
            self.message = "no line below".into();
            return;
        }
        let col = self.buf().col_of(base);
        let start = self.buf().line_start(line + 1);
        let end = self.buf().line_end(line + 1);
        let pos = (start + col).min(end.saturating_sub(1).max(start));
        self.extra_cursors.push(pos);
        self.normalize_cursors();
        let n = self.extra_cursors.len() + 1;
        self.message = format!("{n} cursors");
    }

    /// Normal-mode Esc: collapse to the primary cursor (0013 §3).
    pub(crate) fn collapse_cursors(&mut self) {
        if !self.extra_cursors.is_empty() {
            self.extra_cursors.clear();
            self.message = "1 cursor".into();
        }
    }

    /// Keep the cursor on screen; `rows` = text area height.
    pub fn scroll_to_cursor(&mut self, rows: usize) {
        let line = self.buf().line_of(self.cursor);
        if line < self.view_top {
            self.view_top = line;
        } else if line >= self.view_top + rows {
            self.view_top = line + 1 - rows;
        }
    }

    /// Diagnostics of buffer `idx`, resolved against cwd like
    /// diag_severity_at.
    fn diags_for(&self, idx: usize) -> Option<&Vec<strop_lsp::Diag>> {
        let path = self.buffers.get(idx)?.path.as_deref()?;
        let abs = if std::path::Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.cwd.join(path)
        };
        self.diags.get(&abs)
    }

    /// The worst diagnostic's (severity, message) on a 1-based line —
    /// the cursor line's end-of-line note (0009 UX).
    pub fn diag_message_at(&self, idx: usize, line_1based: usize) -> Option<(u8, &str)> {
        self.diags_for(idx)?
            .iter()
            .filter(|d| d.line + 1 == line_1based)
            .min_by_key(|d| d.severity)
            .map(|d| (d.severity, d.message.as_str()))
    }

    /// Worst diagnostic severity (1=error … 4=hint) for a 1-based line
    /// of buffer `idx`, if any (0001 pillar 4: merges with the git
    /// gutter). Per-buffer, so panes show their own diagnostics.
    pub fn diag_severity_at(&self, idx: usize, line_1based: usize) -> Option<u8> {
        let path = self.buffers.get(idx)?.path.as_deref()?;
        let abs = if std::path::Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.cwd.join(path)
        };
        let diags = self.diags.get(&abs)?;
        let mut best: Option<u8> = None;
        for d in diags {
            if d.line + 1 == line_1based {
                best = Some(best.map_or(d.severity, |b: u8| b.min(d.severity)));
            }
        }
        best
    }

    /// `m{a}`: set mark a at the cursor.
    pub(crate) fn set_mark(&mut self, mark: char) {
        self.marks.insert(mark, (self.current, self.cursor));
        self.message = format!("mark {mark} set");
    }

    /// `'{a}`: jump to mark a (switches buffer if the mark lives there).
    pub(crate) fn jump_mark(&mut self, mark: char) {
        match self.marks.get(&mark).copied() {
            Some((buf, offset)) => {
                if buf < self.buffers.len() {
                    if buf != self.current {
                        self.current = buf;
                        self.touch_mru(buf);
                        self.discover_git();
                    }
                    self.cursor = self
                        .buf()
                        .clamp_boundary(offset.min(self.buf().len_bytes()));
                    self.clamp_cursor();
                }
            }
            None => self.message = format!("mark {mark} not set"),
        }
    }

    pub(crate) fn register(&self, name: Option<char>) -> &(String, bool) {
        static EMPTY: (String, bool) = (String::new(), false);
        self.registers.get(&name.unwrap_or('"')).unwrap_or(&EMPTY)
    }

    pub(crate) fn set_register(&mut self, name: Option<char>, text: String, linewise: bool) {
        // the `+` register is the system clipboard: yank/delete into it
        // stages an OSC52 payload for the TUI to emit
        if name == Some('+') {
            self.osc52 = Some(text.clone());
        }
        self.registers.insert(name.unwrap_or('"'), (text, linewise));
    }

    /// One undo unit per command (change ops hold the transaction open
    /// through the insert session — vim groups `ci[foo<esc>` as one `u`).
    pub(crate) fn tx_begin(&mut self) {
        self.buf_mut().history.begin();
    }

    pub(crate) fn tx_commit(&mut self) {
        self.buf_mut().history.commit();
    }

    /// `u`: undo one revision. Readonly buffers never record.
    pub(crate) fn undo(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        match self.buf_mut().history.undo_ops() {
            Some(ops) => {
                // vim lands the cursor at the *start* of the undone
                // change; undo ops replay in reverse record order, so
                // first() is the tail of the change — take the minimum
                let start = ops.iter().map(|e| e.at).min().unwrap_or(0);
                self.buf_mut().apply_history(ops);
                self.cursor = start;
                self.clamp_cursor();
                self.flash(strop_core::Range::charwise(self.cursor, self.cursor));
            }
            None => self.message = "already at oldest change".into(),
        }
    }

    /// `ctrl-r`: redo along the last-visited branch.
    pub(crate) fn redo(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        match self.buf_mut().history.redo_ops() {
            Some(ops) => {
                // cursor after the redone text for inserts, at the start
                // of the redone deletion for deletes
                let at = ops
                    .last()
                    .map(|e| match e.kind {
                        strop_core::history::EditKind::Insert => e.at + e.text.len(),
                        strop_core::history::EditKind::Delete => e.at,
                    })
                    .unwrap_or(0);
                self.buf_mut().apply_history(ops);
                self.cursor = at;
                self.clamp_cursor();
                self.flash(strop_core::Range::charwise(self.cursor, self.cursor));
            }
            None => self.message = "nothing to redo".into(),
        }
    }

    pub(crate) fn paste(&mut self, name: Option<char>, before: bool) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        // `"+p`: the system clipboard is read by a provider job — never
        // a subprocess on the input path (0001 §3)
        if name == Some('+') {
            self.clipboard_paste(before);
            return;
        }
        let (text, linewise) = self.register(name).clone();
        if text.is_empty() {
            return;
        }
        self.paste_text(text, linewise, before);
    }

    /// `Space p` / `"+p`: spawn a clipboard read; the result lands in
    /// drain_clipboard on a later tick.
    pub(crate) fn clipboard_paste(&mut self, before: bool) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        if self.clip_paste_pending.is_some() {
            return; // one read in flight
        }
        self.clip_paste_pending = Some(before);
        let tx = self.clip_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(read_system_clipboard());
        });
    }

    /// Collect clipboard reads (event-loop tick + headless settle).
    pub fn drain_clipboard(&mut self) {
        if self.buffers.is_empty() {
            return;
        }
        while let Ok(result) = self.clip_rx.try_recv() {
            let Some(before) = self.clip_paste_pending.take() else {
                continue;
            };
            match result {
                Some(text) if !text.is_empty() => {
                    let linewise = text.len() > 1 && text.ends_with('\n');
                    self.paste_text(text, linewise, before);
                }
                _ => {
                    self.message =
                        "clipboard: empty or no provider (wl-paste/xclip/xsel/pbpaste)".into()
                }
            }
        }
    }

    /// Insertion point + landing spot for one cursor's paste.
    fn paste_points(
        &self,
        cursor: usize,
        text_len: usize,
        linewise: bool,
        before: bool,
    ) -> (usize, usize) {
        if linewise {
            let line = self.buf().line_of(cursor);
            let at = if before {
                self.buf().line_start(line)
            } else {
                self.buf().line_start(line + 1)
            };
            (
                at.min(self.buf().len_bytes()),
                at.min(self.buf().len_bytes()),
            )
        } else {
            let at = if before {
                cursor
            } else {
                (cursor + 1).min(self.buf().len_bytes())
            };
            let land = if before {
                at + text_len.saturating_sub(1)
            } else {
                at
            };
            (at, land)
        }
    }

    fn paste_text(&mut self, text: String, linewise: bool, before: bool) {
        let cursors = self.all_cursors();
        if cursors.len() == 1 {
            let (at, land) = self.paste_points(self.cursor, text.len(), linewise, before);
            self.buf_mut().insert(at, &text);
            self.cursor = land;
            self.clamp_cursor();
            return;
        }
        // multicursor paste (0013 §3): same text at every cursor,
        // bottom-up so insertion points stay valid mid-batch
        let primary = self.cursor;
        let mut jobs: Vec<(usize, usize, bool)> = cursors
            .into_iter()
            .map(|c| {
                let (at, land) = self.paste_points(c, text.len(), linewise, before);
                (at, land, c == primary)
            })
            .collect();
        jobs.sort_by_key(|j| j.0);
        jobs.dedup_by_key(|j| j.0); // stacked cursors paste once
                                    // each landing shifts by what lower insertions already added
        let mut shift = 0usize;
        for j in &mut jobs {
            j.1 += shift;
            shift += text.len();
        }
        self.tx_begin();
        for (at, _, _) in jobs.iter().rev() {
            self.buf_mut().insert(*at, &text);
        }
        self.tx_commit();
        self.extra_cursors = jobs.iter().filter(|j| !j.2).map(|j| j.1).collect();
        self.cursor = jobs.iter().find(|j| j.2).map(|j| j.1).unwrap_or(primary);
        self.normalize_cursors();
        self.clamp_cursor();
    }
}

/// Read the system clipboard via the first working provider (helix's
/// playbook: wl-paste, xclip, xsel, pbpaste). Runs on a worker thread.
fn read_system_clipboard() -> Option<String> {
    let providers: [(&str, &[&str]); 4] = [
        ("wl-paste", &[]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
        ("pbpaste", &[]),
    ];
    for (cmd, args) in providers {
        let Ok(out) = std::process::Command::new(cmd).args(args).output() else {
            continue; // not installed
        };
        if out.status.success() {
            return String::from_utf8(out.stdout).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with(text: &str) -> Editor {
        Editor::new(Buffer::from_text(text))
    }

    fn text(e: &Editor) -> String {
        e.buf().rope.to_string()
    }

    #[test]
    fn named_registers_yank_and_paste() {
        let mut e = editor_with("alpha\nbeta\ngamma\n");

        e.feed_text("\"ayy"); // yank line into register a
        assert_eq!(e.register(Some('a')).0, "alpha\n");
        e.feed_text("j");
        e.feed_text("\"ap"); // paste a below beta
        assert_eq!(text(&e), "alpha\nbeta\nalpha\ngamma\n");
        // unnamed register untouched
        assert!(e.register(None).0.is_empty());
    }

    #[test]
    fn space_y_yanks_motion_to_system_register() {
        let mut e = Editor::new(Buffer::from_text("hello world\n"));
        e.feed_text(" yw");
        assert_eq!(e.register(Some('+')).0, "hello ");
        assert!(e.osc52.is_some(), "OSC52 payload staged for the TUI");
    }

    #[test]
    fn visual_space_y_yanks_selection_to_system_register() {
        let mut e = Editor::new(Buffer::from_text("hello world\n"));
        e.feed_text("vl y");
        assert_eq!(e.register(Some('+')).0, "he");
        assert!(e.osc52.is_some());
    }

    #[test]
    fn clipboard_paste_inserts_read_result() {
        let mut e = Editor::new(Buffer::from_text("ab\n"));
        e.clip_paste_pending = Some(false);
        e.clip_tx.send(Some("XY".into())).unwrap();
        e.drain_clipboard();
        assert_eq!(e.buf().rope.to_string(), "aXYb\n");
    }

    #[test]
    fn clipboard_paste_reports_missing_provider() {
        let mut e = Editor::new(Buffer::from_text("ab\n"));
        e.clip_paste_pending = Some(false);
        e.clip_tx.send(None).unwrap();
        e.drain_clipboard();
        assert!(e.message.contains("clipboard"));
        assert_eq!(e.buf().rope.to_string(), "ab\n");
    }

    #[test]
    fn alias_verbs() {
        let mut e = editor_with("let edge = hone;\n");
        e.feed_text("0wD"); // delete from 'edge' to EOL
        assert_eq!(text(&e), "let \n");
        let mut e = editor_with("let x = 1;\n");
        e.feed_text("0wY"); // yy
        assert_eq!(e.register(None).0, "let x = 1;\n");
        let mut e = editor_with("abc\n");
        e.feed_text("sZ"); // cl + insert Z
        e.feed(crate::editor::Key::Esc);
        assert_eq!(text(&e), "Zbc\n");
    }

    #[test]
    fn replace_char_and_join() {
        let mut e = editor_with("abc\ndef\n");
        e.feed_text("rX");
        assert_eq!(text(&e), "Xbc\ndef\n");
        e.feed_text("J");
        assert_eq!(text(&e), "Xbc def\n");
    }

    #[test]
    fn indent_and_dedent() {
        let mut e = editor_with("a\nb\nc\n");
        e.feed_text("2>>");
        assert_eq!(text(&e), "    a\n    b\nc\n");
        e.feed_text("0<<");
        assert_eq!(text(&e), "a\n    b\nc\n");
    }

    #[test]
    fn dot_repeat_replays_insert() {
        let mut e = editor_with("one\ntwo\n");
        e.feed_text("A!");
        e.feed(crate::editor::Key::Esc);
        e.feed_text("j.");
        assert_eq!(text(&e), "one!\ntwo!\n");
    }

    #[test]
    fn visual_line_deletes_whole_lines() {
        let mut e = editor_with("a\nb\nc\nd\n");
        e.feed_text("Vjd");
        assert_eq!(text(&e), "c\nd\n");
        assert!(e.register(None).1); // linewise
        e.feed_text("P");
        assert_eq!(text(&e), "a\nb\nc\nd\n"); // paste linewise above
    }

    #[test]
    fn ex_open_and_close_buffers() {
        std::fs::write("/tmp/strop-test-b.rs", "second\n").unwrap();
        let mut e = editor_with("first\n");
        e.feed_text(":e /tmp/strop-test-b.rs<cr>");
        assert_eq!(e.buffers.len(), 2);
        assert_eq!(text(&e), "second\n");
        e.feed_text(":q<cr>");
        assert_eq!(e.buffers.len(), 1);
        assert_eq!(text(&e), "first\n");
        // dirty buffer refuses :q, allows :q!
        e.feed_text("ix");
        e.feed(crate::editor::Key::Esc);
        e.feed_text(":q<cr>");
        assert_eq!(e.buffers.len(), 1);
        assert!(e.message.contains("unsaved"));
        e.feed_text(":q!<cr>");
        assert!(e.should_quit);
    }
}

#[cfg(test)]
mod indent_tests {
    use super::*;

    #[test]
    fn enter_copies_and_deepens_indent() {
        let mut e = Editor::new(Buffer::from_text("fn f() {\n    let x = 1;\n}\n"));
        e.feed_text("j$"); // on the let line, at EOL
        e.feed(crate::editor::Key::Char('a'));
        e.feed(crate::editor::Key::Enter);
        e.feed_text("let y = 2;");
        assert_eq!(
            e.buf().rope.to_string(),
            "fn f() {\n    let x = 1;\n    let y = 2;\n}\n"
        );
        // after an opener, one level deeper
        e.feed(crate::editor::Key::Esc);
        e.feed_text("gg$");
        e.feed(crate::editor::Key::Char('a'));
        e.feed(crate::editor::Key::Enter);
        e.feed_text("// body");
        let got = e.buf().rope.to_string();
        assert!(got.starts_with("fn f() {\n    // body"), "got: {got:?}");
    }

    #[test]
    fn o_auto_indents() {
        let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
        e.feed_text("o");
        e.feed_text("let x = 1;");
        assert_eq!(e.buf().rope.to_string(), "fn f() {\n    let x = 1;\n}\n");
    }

    #[test]
    fn tab_size_from_config() {
        let mut e = Editor::new(Buffer::from_text("a\nb\n"));
        e.config = crate::config::Config {
            tab_size: 2,
            ..Default::default()
        };
        e.feed_text(">>");
        assert_eq!(e.buf().rope.to_string(), "  a\nb\n");
        e.feed_text("<<");
        assert_eq!(e.buf().rope.to_string(), "a\nb\n");
    }

    #[test]
    fn new_file_opens_empty_and_saves() {
        let path = "/tmp/strop-newfile-test.rs";
        std::fs::remove_file(path).ok();
        let mut e = Editor::new(Buffer::open(path).expect("missing file is a new buffer"));
        assert_eq!(e.buf().len_bytes(), 0);
        e.feed_text("ifresh");
        e.feed(crate::editor::Key::Esc);
        e.feed_text(":w<cr>");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "fresh");
        std::fs::remove_file(path).ok();
    }
}

#[cfg(test)]
mod alignment_tests {
    use super::*;

    /// buffers / highlighters / surfaces stay index-aligned through every
    /// open/close path (bitten twice; the contract now lives here).
    #[test]
    fn parallel_vecs_stay_aligned() {
        std::fs::write("/tmp/strop-align-a.rs", "a\n").unwrap();
        std::fs::write("/tmp/strop-align-b.rs", "b\n").unwrap();
        let mut e = Editor::new(Buffer::open("/tmp/strop-align-a.rs").unwrap());
        let check = |e: &Editor| {
            assert_eq!(e.buffers.len(), e.highlighters.len());
            assert_eq!(e.buffers.len(), e.surfaces.len());
        };
        check(&e);
        e.open_buffer("/tmp/strop-align-b.rs").unwrap();
        check(&e);
        // a surface (readonly virtual buffer)
        e.surfaces.pop();
        e.surfaces.push(None);
        check(&e);
        e.close_buffer(true);
        check(&e);
        e.close_buffer(true);
        assert!(e.should_quit);
        std::fs::remove_file("/tmp/strop-align-a.rs").ok();
        std::fs::remove_file("/tmp/strop-align-b.rs").ok();
    }
}

#[cfg(test)]
mod smartindent_tests {
    use super::*;

    #[test]
    fn closer_dedents_on_indent_only_line() {
        // open a line inside fn f() { } — auto-indented, then '}' dedents
        let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
        e.feed_text("o"); // indented to one level
        assert_eq!(e.buf().line_text(1), "    ");
        e.feed_text("}"); // closer on the indent-only line → dedent first
                          // the new line sits at col 0; the file's own closing brace is untouched
        assert_eq!(e.buf().rope.to_string(), "fn f() {\n}\n}\n");
    }

    #[test]
    fn closer_noop_with_real_text_before() {
        let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
        e.feed_text("o"); // indented one level
        e.feed_text("let x = 1;"); // real text on the line
        e.feed_text("}"); // closer after text: no dedent
        assert_eq!(e.buf().line_text(1), "    let x = 1;}");
    }
}

#[cfg(test)]
mod undo_tests {
    use super::*;

    #[test]
    fn insert_session_undoes_as_one_unit() {
        let mut e = Editor::new(Buffer::from_text("hello\n"));
        e.feed_text("A world"); // append " world" at EOL
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "hello world\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "hello\n");
        e.feed(crate::editor::Key::CtrlR);
        assert_eq!(e.buf().rope.to_string(), "hello world\n");
    }

    #[test]
    fn change_op_holds_one_undo_unit() {
        let mut e = Editor::new(Buffer::from_text("say [old] now\n"));
        e.feed_text("w"); // onto [old]
        e.feed_text("ci["); // change inside brackets
        e.feed_text("new");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "say [new] now\n");
        e.feed_text("u"); // ONE undo restores the whole change
        assert_eq!(e.buf().rope.to_string(), "say [old] now\n");
    }

    #[test]
    fn n_repeats_search_and_wraps() {
        let mut e = Editor::new(Buffer::from_text("foo bar\nfoo baz\n"));
        e.feed_text("/foo\r"); // lands on the *next* match (vim)
        assert_eq!(e.cursor, 8);
        e.feed_text("n"); // wraps to the first
        assert_eq!(e.cursor, 0);
        e.feed_text("N"); // backward, wraps from top
        assert_eq!(e.cursor, 8);
    }

    #[test]
    fn edit_after_undo_forks_and_ctrlr_redoes_last_branch() {
        let mut e = Editor::new(Buffer::from_text("ab\n"));
        e.feed_text("rx"); // replace a with x
        e.feed_text("u");
        e.feed_text("ry"); // fork: replace a with y
        e.feed_text("u"); // back to ab
        e.feed(crate::editor::Key::CtrlR); // redo the last-visited branch
        assert_eq!(e.buf().rope.to_string(), "yb\n");
        // redo once more: nothing (the fork tip is current)
        e.feed(crate::editor::Key::CtrlR);
        assert_eq!(e.buf().rope.to_string(), "yb\n");
    }

    #[test]
    fn readonly_buffers_refuse_undo() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.buf_mut().readonly = true;
        e.feed_text("u");
        assert!(e.message.contains("readonly"));
        assert_eq!(e.buf().rope.to_string(), "x\n");
    }
}

#[cfg(test)]
mod surround_tests {
    use super::*;

    #[test]
    fn ds_deletes_pair() {
        let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
        e.feed_text("w"); // onto "hi"
        e.feed_text("ds\"");
        assert_eq!(e.buf().rope.to_string(), "say hi now\n");
        // and undo restores the pair as one unit
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "say \"hi\" now\n");
    }

    #[test]
    fn cs_changes_pair() {
        let mut e = Editor::new(Buffer::from_text("call(a, b)\n"));
        e.feed_text("f("); // onto the open paren (on-pair counts as inside)
        e.feed_text("cs(["); // change (…) to […]
        assert_eq!(e.buf().rope.to_string(), "call[a, b]\n");
        // and undo restores as one unit
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "call(a, b)\n");
    }

    #[test]
    fn ysiw_wraps_word() {
        let mut e = Editor::new(Buffer::from_text("make it sharp\n"));
        e.feed_text("w"); // onto "it"
        e.feed_text("ysiw\"");
        assert_eq!(e.buf().rope.to_string(), "make \"it\" sharp\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "make it sharp\n");
    }

    #[test]
    fn visual_s_wraps_selection() {
        let mut e = Editor::new(Buffer::from_text("wrap me up\n"));
        e.feed_text("ve"); // select "wrap"
        e.feed_text("S(");
        assert_eq!(e.buf().rope.to_string(), "(wrap) me up\n");
    }
}

#[cfg(test)]
mod visual_object_tests {
    use super::*;

    #[test]
    fn vi_paren_selects_inner() {
        let mut e = Editor::new(Buffer::from_text("call(a, b)\n"));
        e.feed_text("f("); // onto the open paren
        e.feed_text("vi(");
        let r = e.visual_range().expect("visual range");
        assert_eq!(e.buf().slice_string(r), "a, b");
        // and operators consume it
        e.feed_text("d");
        assert_eq!(e.buf().rope.to_string(), "call()\n");
    }

    #[test]
    fn va_quote_includes_quotes() {
        let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
        e.feed_text("w"); // onto "hi"
        e.feed_text("va\"");
        let r = e.visual_range().expect("visual range");
        assert_eq!(e.buf().slice_string(r), "\"hi\"");
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;

    #[test]
    fn undo_after_visual_delete() {
        let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
        e.feed_text("ved"); // vim: deletes "say", the space stays
        assert_eq!(e.buf().rope.to_string(), " \"hi\" now\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "say \"hi\" now\n");
    }

    #[test]
    fn quote_object_scans_forward_on_the_line() {
        // vim i" special case: cursor before the string uses the next pair
        let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
        e.feed_text("vi\"");
        let r = e.visual_range().expect("selection");
        assert_eq!(e.buf().slice_string(r), "hi");
    }

    #[test]
    fn quit_leaves_editor_drain_safe() {
        // regression: the last :q! emptied the buffer list and the
        // post-feed drain tick panicked indexing buffers[current]
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(":q!\r");
        assert!(e.should_quit);
        assert!(e.buffers.is_empty());
        e.drain_picker();
        e.drain_git_jobs();
        e.drain_lsp();
        e.lsp_sync_changed();
    }

    #[test]
    fn undo_lands_cursor_at_change_start() {
        // regression: undo took the first replayed op (the tail of the
        // change) — vim lands at the start of the undone region
        let mut e = Editor::new(Buffer::from_text("hello\n"));
        e.feed_text("A world");
        e.feed(crate::editor::Key::Esc);
        e.feed_text("0"); // move away from the change
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "hello\n");
        // the change started at byte 5 (" world"); normal-mode clamp
        // pulls 5 onto the last char of the line
        assert_eq!(e.cursor, 4);
    }
}

#[cfg(test)]
mod keybinds_tests {
    use super::*;

    #[test]
    fn marks_set_and_jump() {
        std::fs::write("/tmp/strop-mark-a.rs", "one\ntwo\nthree\n").unwrap();
        std::fs::write("/tmp/strop-mark-b.rs", "alpha\nbeta\n").unwrap();
        let mut e = Editor::new(Buffer::open("/tmp/strop-mark-a.rs").unwrap());
        e.feed_text("jj"); // line 3
        e.feed_text("mb"); // mark b here
        e.feed_text(":e /tmp/strop-mark-b.rs<cr>");
        e.feed_text("'b"); // jump back to mark
        assert_eq!(e.buf().path.as_deref(), Some("/tmp/strop-mark-a.rs"));
        assert_eq!(e.buf().line_of(e.cursor), 2);
        std::fs::remove_file("/tmp/strop-mark-a.rs").ok();
        std::fs::remove_file("/tmp/strop-mark-b.rs").ok();
    }

    #[test]
    fn question_mark_is_search_backward() {
        // vim fidelity: ? is search-backward, never the keybinds popup
        let mut e = Editor::new(Buffer::from_text("one two one two\n"));
        e.feed_text("$"); // end
        e.feed_text("?one\r");
        assert_eq!(
            e.buf().col_of(e.cursor),
            8,
            "backward search lands on the second 'one'"
        );
    }
}
