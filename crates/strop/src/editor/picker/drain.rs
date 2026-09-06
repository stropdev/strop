//! Picker job draining: worker messages onto the editor state,
//! called from the event loop between keystrokes (0001 §5.6).

use std::path::PathBuf;

use strop_picker::PickerMsg;
use strop_syntax::Highlighter;

use super::super::Editor;
use super::PreviewEntry;

impl Editor {
    /// Drain worker messages (called from the event loop each tick).
    pub fn drain_picker(&mut self) {
        let mut done = false;
        if let Some(glue) = &mut self.picker {
            if let Some(rx) = &glue.rx {
                let mut items = Vec::new();
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        PickerMsg::Items(batch) => items.extend(batch),
                        PickerMsg::Error(e) => glue.picker.error = Some(e),
                        PickerMsg::Done => done = true,
                    }
                }
                let _ = &done;
                if !items.is_empty() {
                    glue.picker.append(items);
                }
            }
            if done {
                glue.picker.streaming = false;
            }
        }
        self.drain_previews();
    }

    /// One picker stream message (TUI forwards land here — 0018).
    pub(crate) fn handle_picker_msg(&mut self, msg: PickerMsg) {
        if let Some(glue) = &mut self.picker {
            match msg {
                PickerMsg::Items(batch) => glue.picker.append(batch),
                PickerMsg::Error(e) => glue.picker.error = Some(e),
                PickerMsg::Done => glue.picker.streaming = false,
            }
        }
    }

    /// Drain preview worker results (file reads happen off the render
    /// path — 0001 §3). Headless path; the TUI forwards each result as
    /// an AppEvent (0018).
    fn drain_previews(&mut self) {
        loop {
            let next = self.preview_rx.as_ref().and_then(|rx| rx.try_recv().ok());
            match next {
                Some((path, text)) => self.handle_preview(path, text),
                None => break,
            }
        }
    }

    /// One preview worker result → cached entry.
    pub(crate) fn handle_preview(&mut self, path: PathBuf, text: Option<String>) {
        self.preview_inflight.remove(&path);
        if let Some(text) = text {
            let rope = ropey::Rope::from_str(&text);
            let hl = Highlighter::for_path(&path);
            self.previews.insert(path, PreviewEntry { rope, hl });
        } else {
            // unreadable: cache the miss so we don't respawn per frame
            self.previews.insert(
                path,
                PreviewEntry {
                    rope: ropey::Rope::from_str(""),
                    hl: None,
                },
            );
        }
    }
}
