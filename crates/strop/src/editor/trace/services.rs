//! Shared service-handler producers: TUI deliveries and headless drains both
//! reach these handlers. Never record raw clipboard or shell-output payloads.
//!
//! `NativePath` is the tape's path-carrying argument wrapper: native paths
//! serialize through strop-core's versioned encoding so replay never depends
//! on lossy UTF-8 conversion.
use crate::editor::git_memory::GitJob;
use crate::editor::{ClipboardResult, ShellResult};
use serde_json::json;
use strop_trace::{record_with, EventKind};

/// A path crossing the tape boundary. Reusable where a tuple carries a
/// path; structs with path fields use `#[serde(with=...)]` directly.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NativePath(#[serde(with = "strop_core::path_serde")] pub std::path::PathBuf);

pub fn rejected(service: &'static str, reason: &str) {
    record_with(
        EventKind::JobRejected,
        || json!({"service":service,"reason":reason}),
    );
}
fn outcome<T>(value: &strop_core::worker::Outcome<T>) -> serde_json::Value {
    use strop_core::worker::Outcome;
    match value {
        Outcome::Success(_) => json!({"kind":"success"}),
        Outcome::Failed { failure, partial } => {
            json!({"kind":"failed","failure":failure,"partial":partial.is_some()})
        }
        Outcome::Cancelled(reason) => json!({"kind":"cancelled","reason":reason}),
    }
}

fn completion<K: serde::Serialize, T>(
    service: &str,
    result: &str,
    value: &strop_core::worker::Completion<K, T>,
) -> serde_json::Value {
    json!({"service":service,"result":result,"ticket":value.ticket,"outcome":outcome(&value.outcome)})
}

pub fn git(job: &GitJob) {
    record_with(EventKind::JobFinished, || match job {
        GitJob::Context(value) => completion("git", "context", value),
        GitJob::Hunks(value) => completion("git", "hunks", value),
        GitJob::Mutation(value) => completion("git", "mutation", value),
        GitJob::Log(value) => completion("git", "log", value),
        GitJob::Gutter(value) => completion("git", "gutter", value),
        GitJob::Card(value) => completion("git", "blame_card", value),
        GitJob::Dive(value) => completion("git", "dive", value),
    });
}

pub fn io(event: &crate::editor::io::IoEvent) {
    use crate::editor::io::IoEvent;
    record_with(EventKind::JobFinished, || match event {
        IoEvent::Open(value) => completion("io", "open", value),
        IoEvent::Save(value) => completion("io", "save", value),
        IoEvent::Native(value) => completion("io", "native", value),
        IoEvent::Session {
            request,
            outcome: value,
        } => json!({"service":"io","result":"session","request":request,"outcome":outcome(value)}),
    });
}

pub fn shell(result: &ShellResult) {
    record_with(EventKind::JobFinished, || {
        let key = match &result.ticket.key {
            crate::editor::ShellKey::Display {
                origin, revision, ..
            } => json!({
                "kind":"display","document":super::snapshot::document_id(*origin),
                "revision":revision,
            }),
            crate::editor::ShellKey::Pipe {
                document,
                revision,
                start,
                end,
            } => json!({
                "kind":"pipe","document":super::snapshot::document_id(*document),
                "revision":revision,"start_byte":start,"end_byte":end,
            }),
        };
        let outcome = match &result.outcome {
            strop_core::worker::Outcome::Success(output) => json!({
                "stdout_bytes":output.stdout.len(),"stderr_bytes":output.stderr.len(),
            }),
            strop_core::worker::Outcome::Failed { failure, .. } => {
                json!({"error": failure.message})
            }
            _ => json!({"cancelled": true}),
        };
        json!({"service":"shell","request":result.ticket.request.get(),
            "key":key,"outcome":outcome})
    });
}

pub fn clipboard(result: &ClipboardResult) {
    record_with(EventKind::JobFinished, || {
        let outcome = match &result.outcome {
            strop_core::worker::Outcome::Success(text) => json!({"bytes": text.len()}),
            strop_core::worker::Outcome::Failed { failure, .. } => {
                json!({"error": failure.message})
            }
            _ => json!({"cancelled": true}),
        };
        json!({"service":"clipboard","request":result.ticket.request.get(),
            "document":super::snapshot::document_id(result.ticket.key.document),
            "outcome":outcome})
    });
}

pub fn lsp(event: &strop_lsp::LspEvent) {
    use strop_lsp::LspEvent;
    record_with(EventKind::JobFinished, || {
        if strop_trace::capture_content() {
            return json!({"service":"lsp", "event":event});
        }
        match event {
            LspEvent::Ready { server, name } => {
                json!({"service":"lsp","result":"ready","server":server,"name":name})
            }
            LspEvent::Failed { server, name, hint } => {
                // The hint carries the executable and the fix (0033 §3)
                json!({"service":"lsp","result":"failed","server":server,"name":name,"hint":hint})
            }
            LspEvent::Diagnostics { context, diags, .. } => json!({
                "service":"lsp","result":"diagnostics","context":context,"count":diags.len(),
            }),
            LspEvent::HoverText { context, text } => json!({
                "service":"lsp","result":"hover","context":context,"bytes":text.len(),
            }),
            LspEvent::Note { context, text } => json!({
                "service":"lsp","result":"note","context":context,"bytes":text.len(),
            }),
            LspEvent::GotoLocation { context, .. } => json!({
                "service":"lsp","result":"goto","context":context,
            }),
            LspEvent::Locations {
                context,
                kind,
                items,
            } => json!({
                "service":"lsp","result":kind.label(),"context":context,"count":items.len(),
            }),
        }
    });
}

pub fn picker(event: &crate::editor::picker::PickerEvent) {
    use strop_picker::PickerMsg;
    record_with(EventKind::JobFinished, || {
        let outcome = match &event.msg {
            PickerMsg::Items(items) => json!({"result":"items","count":items.len()}),
            PickerMsg::Warning(warning) => json!({"result":"warning","bytes":warning.len()}),
            PickerMsg::Finished(outcome) => match outcome {
                strop_core::worker::Outcome::Success(()) => json!({"result":"finished"}),
                strop_core::worker::Outcome::Failed { failure, .. } => {
                    json!({"result":"error","error":failure.message})
                }
                _ => json!({"result":"cancelled"}),
            },
        };
        json!({"service":"picker","request":event.ticket.request.get(),
            "outcome":outcome})
    });
}
