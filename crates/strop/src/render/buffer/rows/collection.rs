//! Collection views (0049 §6): every visible excerpt's source window is
//! analysed once per frame and its syntax spans and query hits are
//! translated into view coordinates before the row loop runs, which
//! must not re-admit jobs. Per-row requests used to supersede one
//! another, leaving mixed-language collections uncolored.
use super::*;

/// One frame's source projection for the visible rows of a pane.
pub(super) struct CollectionProjection {
    /// Per visible row: the collection row info, `None` off a collection.
    pub(super) rows: Vec<Option<crate::editor::CollectionRowInfo>>,
    /// Per visible row: source syntax spans in view bytes.
    pub(super) spans: Vec<Option<Vec<strop_syntax::Span>>>,
    /// Per visible row: source query hits in view bytes.
    pub(super) hits: Vec<Vec<strop_grammar::SearchMatch>>,
}

impl CollectionProjection {
    pub(super) fn gather(editor: &Editor, view: &PaneView, count: usize, width: usize) -> Self {
        // Aggregate each visible source window once. Per-row requests used to
        // supersede one another, leaving mixed-language collections uncolored.
        let rows: Vec<Option<crate::editor::CollectionRowInfo>> = (0..count)
            .map(|row| editor.collection_row_info(view.doc, view.view_top.saturating_add(row)))
            .collect();
        let mut windows: Vec<(DocumentId, usize, usize)> = Vec::new();
        for info in rows.iter().flatten() {
            if let Some((source, start, end)) = info.source {
                if let Some((_, first, last)) = windows.iter_mut().find(|(doc, ..)| *doc == source)
                {
                    *first = (*first).min(start);
                    *last = (*last).max(end);
                } else {
                    windows.push((source, start, end));
                }
            }
        }
        let source_analyses: Vec<_> = windows
            .into_iter()
            .map(|(source, first, last)| {
                (
                    source,
                    editor.document_analysis_cached(source, first, last, view.hscroll.get(), width),
                )
            })
            .collect();
        // Translate into view coordinates ONCE per frame (borrowed by the
        // row loop, which must not re-admit jobs).
        let view_buf = &editor.doc(view.doc).buf;
        let spans: Vec<Option<Vec<strop_syntax::Span>>> = rows
            .iter()
            .enumerate()
            .map(|(row, info)| {
                let info = info.as_ref()?;
                let (source, src_start, src_end) = info.source?;
                let analysis = source_analyses
                    .iter()
                    .find(|(doc, _)| *doc == source)?
                    .1
                    .as_ref()?;
                let row_start = view_buf.line_start(view.view_top.saturating_add(row));
                Some(
                    analysis
                        .spans
                        .iter()
                        .filter(|span| span.start < src_end && span.end > src_start)
                        .map(|span| strop_syntax::Span {
                            start: span.start.max(src_start) - src_start + row_start,
                            end: span.end.min(src_end) - src_start + row_start,
                            ..*span
                        })
                        .collect(),
                )
            })
            .collect();
        let hits: Vec<Vec<strop_grammar::SearchMatch>> = rows
            .iter()
            .enumerate()
            .map(|(row, info)| {
                let Some(info) = info else {
                    return Vec::new();
                };
                let Some((_, src_start, _)) = info.source else {
                    return Vec::new();
                };
                let row_start = view_buf.line_start(view.view_top.saturating_add(row));
                info.source_matches
                    .iter()
                    .map(|(m, len)| strop_grammar::SearchMatch {
                        start: strop_core::id::ByteOffset::new(row_start + (m - src_start)),
                        end: strop_core::id::ByteOffset::new(row_start + (m - src_start) + len),
                    })
                    .collect()
            })
            .collect();
        Self { rows, spans, hits }
    }
}
