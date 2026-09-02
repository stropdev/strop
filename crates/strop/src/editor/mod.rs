//! The editor state: modes, pending keys, named registers, buffers.
//! One input path for TUI and headless — both call `feed`.
//!
//! Mode handlers live beside this file: `normal`, `visual`, `insert`.

mod git;
mod git_memory;
mod insert;
mod lsp;
mod normal;
mod panes;
mod picker;
mod visual;

pub use git_memory::{git_channel, GitJob, Surface};
pub use panes::{LayoutDir, Pane};
pub use picker::{PickerGlue, PreviewEntry, PreviewSource, Previews};

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
    pub hunks: Vec<strop_git::Hunk>,
    pub hunks_epoch: u64,
    /// Hunk preview card (`Space g p`).
    pub hunk_preview: Option<strop_git::Hunk>,
    /// Git memory (M3): per-buffer surface kinds, blame card, job channel,
    /// OSC52 clipboard payload drained by the TUI.
    pub surfaces: Vec<Option<Surface>>,
    pub blame_card: Option<strop_git::memory::BlameCard>,
    pub git_tx: std::sync::mpsc::Sender<GitJob>,
    pub git_rx: std::sync::mpsc::Receiver<GitJob>,
    pub osc52: Option<String>,
    /// The keybinds popup (`Space ?`): table-driven (keymap.rs).
    pub keybinds_open: bool,
    pub keybinds_section: usize,
    pub keybinds_scroll: usize,
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
        let (git_tx, git_rx) = git_channel();
        let mut e = Self {
            highlighters: vec![buf.path.as_deref().and_then(Highlighter::for_path)],
            buffers: vec![buf],
            current: 0,
            mode: Mode::Normal,
            cursor: 0,
            pending: String::new(),
            anchor: 0,
            registers: HashMap::new(),
            marks: HashMap::new(),
            flash: None,
            message: String::new(),
            should_quit: false,
            view_top: 0,
            last_cmd_keys: String::new(),
            last_insert: None,
            recording_insert: None,
            picker: None,
            cwd,
            mru: vec![0],
            previews: HashMap::new(),
            git: None,
            hunks: Vec::new(),
            hunks_epoch: u64::MAX,
            hunk_preview: None,
            surfaces: vec![None],
            blame_card: None,
            git_tx,
            git_rx,
            osc52: None,
            keybinds_open: false,
            keybinds_section: 0,
            keybinds_scroll: 0,
            lsp: None,
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

    /// Current buffer's highlighter.
    pub fn highlighter(&mut self) -> Option<&mut Highlighter> {
        self.highlighters
            .get_mut(self.current)
            .and_then(|h| h.as_mut())
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
            self.current = self.current.min(self.buffers.len() - 1);
            self.touch_mru(self.current);
            self.cursor = 0;
            self.view_top = 0;
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
        if self.keybinds_open {
            return self.feed_keybinds(key);
        }
        if self.hover_card.is_some() {
            self.hover_card = None;
            return;
        }
        if self.blame_card.is_some() {
            match key {
                Key::Enter => {
                    self.blame_card = None;
                    self.open_log(false); // dive into the commit browser
                }
                _ => self.blame_card = None,
            }
            return;
        }
        if self.hunk_preview.is_some() {
            self.hunk_preview = None;
            return; // first key dismisses the card
        }
        if self.picker_open() {
            return self.feed_picker(key);
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

    /// Keep the cursor on screen; `rows` = text area height.
    pub fn scroll_to_cursor(&mut self, rows: usize) {
        let line = self.buf().line_of(self.cursor);
        if line < self.view_top {
            self.view_top = line;
        } else if line >= self.view_top + rows {
            self.view_top = line + 1 - rows;
        }
    }

    /// Keybinds popup keys: j/k scroll, tab/h/l section, esc/q close.
    pub(crate) fn feed_keybinds(&mut self, key: Key) {
        match key {
            Key::Esc => self.keybinds_open = false,
            Key::Char('q') => self.keybinds_open = false,
            Key::Char('j') | Key::Down => self.keybinds_scroll += 1,
            Key::Char('k') | Key::Up => {
                self.keybinds_scroll = self.keybinds_scroll.saturating_sub(1)
            }
            Key::Tab | Key::Char('l') => {
                self.keybinds_section = (self.keybinds_section + 1) % crate::keymap::SECTIONS.len();
                self.keybinds_scroll = 0;
            }
            Key::Backtab | Key::Char('h') => {
                let n = crate::keymap::SECTIONS.len();
                self.keybinds_section = (self.keybinds_section + n - 1) % n;
                self.keybinds_scroll = 0;
            }
            _ => {}
        }
    }

    /// Diagnostic severity letter for a 1-based line on the current
    /// buffer, if any (0001 pillar 4: merges with the git gutter).
    pub fn diag_at(&self, line_1based: usize) -> Option<&'static str> {
        let path = self.buf().path.as_deref()?;
        let abs = if std::path::Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.cwd.join(path)
        };
        let diags = self.diags.get(&abs)?;
        let mut best: Option<u8> = None;
        for d in diags {
            if d.line + 1 == line_1based {
                best = Some(best.map_or(d.severity, |b| b.min(d.severity)));
            }
        }
        best.map(|s| match s {
            1 => "E",
            2 => "W",
            3 => "I",
            _ => "H",
        })
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
                let start = ops.first().map(|e| e.at).unwrap_or(0);
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
                let end = ops.last().map(|e| e.at + e.text.len()).unwrap_or(0);
                self.buf_mut().apply_history(ops);
                self.cursor = end;
                self.clamp_cursor();
                self.flash(strop_core::Range::charwise(self.cursor, self.cursor));
            }
            None => self.message = "nothing to redo".into(),
        }
    }

    pub(crate) fn paste(&mut self, name: Option<char>, before: bool) {
        let (text, linewise) = self.register(name).clone();
        if text.is_empty() {
            return;
        }
        if linewise {
            let line = self.buf().line_of(self.cursor);
            let at = if before {
                self.buf().line_start(line)
            } else {
                self.buf().line_start(line + 1)
            };
            let at = at.min(self.buf().len_bytes());
            self.buf_mut().insert(at, &text);
            self.cursor = at;
        } else {
            let at = if before {
                self.cursor
            } else {
                (self.cursor + 1).min(self.buf().len_bytes())
            };
            self.buf_mut().insert(at, &text);
            self.cursor = if before {
                at + text.len().saturating_sub(1)
            } else {
                at
            };
        }
        self.clamp_cursor();
    }
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
}

#[cfg(test)]
mod keybinds_tests {
    use super::*;

    #[test]
    fn space_question_opens_popup() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" ?");
        assert!(e.keybinds_open);
        e.feed(crate::editor::Key::Esc);
        assert!(!e.keybinds_open);
    }

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
