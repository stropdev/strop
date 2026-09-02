//! The editor state: modes, pending keys, named registers, buffers.
//! One input path for TUI and headless — both call `feed`.
//!
//! Mode handlers live beside this file: `normal`, `visual`, `insert`.

mod git;
mod git_memory;
mod insert;
mod normal;
mod picker;
mod visual;

pub use git_memory::{git_channel, GitJob, Surface};
pub use picker::{PickerGlue, PreviewEntry, PreviewSource, Previews};

use std::collections::HashMap;
use std::path::PathBuf;
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
    /// User config (0005-lite: TOML, embedded defaults, never bricks).
    pub config: crate::config::Config,
    pub(crate) last_cmd_keys: String,
    pub(crate) last_insert: Option<String>,
    pub(crate) recording_insert: Option<String>,
}

impl Editor {
    pub fn new(buf: Buffer) -> Self {
        // cwd is always absolute: relative buffer paths resolve against
        // the process cwd, or the picker walks "" and finds nothing.
        let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let cwd = buf
            .path
            .as_deref()
            .map(|p| {
                let full = base.join(p);
                full.parent()
                    .map(|x| x.to_path_buf())
                    .unwrap_or_else(|| base.clone())
            })
            .unwrap_or(base);
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

    pub(crate) fn register(&self, name: Option<char>) -> &(String, bool) {
        static EMPTY: (String, bool) = (String::new(), false);
        self.registers.get(&name.unwrap_or('"')).unwrap_or(&EMPTY)
    }

    pub(crate) fn set_register(&mut self, name: Option<char>, text: String, linewise: bool) {
        self.registers.insert(name.unwrap_or('"'), (text, linewise));
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
