//! The editor state: modes, pending keys, named registers, buffers.
//! One input path for TUI and headless — both call `feed`.
//!
//! Mode handlers live beside this file: `normal`, `visual`, `insert`.

mod git;
mod insert;
mod normal;
mod picker;
mod visual;

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
    /// Syntax highlighter for the current buffer (None: unsupported ext).
    pub highlighter: Option<Highlighter>,
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
    pub(crate) last_cmd_keys: String,
    pub(crate) last_insert: Option<String>,
    pub(crate) recording_insert: Option<String>,
}

impl Editor {
    pub fn new(buf: Buffer) -> Self {
        let highlighter = buf.path.as_deref().and_then(Highlighter::for_path);
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
        let mut e = Self {
            buffers: vec![buf],
            highlighter,
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
        self.highlighter = buf.path.as_deref().and_then(Highlighter::for_path);
        self.buffers.push(buf);
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
            self.current = self.current.min(self.buffers.len() - 1);
            self.touch_mru(self.current);
            self.highlighter = self.buf().path.as_deref().and_then(Highlighter::for_path);
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
