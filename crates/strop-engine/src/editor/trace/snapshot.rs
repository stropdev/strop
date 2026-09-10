//! State snapshots retain native paths and complete prompt ownership.
use crate::editor::Editor;
use serde_json::json;
use strop_core::id::DocumentId;
use strop_trace::{capture_content, enabled, record, EventKind};

pub(super) fn document_id(id: DocumentId) -> serde_json::Value {
    json!(id)
}

fn pending_state(editor: &Editor) -> serde_json::Value {
    let Some(prompt) = editor.pending.prompt() else {
        return serde_json::Value::Null;
    };
    let origin = prompt.origin();
    let context = match prompt.context() {
        crate::editor::pending::PromptContext::Ex(_) => json!({"kind":"ex"}),
        crate::editor::pending::PromptContext::Pipe { range, visual, .. } => {
            json!({"kind":"pipe","range":range,"visual":visual})
        }
        crate::editor::pending::PromptContext::Search { backward, .. } => {
            json!({"kind":"search","backward":backward})
        }
    };
    json!({"text":capture_content().then(||prompt.text()),"cursor_byte":prompt.cursor(),
        "normal":prompt.normal(),"context":context,"pane_index":origin.pane_index,
        "origin":origin.pane,"revision":origin.revision})
}

impl Editor {
    pub fn trace_state(&mut self) {
        if !enabled() {
            return;
        }
        self.trace_documents
            .retain(|id, _| self.docs.get(*id).is_some());
        for (id, document) in self.docs.iter() {
            if self.trace_documents.insert(id, document.buf.trace_id())
                != Some(document.buf.trace_id())
            {
                #[derive(serde::Serialize)]
                struct DocumentRecord<'a> {
                    document: DocumentId,
                    buffer: strop_core::diagnostics::BufferTraceId,
                    #[serde(with = "strop_core::path_serde::option")]
                    path: &'a Option<std::path::PathBuf>,
                    name: &'a Option<String>,
                    revision: strop_core::id::BufferRevision,
                    bytes: usize,
                    readonly: bool,
                    text: Option<String>,
                }
                record(
                    EventKind::Document,
                    &DocumentRecord {
                        document: id,
                        buffer: document.buf.trace_id(),
                        path: &document.buf.path,
                        name: &document.buf.name,
                        revision: document.buf.revision(),
                        bytes: document.buf.len_bytes(),
                        readonly: document.buf.readonly,
                        text: capture_content().then(|| document.buf.text().to_string()),
                    },
                );
            }
        }
        let documents: Vec<_> = self.docs.iter().map(|(id, document)| json!({
            "document":id,"buffer":document.buf.trace_id(),"revision":document.buf.revision(),
            "bytes":document.buf.len_bytes(),"dirty":document.buf.dirty,"history_nodes":document.buf.history().depth(),
        })).collect();
        let cursor = self.panes.get(self.active_pane).and_then(|pane| {
            let document = self.docs.get(pane.doc)?;
            let selection = pane.sels.primary();
            Some(json!({"document":pane.doc,"head_byte":selection.head,"anchor_byte":selection.anchor,
                "line":document.buf.line_of(selection.head),"byte_column":document.buf.col_of(selection.head)}))
        });
        record(
            EventKind::State,
            &json!({
                "mode":self.mode.chip(),"pending":pending_state(self),"walker":self.walker.display(),
                "last_search":self.last_search.as_ref().map(|search|json!({
                    "pattern":capture_content().then(||search.query.source()),"backward":search.backward,
                    "whole_word":search.query.whole_word()})),
                "cursor":cursor,"documents":documents,"panes":self.panes,"active_pane":self.active_pane,
                "view_rows":self.view_rows,"message":self.message,"should_quit":self.should_quit,
                "picker":self.picker.as_ref().map(|glue|json!({"id":glue.id.0.get(),
                    "request":glue.active.as_ref().map(|ticket|ticket.request.get()),"query":glue.picker.input.text,
                    "items":glue.picker.items.len(),"streaming":glue.picker.streaming})),
                "config":self.config,
            }),
        );
    }
}
