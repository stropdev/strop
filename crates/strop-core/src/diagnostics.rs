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
#[serde(rename_all = "snake_case")]
pub(crate) enum MutationSource {
    User,
    System,
}

#[derive(Serialize)]
struct Mutation<'a> {
    buffer: BufferTraceId,
    path: Option<&'a std::path::Path>,
    source: MutationSource,
    revision: u64,
    start_byte: usize,
    removed_bytes: usize,
    inserted_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    inserted_text: Option<&'a str>,
}

impl Buffer {
    pub fn trace_id(&self) -> BufferTraceId {
        self.trace_identity
    }

    pub(crate) fn trace_edit(
        &self,
        source: MutationSource,
        start_byte: usize,
        removed_bytes: usize,
        inserted: &str,
    ) {
        if !enabled() {
            return;
        }
        // Path's serde conversion rejects non-UTF8 paths; preserve display plus
        // byte identity in the editor's document record instead of failing here.
        let path = self.path.as_deref().filter(|p| p.to_str().is_some());
        record(
            EventKind::Mutation,
            &Mutation {
                buffer: self.trace_id(),
                path,
                source,
                revision: self.epoch,
                start_byte,
                removed_bytes,
                inserted_bytes: inserted.len(),
                inserted_text: capture_content().then_some(inserted),
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
                revision: self.epoch,
                edits: edits
                    .iter()
                    .map(|edit| HistoryEdit {
                        start_byte: edit.at,
                        bytes: edit.text.len(),
                        kind: edit.kind,
                        text: capture_content().then_some(edit.text.as_str()),
                    })
                    .collect(),
            },
        );
    }
}
