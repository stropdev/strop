//! Mutation diagnostics at the mechanics boundary, including open undo groups.
use crate::Buffer;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use strop_trace::{capture_content, enabled, record, EventKind};

static NEXT_BUFFER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BufferTraceId(u64);
impl BufferTraceId {
    pub(crate) fn next() -> Self {
        Self(NEXT_BUFFER.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Serialize)]
struct Mutation {
    buffer: BufferTraceId,
    source: crate::ChangeOrigin,
    revision: u64,
    start_byte: usize,
    removed_bytes: usize,
    inserted_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    inserted_text: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    text_truncated: bool,
}

impl Buffer {
    pub fn trace_id(&self) -> BufferTraceId {
        self.trace_identity
    }

    pub(crate) fn trace_edit(
        &self,
        source: crate::ChangeOrigin,
        start_byte: usize,
        removed_bytes: usize,
        inserted: &str,
    ) {
        if !enabled() {
            return;
        }
        // True size always recorded; the text itself rides as a bounded
        // excerpt so no edit can outgrow the per-record cap.
        let (text, text_truncated) = strop_trace::excerpt(inserted);
        record(
            EventKind::Mutation,
            &Mutation {
                buffer: self.trace_id(),
                source,
                revision: self.revision().get(),
                start_byte,
                removed_bytes,
                inserted_bytes: inserted.len(),
                inserted_text: capture_content().then(|| text.to_owned()),
                text_truncated: text_truncated && capture_content(),
            },
        );
    }

    pub(crate) fn trace_snapshot(&self, removed_bytes: usize) {
        if !enabled() {
            return;
        }
        let (text, text_truncated) = capture_content()
            .then(|| self.text_excerpt())
            .map_or((None, false), |(text, truncated)| (Some(text), truncated));
        record(
            EventKind::Mutation,
            &Mutation {
                buffer: self.trace_id(),
                source: crate::ChangeOrigin::System,
                revision: self.revision().get(),
                start_byte: 0,
                removed_bytes,
                inserted_bytes: self.len_bytes(),
                inserted_text: text,
                text_truncated,
            },
        );
    }

    pub(crate) fn trace_history(&self, edits: &[crate::history::Edit]) {
        if !enabled() {
            return;
        }
        #[derive(Serialize)]
        struct HistoryEdit<'a> {
            start_byte: usize,
            bytes: usize,
            kind: crate::history::EditKind,
            #[serde(skip_serializing_if = "Option::is_none")]
            text: Option<&'a str>,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            text_truncated: bool,
        }
        #[derive(Serialize)]
        struct AppliedHistory<'a> {
            buffer: BufferTraceId,
            revision: u64,
            edits: Vec<HistoryEdit<'a>>,
        }
        record(
            EventKind::History,
            &AppliedHistory {
                buffer: self.trace_id(),
                revision: self.revision().get(),
                edits: edits
                    .iter()
                    .map(|edit| {
                        let (text, text_truncated) = strop_trace::excerpt(&edit.text);
                        HistoryEdit {
                            start_byte: edit.at,
                            bytes: edit.text.len(),
                            kind: edit.kind,
                            text: capture_content().then_some(text),
                            text_truncated: text_truncated && capture_content(),
                        }
                    })
                    .collect(),
            },
        );
    }
}
