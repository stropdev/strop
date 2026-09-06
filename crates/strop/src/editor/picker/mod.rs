//! Picker glue: the editor side of strop-picker. Workers post onto the
//! event loop (0001 §5.6); the editor drains them between keystrokes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use strop_picker::{spawn_files, GrepWorker, Item, Kind, Payload, Picker, PickerMsg};
use strop_syntax::Highlighter;

use super::{Editor, Key};

mod accept;
mod drain;
mod preview;
mod replace;
mod tests;

impl PickerGlue {
    /// Take the streaming channel (the TUI's forwarder owns it — 0018).
    pub(crate) fn take_rx(&mut self) -> Option<Receiver<PickerMsg>> {
        self.rx.take()
    }

    /// Hand the stream channel to the app event forwarder (0018),
    /// tagged with this picker's identity and generation (0020 §2).
    pub(crate) fn forward_stream(&mut self, tx: &Sender<super::events::AppEvent>) {
        if let Some(rx) = self.rx.take() {
            let (id, gen) = (self.id, self.gen);
            let tx = tx.clone();
            std::thread::spawn(move || {
                while let Ok(msg) = rx.recv() {
                    if tx
                        .send(super::events::AppEvent::Picker { id, gen, msg })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    }

    /// A picker over editor-computed items (diagnostics; 0009 §3 Space d).
    pub fn diagnostics(picker: Picker) -> Self {
        Self {
            id: 0,
            gen: 0,
            picker,
            tx: None,
            rx: None,
            grep_worker: None,
        }
    }
}

pub struct PickerGlue {
    pub picker: Picker,
    /// This instance's identity (streams tag their messages with it).
    pub id: u64,
    /// The current query generation — bumped per respawn (0020 §2).
    pub gen: u64,
    /// Sender stays alive for grep respawns (kill + respawn per keystroke).
    tx: Option<Sender<PickerMsg>>,
    rx: Option<Receiver<PickerMsg>>,
    grep_worker: Option<GrepWorker>,
}

impl Editor {
    /// Assign a picker; a live stream forwards onto the app channel
    /// when the TUI is connected (0018).
    pub(crate) fn set_picker(&mut self, mut glue: PickerGlue) {
        glue.id = self.next_picker_id;
        self.next_picker_id += 1;
        if let Some(tx) = &self.app_tx {
            glue.forward_stream(tx);
        }
        self.picker = Some(glue);
    }

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
                        let name = self
                            .doc(i)
                            .buf
                            .path
                            .as_ref()
                            .map(|p| p.to_string_lossy().into_owned())
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
            Kind::Grep | Kind::Replace => (vec![], false, Some(rx)),
            Kind::Diagnostics | Kind::Locations => {
                unreachable!("location lists use PickerGlue::diagnostics")
            }
        };
        let picker = Picker::new(kind, items, streaming);
        self.set_picker(PickerGlue {
            id: 0,
            gen: 0,
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
        let replace = glue.picker.kind == Kind::Replace;
        match key {
            Key::Esc => {
                // rootle's input boxes: Esc enters vim normal mode on the
                // field; Esc again closes the picker
                if glue.picker.input_normal() {
                    self.close_picker();
                } else {
                    glue.picker.enter_normal();
                }
            }
            Key::Enter if replace => self.apply_replace(),
            Key::Enter => {
                let payload = glue.picker.current().map(|i| i.payload.clone());
                self.picker = None;
                if let Some(p) = payload {
                    self.accept_picker(p);
                }
            }
            // 0007 §2: Tab cycles the two fields; results nav is
            // arrows / ctrl-n,p while a field has focus
            Key::Tab | Key::Backtab if replace => glue.picker.toggle_field(),
            Key::CtrlX if replace => glue.picker.toggle_excluded(),
            Key::CtrlD if replace => glue.picker.toggle_file_excluded(),
            Key::CtrlD => {}
            Key::CtrlX | Key::CtrlO => {}
            Key::Backspace => {
                if glue.picker.input_normal() {
                    glue.picker.normal_key('h'); // vim: bs in normal = h
                } else if replace && glue.picker.field == strop_picker::Field::Replace {
                    glue.picker.pop_replace_char();
                } else {
                    glue.picker.pop_char();
                    self.picker_input_changed();
                }
            }
            Key::CtrlR | Key::CtrlW => {}
            Key::CtrlU | Key::CtrlF | Key::CtrlB | Key::CtrlV | Key::CtrlCaret => {}
            Key::Up => glue.picker.move_by(-1),
            Key::Down => glue.picker.move_by(1),
            Key::Tab => glue.picker.move_by(1),
            Key::Backtab => glue.picker.move_by(-1),
            // arrows: Up/Down walk results, Left/Right move the caret
            Key::Left => glue.picker.caret_left(),
            Key::Right => glue.picker.caret_right(),
            // picker normal mode (Esc): the field is one line, so h/l
            // own the caret and j/k walk the results — the muscle
            // memory you bring from the buffer
            Key::Char('j') if glue.picker.input_normal() => glue.picker.move_by(1),
            Key::Char('k') if glue.picker.input_normal() => glue.picker.move_by(-1),
            Key::Char(c) => {
                if glue.picker.input_normal() {
                    // modal editing on the field (0003 §1); x/X change
                    // the text → respawn
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

    fn picker_input_changed(&mut self) {
        let Some(glue) = &mut self.picker else { return };
        if matches!(glue.picker.kind, Kind::Grep | Kind::Replace) {
            // rg filters; kill + respawn per keystroke (worker is cheap).
            // A fresh channel per respawn: the old worker's messages (incl.
            // its trailing Done) fail to send on the dropped receiver, so
            // stale generations can't race the new one.
            let pattern = glue.picker.input.text.clone();
            let cwd = self.cwd.clone();
            glue.picker.error = None;
            glue.grep_worker = None; // drop kills the old rg
            glue.picker.items.clear();
            glue.picker.rows.clear(); // stale item indices must never render
            glue.picker.excluded.clear(); // item indices die with the respawn
            let (tx, rx) = channel();
            glue.tx = Some(tx.clone());
            glue.rx = Some(rx);
            glue.grep_worker = GrepWorker::spawn(&pattern, &cwd, tx);
            glue.picker.streaming = glue.grep_worker.is_some();
            glue.gen += 1;
            // the respawn's channel must reach the SAME event source as
            // the initial stream (0020 §2 — 0.9.0 silently dropped it)
            if let Some(app_tx) = &self.app_tx {
                let (id, gen) = (glue.id, glue.gen);
                let atx = app_tx.clone();
                if let Some(rx) = glue.rx.take() {
                    std::thread::spawn(move || {
                        while let Ok(msg) = rx.recv() {
                            if atx
                                .send(super::events::AppEvent::Picker { id, gen, msg })
                                .is_err()
                            {
                                break;
                            }
                        }
                    });
                }
            }
        } else {
            glue.picker.refilter();
        }
    }
}

pub struct PreviewEntry {
    pub rope: ropey::Rope,
    pub hl: Option<Highlighter>,
}

pub enum PreviewSource<'a> {
    /// Live document, highlighted with its own highlighter.
    Buffer(strop_core::id::DocumentId),
    Cached(&'a mut PreviewEntry),
    /// Worker read still in flight (or unreadable); render shows a
    /// placeholder, never blocks.
    Loading,
}

pub type Previews = HashMap<PathBuf, PreviewEntry>;
