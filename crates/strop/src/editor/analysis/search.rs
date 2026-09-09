//! Count with the shared query engine while retaining only visible hit ranges.
//! Long lines are cell-windowed here, never expanded or searched during painting.
use super::{AnalysisKey, SearchSummary};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use strop_grammar::{CompiledQuery, QueryError};

pub(super) fn summarize(
    buffer: &strop_core::Buffer,
    key: &AnalysisKey,
    query: &CompiledQuery,
    cancel: &Arc<AtomicBool>,
) -> Result<SearchSummary, QueryError> {
    let rope = buffer.text();
    let first_line = rope.byte_to_line(key.first.min(rope.len_bytes()));
    let end_line = rope
        .byte_to_line(key.last.min(rope.len_bytes()))
        .saturating_add(1)
        .min(rope.len_lines());
    let mut windows = Vec::with_capacity(end_line.saturating_sub(first_line));
    for line in first_line..end_line {
        let start = rope.line_to_byte(line);
        let text = rope.line(line);
        let mut end = text.len_bytes();
        while end > 0 && matches!(text.byte(end - 1), b'\n' | b'\r') {
            end -= 1;
        }
        let mut visible_start = None;
        let mut visible_end = 0;
        let point = buffer
            .layout_checkpoint(
                strop_core::id::LineIndex::new(line),
                strop_core::id::DisplayColumn::new(key.left),
                key.tab,
            )
            .unwrap_or_default();
        for (glyph, text) in strop_core::layout::RopeGraphemes::from_checkpoint(
            text.byte_slice(..end),
            key.tab,
            point,
        ) {
            if cancel.load(Ordering::Acquire) {
                return Err(QueryError::Cancelled);
            }
            if glyph.cell.get() >= key.right {
                break;
            }
            if glyph.cell.get().saturating_add(glyph.width) > key.left {
                visible_start.get_or_insert(start + glyph.byte);
                visible_end = start + glyph.byte + text.len();
            }
        }
        if let Some(start) = visible_start {
            windows.push(start..visible_end);
        }
    }
    let query = query.cancellable(cancel.clone());
    let mut summary = SearchSummary {
        count: 0,
        hits: Vec::new(),
    };
    strop_grammar::search_visit(buffer, &query, |hit| {
        summary.count += 1;
        let at = windows.partition_point(|window| window.end <= hit.start.get());
        if windows
            .get(at)
            .is_some_and(|window| window.start < hit.end.get())
        {
            summary.hits.push(hit);
        }
        std::ops::ControlFlow::Continue(())
    })?;
    Ok(summary)
}
