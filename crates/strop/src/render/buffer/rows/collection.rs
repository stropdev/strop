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

/// The collection's adapter (0100 §4.1): a typed row in, a painted block
/// row out. The title reads its subject strong; a heading is the ruled
/// heading with the file's marks and hit count; a gap is a break; a body
/// row carries the source file's line number. The cursor row is the
/// cursor line, as in a file.
#[allow(clippy::too_many_arguments)]
pub(super) fn painted(
    editor: &Editor,
    t: &Tokens,
    view: &PaneView,
    info: &crate::editor::CollectionRowInfo,
    line: usize,
    text: ropey::RopeSlice<'_>,
    sign: Span<'static>,
    (gutter_cells, text_cells): (usize, usize),
) -> crate::render::block::Painted {
    use crate::editor::CollectionRow;
    use crate::render::block::{self, gutter, heading, Painted};
    let cursor = view.overlays && editor.doc(view.doc).buf.line_of(view.cursor) == line;
    let mut painted = match info.kind {
        CollectionRow::Title => {
            let text = text.to_string();
            let subject = editor
                .collection_title(view.doc)
                .filter(|subject| text.starts_with(subject))
                .map_or(0, str::len);
            let (subject, rest) = text.split_at(subject);
            Painted::title(
                gutter_cells,
                vec![
                    Span::styled(
                        subject.to_owned(),
                        Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(rest.to_owned(), Style::default().fg(t.secondary)),
                ],
                text_cells,
            )
        }
        CollectionRow::Blank => Painted::blank(gutter_cells),
        CollectionRow::Heading(_) => {
            let label = crate::render::file_name::Label::parse(&text.to_string());
            let hits = match info.hits {
                1 => "1 hit".to_owned(),
                hits => format!("{hits} hits"),
            };
            let heading = heading::Heading {
                gutter: gutter_cells,
                ruled: true,
                name: heading::Name::Path(&label),
                marks: heading::marks(t, info.source_dirty, info.source_readonly),
                fact: heading::fact(t, hits),
                focus: if info.block_active {
                    heading::Focus::Here
                } else {
                    heading::Focus::Elsewhere
                },
            };
            Painted::heading(t, &heading, text_cells)
        }
        CollectionRow::Gap => Painted::band(
            t,
            gutter::jump(t, gutter_cells, &gutter::source_jump(gutter_cells)),
            vec![block::muted(t, text.to_string())],
            text_cells,
        ),
        CollectionRow::Body => Painted {
            gutter: gutter::numbered(
                t,
                sign,
                editor.collection_source_lineno(view.doc, line).flatten(),
                cursor,
                gutter_cells,
            ),
            text: Vec::new(),
            row_bg: None,
            fixed: false,
        },
    };
    if cursor {
        painted.row_bg = Some(t.cursorline);
    }
    painted
}
