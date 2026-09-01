//! Picker glue: the editor side of strop-picker. Workers post onto the
//! event loop (0001 §5.6); the editor drains them between keystrokes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use strop_picker::{spawn_files, GrepWorker, Item, Kind, Payload, Picker, PickerMsg};
use strop_syntax::Highlighter;

use super::{Editor, Key};

pub struct PickerGlue {
    pub picker: Picker,
    /// Sender stays alive for grep respawns (kill + respawn per keystroke).
    tx: Option<Sender<PickerMsg>>,
    rx: Option<Receiver<PickerMsg>>,
    grep_worker: Option<GrepWorker>,
}

impl Editor {
    pub fn open_picker(&mut self, kind: Kind) {
        let (tx, rx) = channel();
        let mut tx = Some(tx);
        let (items, streaming, rx) = match kind {
            Kind::Buffers => {
                // MRU-ordered (0003 §2): most-recent *other* buffer first,
                // vim's alternate-file instinct.
                let items = self
                    .mru
                    .iter()
                    .map(|&i| {
                        let name = self.buffers[i]
                            .path
                            .clone()
                            .unwrap_or_else(|| "[scratch]".into());
                        Item {
                            text: name,
                            payload: Payload::Buffer(i),
                        }
                    })
                    .collect();
                (items, false, None)
            }
            Kind::Files => {
                spawn_files(self.cwd.clone(), tx.take().expect("fresh channel"));
                (vec![], true, Some(rx))
            }
            Kind::Grep => (vec![], false, Some(rx)),
        };
        let picker = Picker::new(kind, items, streaming);
        self.picker = Some(PickerGlue {
            picker,
            tx,
            rx,
            grep_worker: None,
        });
    }

    pub fn close_picker(&mut self) {
        self.picker = None;
    }

    pub fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// Keystrokes while a picker is open. The input line is insert-mode
    /// semantics (0003 §1); nav is arrows / ctrl-n,p / tab.
    pub(crate) fn feed_picker(&mut self, key: Key) {
        let Some(glue) = &mut self.picker else { return };
        match key {
            Key::Esc => self.close_picker(),
            Key::Enter => {
                let payload = glue.picker.current().map(|i| i.payload.clone());
                self.picker = None;
                if let Some(p) = payload {
                    self.accept_picker(p);
                }
            }
            Key::Backspace => {
                glue.picker.pop_char();
                self.picker_input_changed();
            }
            Key::Up => glue.picker.move_by(-1),
            Key::Down => glue.picker.move_by(1),
            Key::Tab => glue.picker.move_by(1),
            Key::Backtab => glue.picker.move_by(-1),
            Key::Char(c) => {
                glue.picker.push_char(c);
                self.picker_input_changed();
            }
        }
    }

    fn picker_input_changed(&mut self) {
        let Some(glue) = &mut self.picker else { return };
        if glue.picker.kind == Kind::Grep {
            // rg filters; kill + respawn per keystroke (worker is cheap).
            // A fresh channel per respawn: the old worker's messages (incl.
            // its trailing Done) fail to send on the dropped receiver, so
            // stale generations can't race the new one.
            let pattern = glue.picker.input.clone();
            let cwd = self.cwd.clone();
            glue.grep_worker = None; // drop kills the old rg
            glue.picker.items.clear();
            let (tx, rx) = channel();
            glue.tx = Some(tx.clone());
            glue.rx = Some(rx);
            glue.grep_worker = GrepWorker::spawn(&pattern, &cwd, tx);
            glue.picker.streaming = glue.grep_worker.is_some();
        } else {
            glue.picker.refilter();
        }
    }

    /// Drain worker messages (called from the event loop each tick).
    pub fn drain_picker(&mut self) {
        let mut done = false;
        if let Some(glue) = &mut self.picker {
            if let Some(rx) = &glue.rx {
                let mut items = Vec::new();
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        PickerMsg::Items(batch) => items.extend(batch),
                        PickerMsg::Done => done = true,
                    }
                }
                if !items.is_empty() {
                    glue.picker.append(items);
                }
            }
            if done {
                glue.picker.streaming = false;
            }
        }
    }

    fn accept_picker(&mut self, payload: Payload) {
        match payload {
            Payload::File(rel) => {
                let path = self.cwd.join(&rel);
                match self.open_buffer(&path.display().to_string()) {
                    Ok(()) => {}
                    Err(e) => self.message = format!("open {}: {e}", rel.display()),
                }
            }
            Payload::Buffer(i) => {
                if i < self.buffers.len() {
                    self.current = i;
                    self.touch_mru(i);
                    self.highlighter = self
                        .buf()
                        .path
                        .as_deref()
                        .and_then(strop_syntax::Highlighter::for_path);
                    self.cursor = 0;
                    self.view_top = 0;
                }
            }
            Payload::Grep {
                path, line, col, ..
            } => {
                let full = self.cwd.join(&path);
                if let Err(e) = self.open_buffer(&full.display().to_string()) {
                    self.message = format!("open {}: {e}", path.display());
                    return;
                }
                let start = self.buf().line_start(line.saturating_sub(1));
                self.cursor = self.buf().clamp_boundary(start + col.saturating_sub(1));
                self.clamp_cursor();
            }
        }
    }

    /// Preview payload for the render layer: (title, focus line, rope).
    /// Files are read once and cached with a highlighter; buffers render
    /// from the live rope.
    pub fn picker_preview(&mut self) -> Option<(String, Option<usize>, PreviewSource<'_>)> {
        let item = self.picker.as_ref()?.picker.current()?.clone();
        match item.payload {
            Payload::Buffer(i) => {
                let name = self
                    .buffers
                    .get(i)?
                    .path
                    .clone()
                    .unwrap_or_else(|| "[scratch]".into());
                Some((name, None, PreviewSource::Live(&self.buffers.get(i)?.rope)))
            }
            Payload::File(rel) => {
                let full = self.cwd.join(&rel);
                self.preview_cache(&full)?;
                let entry = self.previews.get_mut(&full)?;
                Some((
                    rel.display().to_string(),
                    None,
                    PreviewSource::Cached(entry),
                ))
            }
            Payload::Grep { path, line, .. } => {
                let full = self.cwd.join(&path);
                self.preview_cache(&full)?;
                let entry = self.previews.get_mut(&full)?;
                Some((
                    path.display().to_string(),
                    Some(line),
                    PreviewSource::Cached(entry),
                ))
            }
        }
    }

    fn preview_cache(&mut self, path: &PathBuf) -> Option<String> {
        if self.previews.contains_key(path) {
            return Some(String::new()); // entry exists; render reads the rope
        }
        let text = std::fs::read_to_string(path).ok()?;
        let rope = ropey::Rope::from_str(&text);
        let hl = Highlighter::for_path(&path.display().to_string());
        self.previews
            .insert(path.clone(), crate::editor::PreviewEntry { rope, hl });
        Some(String::new())
    }
}

pub struct PreviewEntry {
    pub rope: ropey::Rope,
    pub hl: Option<Highlighter>,
}

pub enum PreviewSource<'a> {
    Live(&'a ropey::Rope),
    Cached(&'a mut PreviewEntry),
}

pub type Previews = HashMap<PathBuf, PreviewEntry>;
