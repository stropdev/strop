//! The unified event source (0018 §services): every async producer —
//! terminal, LSP, git, shell, picker, clipboard — lands on ONE channel
//! as a typed `AppEvent`. The main loop parks on it; workers wake the
//! loop the instant they post. Gone: the 500ms poll latency between a
//! job finishing and the UI noticing.
//!
//! The transport itself is bounded (0056 AR06): wake hints coalesce,
//! semantic events are admitted under count/byte bounds with visible
//! refusal, and native readers backpressure off the UI thread. Fairness
//! (EVENTS_PER_TURN/TURN_BUDGET) stays a scheduling property, not a
//! substitute for a retention bound.
//!
//! Forwarder threads move each job channel into the app channel. The
//! headless harness keeps the raw channels (no forwarders) and drives
//! the same per-event handlers through the drains.

use std::sync::mpsc::Receiver;
mod channel;
pub use channel::{
    channel, AdmissionRefusal, EventReceiver, EventSender, RecvTimeoutError, TryRecvError,
    EVENTS_PER_TURN, MAX_QUEUED_PASTE_BYTES, MAX_SEMANTIC_EVENTS, TURN_BUDGET,
};

use super::{Editor, Key, ShellResult};

/// External input retains its physical facts until the engine selects an owner.
/// EditorKey is already-normalized semantic input from scripted/editor commands.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum AppEvent {
    Input(strop_core::frontend_input::Input),
    EditorKey(Key),
    TerminalUpdate(strop_terminal::model::SessionId),
    Focus(bool),
    /// Terminal resized — a redraw is owed even with no input (0020 §12).
    Resize {
        columns: u16,
        rows: u16,
    },
    /// Bracketed paste: one text payload, never a key stream.
    Paste(String),
    /// ctrl-c: the quit intent (0015's policy lives in the editor).
    QuitIntent,
    Lsp(strop_lsp::LspEvent),
    LspAttach(super::lsp::attach::AttachRecord),
    Shell(ShellResult),
    Io(super::io::IoEvent),
    RemoteCompletion(Box<super::remote_completion::RemoteCompletionEvent>),
    Container(super::containers::ContainerEvent),
    Git(super::GitJob),
    Picker(super::picker::PickerEvent),
    PickerRanking(super::picker::ranking::Event),
    Analysis(super::analysis::AnalysisEvent),
    Resolution(super::resolution::ResolutionEvent),
    ResumeInput,
    Preview(super::picker::PreviewResult),
    /// Filesystem notifications landed on the bounded notify queue
    /// (0058 S7): a pure wake hint — the records ARE the state, so
    /// coalescing is legal (AR06).
    Notify,
    Clipboard(super::ClipboardResult),
}

