//! Picker job draining: worker messages onto the editor state,
//! called from the event loop between keystrokes (0001 §5.6).

use std::path::PathBuf;

use strop_picker::PickerMsg;
use strop_syntax::Highlighter;

use super::super::trace;
use super::super::Editor;
use super::PreviewEntry;

impl Editor {
    /// Drain worker messages (called from the event loop each tick).
    pub fn drain_picker(&mut self) {
        loop {
            let next = self
                .picker
                .as_ref()
                .and_then(|glue| glue.rx.as_ref())
                .and_then(|receiver| receiver.try_recv().ok());
            let Some(message) = next else { break };
            self.handle_picker_msg(message);
        }
        self.drain_previews();
    }

    /// One picker stream message (TUI forwards land here — 0018).
    pub(crate) fn handle_picker_msg(&mut self, msg: PickerMsg) {
        trace::services::picker(&msg);
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
        strop_trace::record_with(
            strop_trace::EventKind::JobFinished,
            || serde_json::json!({"service":"preview","path":path.to_string_lossy(),"bytes":text.as_ref().map(String::len),"available":text.is_some()}),
        );
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
