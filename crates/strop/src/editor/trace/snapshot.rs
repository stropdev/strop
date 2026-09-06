//! State snapshots tolerate shutdown and retain stable document identities.
use crate::editor::Editor;
use serde_json::json;
use strop_core::id::DocumentId;
use strop_trace::{capture_content, enabled, record, EventKind};

pub(super) fn document_id(id: DocumentId) -> serde_json::Value {
    json!({"slot":id.index(), "generation":id.generation()})
}

impl Editor {
    pub(crate) fn trace_state(&mut self) {
        if !enabled() {
            return;
        }
        for (id, document) in self.docs.iter() {
            if self.trace_documents.insert(id, document.buf.trace_id())
                != Some(document.buf.trace_id())
            {
                let path = document.buf.path.as_ref();
                #[cfg(unix)]
                let path_bytes = path.map(|path| {
                    use std::os::unix::ffi::OsStrExt;
                    path.as_os_str().as_bytes().to_vec()
                });
                #[cfg(not(unix))]
                let path_bytes: Option<Vec<u8>> = None;
                record(
                    EventKind::Document,
                    &json!({
                        "document":document_id(id), "buffer":document.buf.trace_id(),
                        "path":path.map(|p|p.to_string_lossy()), "path_bytes":path_bytes,
                        "name":document.buf.name, "revision":document.buf.epoch,
                        "bytes":document.buf.len_bytes(), "readonly":document.buf.readonly,
                        "text":capture_content().then(||document.buf.rope.to_string()),
                    }),
                );
            }
        }
        let panes: Vec<_> = self
            .panes
            .iter()
            .map(|pane| {
                let selections: Vec<_> = std::iter::once(pane.sels.primary())
                .chain(pane.sels.extra_heads().iter().copied())
                .map(|selection| json!({"anchor_byte":selection.anchor,"head_byte":selection.head}))
                .collect();
                json!({"document":document_id(pane.doc), "view_top_line":pane.view_top,
                "selections":selections,"live":self.docs.get(pane.doc).is_some()})
            })
            .collect();
        let documents: Vec<_> = self
            .docs
            .iter()
            .map(|(id, doc)| {
                json!({
                    "document":document_id(id), "buffer":doc.buf.trace_id(),
                    "revision":doc.buf.epoch, "bytes":doc.buf.len_bytes(), "dirty":doc.buf.dirty,
                    "history_node":doc.buf.history.depth(),
                })
            })
            .collect();
        let current = self
            .panes
            .get(self.active_pane)
            .and_then(|pane| self.docs.get(pane.doc).map(|doc| (pane, doc)));
        let cursor = current.map(|(pane, document)| {
            let selection = pane.sels.primary();
            json!({"document":document_id(pane.doc), "head_byte":selection.head,
                "anchor_byte":selection.anchor, "line":document.buf.line_of(selection.head),
                "byte_column":document.buf.col_of(selection.head)})
        });
        record(
            EventKind::State,
            &json!({
                "mode":self.mode.chip(), "pending":self.pending,
                "pending_cursor_byte":self.pending_cursor,"pending_normal":self.pending_normal,
                "walker":self.walker.display(), "search_origin_byte":self.search_origin,
                "last_search":self.last_search.as_ref().map(|search|json!({
                    "pattern":search.pattern,"backward":search.backward,"whole_word":search.whole_word})),
                "cursor":cursor,"documents":documents,"panes":panes,
                "active_pane":self.active_pane,"view_rows":self.view_rows,
                "message":self.message,"should_quit":self.should_quit,
                "picker":self.picker.as_ref().map(|glue| json!({
                    "id":glue.id,"generation":glue.gen,"query":glue.picker.input.text,
                    "items":glue.picker.items.len(),"streaming":glue.picker.streaming})),
                "config":{"tab_size":self.config.tab_size,"indent_guides":self.config.indent_guides},
            }),
        );
        if let Some(error) = strop_trace::take_failure() {
            self.message = format!("trace incomplete: {error}");
        }
    }
}