/// A forwarder: move every item of a job channel onto the app channel.
fn forward<T: Send + 'static>(
    rx: Receiver<T>,
    tx: EventSender,
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
    pub fn connect_events(&mut self, tx: EventSender) {
        if let Some(rx) = self.terminals.rx.take() {
            forward(rx, tx.clone(), AppEvent::TerminalUpdate);
        }
        if let Some(rx) = self.io.rx.take() {
            forward(rx, tx.clone(), AppEvent::Io);
        }
        if let Some(rx) = self.remote_completion.rx.take() {
            forward(rx, tx.clone(), |event| {
                AppEvent::RemoteCompletion(Box::new(event))
            });
        }
        if let Some(rx) = self.containers.take_rx() {
            forward(rx, tx.clone(), AppEvent::Container);
        }
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
            forward(rx, tx.clone(), AppEvent::Preview);
        }
        if let Some(rx) = self.picker_ranking.rx.take() {
            forward(rx, tx.clone(), AppEvent::PickerRanking);
        }
        if let Some(rx) = self.analysis.rx.take() {
            forward(rx, tx.clone(), AppEvent::Analysis);
        }
        if let Some(rx) = self.resolution.rx.take() {
            forward(rx, tx.clone(), AppEvent::Resolution);
        }
        if let Some(rx) = self.notify.take_rx() {
            let queue = std::sync::Arc::clone(&self.notify.queue);
            forward(rx, tx.clone(), move |event| {
                queue.push_event(event);
                AppEvent::Notify
            });
        }
        self.connect_picker_stream(&tx);
        for srv in &mut self.lsp_servers {
            let rx = std::mem::replace(&mut srv.rx, std::sync::mpsc::channel().1);
            forward(rx, tx.clone(), AppEvent::Lsp);
        }
        let rx = self.lsp_state.attach.take_rx();
        forward(rx, tx.clone(), AppEvent::LspAttach);
        self.app_tx = Some(tx);
        // 0058 S7: the local scope subscription starts with the event
        // loop, never at construction (pure seeding) — readiness is the
        // subscribe settle on the notify queue.
        self.start_notifications();
    }

    /// Route one event to its handler (the per-event halves of the old
    /// drain loops; the drains call these in a try_recv loop).
    pub fn handle_app_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Input(input) => self.handle_frontend_input(input),
            AppEvent::TerminalUpdate(session) => self.handle_terminal_update(session),
            AppEvent::Focus(focused) => self.terminal_focus_changed(focused),
            AppEvent::EditorKey(key) => self.feed(key),
            AppEvent::Resize { .. } => {} // the loop redraws after every event
            AppEvent::Paste(text) => {
                if self.terminal_owns_input() {
                    self.feed_terminal(strop_core::frontend_input::Input::Paste(text));
                    return;
                }
                let bytes = text.len();
                let excerpt = strop_trace::capture_content().then(|| strop_trace::excerpt(&text));
                strop_trace::record_with(strop_trace::EventKind::Paste, || match &excerpt {
                    Some((text, truncated)) => serde_json::json!({
                        "bytes": bytes, "text": text, "truncated": truncated,
                    }),
                    None => serde_json::json!({"bytes": bytes}),
                });
                if self.resolution.blocked() || !self.resolution.queue.is_empty() {
                    self.resolution
                        .queue
                        .push_back(super::resolution::DeferredInput::Paste(text));
                    return;
                }
                self.paste_bracketed(&text);
            }
            AppEvent::Notify => self.handle_notify(),
            AppEvent::QuitIntent => {
                strop_trace::record_with(
                    strop_trace::EventKind::Input,
                    || serde_json::json!({"action":"quit_intent","source":"external"}),
                );
                self.resolution.cancel();
                self.resolution.queue.clear();
                if self.ctrl_c_quit() {
                    self.should_quit = true;
                }
            }
            AppEvent::Lsp(event) => self.handle_lsp_event(event),
            AppEvent::LspAttach(record) => self.handle_lsp_attach(record),
            AppEvent::Shell(r) => self.handle_shell_result(r),
            AppEvent::Io(event) => self.handle_io(event),
            AppEvent::RemoteCompletion(event) => self.handle_remote_completion(*event),
            AppEvent::Container(event) => self.handle_container_event(event),
            AppEvent::Git(job) => self.handle_git_job(job),
            AppEvent::Picker(event) => self.handle_picker_event(event),
            AppEvent::PickerRanking(event) => self.handle_picker_ranking(event),
            AppEvent::Analysis(event) => self.handle_analysis(event),
            AppEvent::Resolution(event) => self.handle_resolution(event),
            AppEvent::ResumeInput => self.resume_resolution_input(),
            AppEvent::Preview(result) => self.handle_preview(result),
            AppEvent::Clipboard(content) => self.handle_clipboard(content),
        }
        // 0058 S7: a worker restart kills the subscription with its
        // session; observe the lease (cheap, no I/O) before staleness.
        self.notify_observe_lease();
        // Coalesced draft checkpointing (0056 AR04): cheap staleness check
        // after every event; captures only what actually moved.
        self.recovery_after_event();
        self.surface_event_refusals();
    }

    /// A refused admission is visible (0056 AR06): the bounded lane
    /// records every refusal and the loop reports it rather than
    /// pretending the event landed.
    fn surface_event_refusals(&mut self) {
        let Some(tx) = &self.app_tx else { return };
        let refusals = tx.take_refusals();
        if refusals.is_empty() {
            return;
        }
        let mut classes: std::collections::BTreeMap<&'static str, usize> =
            std::collections::BTreeMap::new();
        for refusal in &refusals {
            *classes.entry(refusal.class).or_insert(0) += 1;
        }
        let summary = classes
            .iter()
            .map(|(class, count)| format!("{count}×{class}"))
            .collect::<Vec<_>>()
            .join(", ");
        self.message = format!("event queue full — refused: {summary}");
    }
}
impl Editor {
    /// Outstanding finite work, independent of whether channels are
    /// forwarded (0056 AR06): start/handshake, mutation, checkpoint and
    /// stopping steps — never the liveness of a long-lived service. A
    /// running terminal session or a ready language server is NOT
    /// pending; a terminal in Starting/Closing or a server in
    /// start/handshake is, until it reaches its terminal outcome. Once
    /// `finishing` is set, LSP start/readiness stops counting: shutdown
    /// quiesces and closes services rather than waiting on them.
    pub fn async_pending(&self) -> bool {
        use strop_core::worker::Load;
        self.io_pending()
            || self.terminals.pending()
            || !self.shell_requests.is_empty()
            || self.clip_paste_pending.is_some()
            || self
                .picker
                .as_ref()
                .is_some_and(|glue| glue.picker.streaming || glue.rank_pending.is_some())
            || !self.picker_ranking.retiring.is_empty()
            || self
                .picker_source
                .as_ref()
                .is_some_and(strop_picker::SourceWorker::busy)
            || self.analysis.pending()
            || self.resolution.pending()
            || self
                .preview_loads
                .values()
                .any(|load| matches!(load, Load::Running(_)))
            || matches!(self.git_discovery, Load::Running(_))
            || matches!(self.hunk_load, Load::Running(_))
            || !self.log_requests.is_empty()
            || !self.dive_requests.is_empty()
            || self.card_request.is_some()
            || self.git_mutation.is_some()
            || !self.git_mutations.is_empty()
            || self
                .blame_gutters
                .values()
                .any(|gutter| gutter.request.is_some())
            || self.containers.pending.is_some()
            || !self.lsp_state.attach.pending.is_empty()
            || self.remote_completion.pending.is_some()
            || (!self.finishing
                && (self.lsp_state.hover.is_some()
                    || self.lsp_state.navigation.is_some()
                    || self.lsp_servers.iter().any(|server| !server.ready)))
    }

