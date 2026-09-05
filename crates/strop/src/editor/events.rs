//! The unified event source (0018 §services): every async producer —
//! terminal, LSP, git, shell, picker, clipboard — lands on ONE channel
//! as a typed `AppEvent`. The main loop parks on it; workers wake the
//! loop the instant they post. Gone: the 500ms poll latency between a
//! job finishing and the UI noticing.
//!
//! Forwarder threads move each job channel into the app channel. The
//! headless harness keeps the raw channels (no forwarders) and drives
//! the same per-event handlers through the drains.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use strop_picker::PickerMsg;

use super::{Editor, Key, ShellResult};

/// One app event. Terminal input is already translated to editor keys
/// by the reader thread.
pub enum AppEvent {
    Terminal(Key),
    /// Bracketed paste: one text payload, never a key stream.
    Paste(String),
    /// ctrl-c: the quit intent (0015's policy lives in the editor).
    QuitIntent,
    Lsp(strop_lsp::PositionEncoding, strop_lsp::LspEvent),
    Shell(ShellResult),
    Git(super::GitJob),
    Picker(PickerMsg),
    Preview(PathBuf, Option<String>),
    Clipboard(Option<String>),
}

/// A forwarder: move every item of a job channel onto the app channel.
fn forward<T: Send + 'static>(
    rx: Receiver<T>,
    tx: Sender<AppEvent>,
    wrap: impl Fn(T) -> AppEvent + Send + 'static,
) {
    std::thread::spawn(move || {
        while let Ok(item) = rx.recv() {
            if tx.send(wrap(item)).is_err() {
                break;
            }
        }
    });
}

impl Editor {
    /// Connect the editor's job channels to the app event channel
    /// (TUI only — headless keeps the raw channels for its drains).
    /// Late-attaching LSP servers forward through the retained sender.
    pub fn connect_events(&mut self, tx: Sender<AppEvent>) {
        if let Some(rx) = self.shell_rx.take() {
            forward(rx, tx.clone(), AppEvent::Shell);
        }
        if let Some(rx) = self.git_rx.take() {
            forward(rx, tx.clone(), AppEvent::Git);
        }
        if let Some(rx) = self.clip_rx.take() {
            forward(rx, tx.clone(), AppEvent::Clipboard);
        }
        if let Some(rx) = self.preview_rx.take() {
            forward(rx, tx.clone(), |(p, c)| AppEvent::Preview(p, c));
        }
        if let Some(glue) = &mut self.picker {
            if let Some(rx) = glue.take_rx() {
                forward(rx, tx.clone(), AppEvent::Picker);
            }
        }
        for srv in &mut self.lsp_servers {
            let enc = srv.client.encoding();
            let rx = std::mem::replace(&mut srv.rx, std::sync::mpsc::channel().1);
            forward(rx, tx.clone(), move |ev| AppEvent::Lsp(enc, ev));
        }
        self.app_tx = Some(tx);
    }

    /// Route one event to its handler (the per-event halves of the old
    /// drain loops; the drains call these in a try_recv loop).
    pub fn handle_app_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Terminal(key) => self.feed(key),
            AppEvent::Paste(text) => self.paste_bracketed(&text),
            AppEvent::QuitIntent => {
                if self.ctrl_c_quit() {
                    self.should_quit = true;
                }
            }
            AppEvent::Lsp(enc, ev) => self.handle_lsp_event(enc, ev),
            AppEvent::Shell(r) => self.handle_shell_result(r),
            AppEvent::Git(job) => self.handle_git_job(job),
            AppEvent::Picker(msg) => self.handle_picker_msg(msg),
            AppEvent::Preview(path, content) => self.handle_preview(path, content),
            AppEvent::Clipboard(content) => self.handle_clipboard(content),
        }
    }
}
