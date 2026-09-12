//! Ticket-stamped picker deliveries. Live and headless drivers both
//! forward worker messages through the shared AppEvent channel.

use std::sync::mpsc::Receiver;

use strop_core::worker::{FailureKind, Load, Outcome, Ticket};
use strop_picker::PickerMsg;

use super::super::events::AppEvent;
use super::super::trace;
use super::super::Editor;
use super::{PickerEvent, PickerKey, PreviewEntry, PreviewResult};

/// Bridge one request's raw stream onto the app event channel, stamping
/// every message with the ticket that produced it. A disconnect before
/// the terminal event synthesizes one — streaming can never hang; after
/// the terminal event nothing else is forwarded.
pub(crate) fn forward_picker_stream(
    rx: Receiver<PickerMsg>,
    ticket: Ticket<PickerKey>,
    tx: super::super::events::EventSender,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("picker-bridge".into())
        .spawn(move || loop {
            match rx.recv() {
                Ok(msg) => {
                    let terminal = matches!(msg, PickerMsg::Finished(_));
                    if tx
                        .send(AppEvent::Picker(PickerEvent {
                            ticket: ticket.clone(),
                            msg,
                        }))
                        .is_err()
                    {
                        return; // event loop gone
                    }
                    if terminal {
                        return;
                    }
                }
                Err(_) => {
                    // the worker's channel died before any terminal
                    // event: synthesize one with the same ticket
                    let outcome =
                        Outcome::failed(FailureKind::Disconnected, "picker worker channel closed");
                    let _ = tx.send(AppEvent::Picker(PickerEvent {
                        ticket: ticket.clone(),
                        msg: PickerMsg::Finished(outcome),
                    }));
                    return;
                }
            }
        })
        .map(|_| ())
}

impl Editor {
    /// One picker stream event: the TUI's AppEvent deliveries and the
    /// headless drain both land here. Only the owning ticket may touch
    /// the model; everything else is a trace-level rejection.
    pub(crate) fn handle_picker_event(&mut self, event: PickerEvent) {
        trace::services::picker(&event);
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        if glue.active.as_ref() != Some(&event.ticket) {
            trace::services::rejected("picker", "picker request superseded or completed");
            return;
        }
        let appended = matches!(&event.msg, PickerMsg::Items(_));
        match event.msg {
            PickerMsg::Items(items) => glue.picker.append(items.into_items()),
            PickerMsg::Warning(message) => glue.picker.warning = Some(message),
            PickerMsg::QueryError(diagnostic) => {
                let range = match (&glue.query, &glue.file_scope) {
                    (Some(current), Some(previous)) => {
                        current.diagnostic_range_from(previous, &diagnostic)
                    }
                    _ => diagnostic.range.clone(),
                };
                glue.query_highlights
                    .push(strop_picker::query::HighlightSpan {
                        range,
                        role: strop_picker::query::Role::Error,
                    });
                glue.picker.error = Some(diagnostic.message);
                glue.accept_when_ranked = false;
                glue.file_scope = None;
            }
            PickerMsg::Finished(outcome) => {
                // exactly-once terminal: settle streaming, drop the
                // worker (its Drop kills/reaps any live rg), keep
                // whatever items already streamed — a failure never
                // erases the useful partial results
                glue.active = None;
                glue.rx = None;
                glue.picker.streaming = false;
                glue.worker = None;
                if let Outcome::Failed { failure, .. } = outcome {
                    glue.picker.error = Some(failure.message);
                    glue.accept_when_ranked = false;
                    glue.file_scope = None;
                }
            }
        }
        if appended {
            self.request_picker_ranking();
        }
        self.finish_pending_picker_accept();
    }

    /// One preview completion → cached entry. Only the exact registered
    /// ticket for the path may publish; failures render blank (as
    /// before) but stay owned — a reopen retries instead of caching
    /// the miss forever.
    pub(crate) fn handle_preview(&mut self, result: PreviewResult) {
        let path = result.ticket.key.path.clone();
        if !self
            .preview_loads
            .get(&path)
            .is_some_and(|load| load.owns(&result.ticket))
        {
            trace::services::rejected("preview", "preview request superseded");
            return;
        }
        self.worker_handles.remove(&result.ticket.request);
        self.preview_loads.remove(&path);
        if !self
            .picker
            .as_ref()
            .is_some_and(|glue| glue.id == result.ticket.key.picker)
        {
            trace::services::rejected("preview", "picker closed");
            return;
        }
        strop_trace::record_with(strop_trace::EventKind::JobFinished, || {
            let outcome = match &result.outcome {
                Outcome::Success(text) => {
                    serde_json::json!({"result":"success","bytes":text.rope.len_bytes()})
                }
                Outcome::Failed { failure, .. } => serde_json::json!({
                    "result":"failed","kind":format!("{:?}",failure.kind),
                    "message":failure.message}),
                Outcome::Cancelled(reason) => serde_json::json!({
                    "result":"cancelled","reason":format!("{reason:?}")}),
            };
            serde_json::json!({
                "service":"preview","request":result.ticket.request.get(),
                "path":path.to_string_lossy(),"outcome":outcome,
            })
        });
        let key = result.ticket.key;
        match result.outcome {
            Outcome::Success(prepared) => {
                const CACHED_PREVIEWS: usize = 32;
                if self.previews.len() >= CACHED_PREVIEWS && !self.previews.contains_key(&path) {
                    if let Some(old) = self.previews.keys().next().cloned() {
                        self.previews.remove(&old);
                        self.preview_loads.remove(&old);
                        self.analysis
                            .forget(super::super::analysis::AnalysisTarget::Preview(old));
                    }
                }
                self.analysis
                    .forget(super::super::analysis::AnalysisTarget::Preview(
                        path.clone(),
                    ));
                self.previews.insert(
                    path.clone(),
                    PreviewEntry {
                        rope: prepared.rope,
                    },
                );
                self.preview_loads.insert(path, Load::Ready(key));
            }
            Outcome::Failed { failure, .. } => {
                self.message = format!("preview: {}", failure.message);
                self.preview_loads
                    .insert(path, Load::Failed { key, failure });
            }
            Outcome::Cancelled(reason) => {
                self.preview_loads
                    .insert(path, Load::Cancelled { key, reason });
            }
        }
    }
}