    /// Finish is an explicit action in both modes. Preserve accepted writes;
    /// cancel observational work so shutdown cannot depend on a slow reader.
    pub(crate) fn finish_background_work(&mut self) {
        self.finishing = true;
        self.lsp_state.attach.enabled = false;
        self.stop_all_terminals();
        self.stop_remote_work();
        self.close_picker();
        // 0058 S7: the subscription dies with the session — typed
        // retirement on the lease, then the lease drop reaps the worker.
        if let Some(subscription) = self.notify.subscription.take() {
            let worker = self.filesystem.worker().clone();
            std::thread::spawn(move || {
                let (token, _handle) = strop_core::worker::CancelToken::standalone();
                let _ = worker.unsubscribe(&token, subscription);
            });
        }
        if let Some(source) = self.picker_source.as_ref() {
            source.close();
        }
        self.cancel_review_preparation();
        self.analysis.stop();
        self.resolution.stop();
        self.git_mutations.clear();
        self.request_session_save();
        self.recovery_on_finish();
        let cancel: Vec<_> = self
            .worker_handles
            .keys()
            .copied()
            .filter(|id| !self.io_write_pending(*id))
            .collect();
        for request in cancel {
            if let Some(handle) = self.worker_handles.remove(&request) {
                handle.cancel(strop_core::worker::CancelReason::Shutdown);
            }
        }
    }

    /// Report admitted effects that did not reach a confirmed shutdown outcome.
    pub fn take_shutdown_error(&mut self) -> Option<String> {
        let mut errors: Vec<String> = [
            self.io.session_error.take(),
            self.filesystem_shutdown_error(),
            self.recovery.last_error.take(),
        ]
        .into_iter()
        .flatten()
        .collect();
        match errors.len() {
            0 => None,
            1 => errors.pop(),
            _ => Some(errors.join("\n")),
        }
    }
}

#[cfg(test)]
#[path = "events/tests.rs"]
mod tests;
