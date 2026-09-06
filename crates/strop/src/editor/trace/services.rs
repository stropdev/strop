//! Shared service-handler producers: TUI deliveries and headless drains both
//! reach these handlers. Never record raw clipboard or shell-output payloads.
use crate::editor::{GitJob, ShellResult};
use serde_json::json;
use strop_trace::{record_with, EventKind};

pub fn rejected(service: &'static str, reason: &str) {
    record_with(
        EventKind::JobRejected,
        || json!({"service":service,"reason":reason}),
    );
}

pub fn git(job: &GitJob) {
    record_with(EventKind::JobFinished, || match job {
        GitJob::Log {
            buffer,
            generation,
            rows,
        } => {
            json!({"service":"git","result":"log","document":super::snapshot::document_id(*buffer),"generation":generation,"rows":rows.len()})
        }
        GitJob::Card { generation, .. } => {
            json!({"service":"git","result":"blame_card","generation":generation})
        }
        GitJob::Gutter {
            path,
            generation,
            lines,
        } => {
            json!({"service":"git","result":"gutter","path":path.to_string_lossy(),"generation":generation,"lines":lines.len()})
        }
        GitJob::Hunks {
            doc,
            epoch,
            staged,
            unstaged,
        } => {
            json!({"service":"git","result":"hunks","document":super::snapshot::document_id(*doc),"revision":epoch,"staged":staged.len(),"unstaged":unstaged.len()})
        }
        GitJob::Error(message) => json!({"service":"git","result":"error","message":message}),
    });
}

pub fn shell(result: &ShellResult) {
    record_with(EventKind::JobFinished, || match result {
        ShellResult::Display { cmd, output } => {
            json!({"service":"shell","result":"display","command":cmd,"output_bytes":output.len()})
        }
        ShellResult::Pipe {
            buffer,
            start,
            end,
            original,
            output,
            ok,
            err,
        } => json!({
            "service":"shell","result":"pipe","document":super::snapshot::document_id(*buffer),
            "start_byte":start,"end_byte":end,"original_bytes":original.len(),"output_bytes":output.len(),"success":ok,"error":err,
        }),
    });
}

pub fn lsp(event: &strop_lsp::LspEvent) {
    use strop_lsp::LspEvent;
    record_with(EventKind::JobFinished, || match event {
        LspEvent::Ready { server } => json!({"service":"lsp","result":"ready","server":server}),
        LspEvent::Failed { server, hint } => {
            json!({"service":"lsp","result":"failed","server":server,"hint":hint})
        }
        LspEvent::Diagnostics {
            path,
            diags,
            version,
        } => {
            json!({"service":"lsp","result":"diagnostics","path":path.to_string_lossy(),"version":version,
            "diagnostics":diags.iter().map(|diag|json!({"line":diag.line,"column":diag.col,"severity":diag.severity,"message":diag.message})).collect::<Vec<_>>()})
        }
        LspEvent::Note { text } => json!({"service":"lsp","result":"note","message":text}),
        LspEvent::HoverText { text } => {
            json!({"service":"lsp","result":"hover","bytes":text.len()})
        }
        LspEvent::GotoLocation {
            path,
            line,
            col,
            req_revision,
        } => {
            json!({"service":"lsp","result":"goto","path":path.to_string_lossy(),"line":line,"column":col,"requested_revision":req_revision})
        }
        LspEvent::Locations {
            kind,
            items,
            req_revision,
        } => {
            json!({"service":"lsp","result":kind.label(),"items":items.len(),"requested_revision":req_revision})
        }
    });
}

pub fn picker(message: &strop_picker::PickerMsg) {
    use strop_picker::PickerMsg;
    record_with(EventKind::JobFinished, || match message {
        PickerMsg::Items(items) => json!({"service":"picker","result":"items","count":items.len()}),
        PickerMsg::Done => json!({"service":"picker","result":"done"}),
        PickerMsg::Error(error) => json!({"service":"picker","result":"error","message":error}),
    });
}
