//! Source-to-view rendering and incremental projection updates.
use super::{Collection, CollectionRow, CollectionRowInfo, Document, Editor};
use strop_core::id::{Arena, DocumentId, DocumentKind};
use strop_core::Range;

/// The canonical rendering (0049 §6): a title row with counts, then ONE
/// CARD PER FILE — top border with path and badges, excerpt bodies with
/// a "⋮ N source lines omitted" gap between disjoint spans, and a bottom
/// border. Row roles are recorded in `collection.rows` for the renderer;
/// chrome rows are protected (write-back refuses them) and keep the
/// excerpt's `view_line` invariant: the row before the body.
pub(super) fn render(
    docs: &Arena<DocumentKind, Document>,
    cwd: &std::path::Path,
    collection: &mut Collection,
) -> String {
    let files = collection
        .excerpts
        .iter()
        .enumerate()
        .filter(|(index, excerpt)| {
            *index == 0 || collection.excerpts[index - 1].source != excerpt.source
        })
        .count();
    let unavailable = if collection.skipped == 0 {
        String::new()
    } else {
        format!(" · {} unavailable hit(s)", collection.skipped)
    };
    let mut text = format!(
        "collection: {} — {} match(es) · {} file(s){unavailable}\n",
        collection.title, collection.match_count, files
    );
    let mut rows = vec![CollectionRow::Title];
    let mut line = 1;
    let mut at = 0;
    while at < collection.excerpts.len() {
        let source = collection.excerpts[at].source;
        let mut card_end = at;
        while card_end + 1 < collection.excerpts.len()
            && collection.excerpts[card_end + 1].source == source
        {
            card_end += 1;
        }
        let doc = docs.get(source).unwrap();
        let path = doc.label(cwd);
        let lang = doc
            .syntax_path()
            .and_then(|path| path.extension())
            .and_then(|extension| extension.to_str())
            .and_then(strop_lsp::registry::language_for_extension_name);
        let count = card_end - at + 1;
        let lang_badge = lang.map(|l| format!("{l} · ")).unwrap_or_default();
        let context = collection.excerpts[at..=card_end]
            .iter()
            .map(|excerpt| excerpt.context)
            .max()
            .unwrap_or(0);
        text.push_str(&format!(
            "╭─ {path} ── {lang_badge}{count} excerpt(s) · context {context} (+/-)\n"
        ));
        rows.push(CollectionRow::CardTop(at));
        line += 1;
        for i in at..=card_end {
            if i > at {
                let prev = &collection.excerpts[i - 1];
                let here = &collection.excerpts[i];
                let gap_lines = doc
                    .buf
                    .line_of(here.start)
                    .saturating_sub(doc.buf.line_of(prev.end));
                text.push_str(&format!("⋮ {gap_lines} source lines omitted\n"));
                rows.push(CollectionRow::Gap);
                line += 1;
            }
            let start = collection.excerpts[i].start;
            let end = collection.excerpts[i].end;
            let body = doc.buf.text().byte_slice(start..end).to_string();
            let excerpt = &mut collection.excerpts[i];
            excerpt.view_line = line - 1; // the chrome row before the body
            excerpt.view_start = text.len();
            excerpt.view_lines = body.lines().count().max(1);
            text.push_str(&body);
            if !body.ends_with('\n') {
                text.push('\n');
            }
            excerpt.view_end = text.len();
            for _ in 0..excerpt.view_lines {
                rows.push(CollectionRow::Body);
            }
            line += excerpt.view_lines;
        }
        text.push_str("╰\n");
        rows.push(CollectionRow::CardBottom);
        line += 1;
        at = card_end + 1;
    }
    collection.rows = rows;
    text
}

impl Editor {
    /// Re-render the collection view from its sources and reset shadow +
    /// revision together (0049 §5: no stale text may be presented).
    pub(crate) fn collection_render_view(&mut self, id: DocumentId) {
        // Preserve the logical caret across the regeneration (0049 §5):
        // same row/column clamped into the new text.
        let caret = if self.current() == id {
            Some((
                self.buf().line_of(self.head()),
                self.buf().col_of(self.head()),
            ))
        } else {
            None
        };
        let text = {
            let entry = self.collections.get_mut(&id).unwrap();
            render(&self.docs, &self.cwd, entry)
        };
        let _ = self.doc_mut(id).buf.system_edit().replace_all(&text);
        if let Some((line, col)) = caret {
            let line = line.min(self.docs.get(id).unwrap().buf.len_lines().saturating_sub(1));
            let head = self.docs.get(id).unwrap().buf.clamp_boundary(
                self.docs
                    .get(id)
                    .unwrap()
                    .buf
                    .line_start(line)
                    .saturating_add(col),
            );
            if self.current() == id {
                self.set_head(head);
                self.clamp_cursor();
            }
        }
        let revision = self.docs.get(id).unwrap().buf.revision();
        self.collections.get_mut(&id).unwrap().revision = revision;
        // The view is a presentation: its dirty bit is never the story
        // (0049 §5 — the sources own unsaved state).
        self.docs.get_mut(id).unwrap().buf.dirty = false;
    }

    /// A source change strictly inside one excerpt (0049 §5): splice the
    /// excerpt's view span with the new source body instead of
    /// re-rendering the whole view. The caller gates on a clean view
    /// (no unsynced user edit) — the spans assume shadow == view.
    pub(crate) fn collection_splice_excerpt(&mut self, id: DocumentId, index: usize) {
        let (source, view_start, view_end, view_lines) = {
            let entry = &self.collections[&id];
            let excerpt = &entry.excerpts[index];
            (
                excerpt.source,
                excerpt.view_start,
                excerpt.view_end,
                excerpt.view_lines,
            )
        };
        let Some(source_doc) = self.docs.get(source) else {
            return;
        };
        let excerpt_span = {
            let entry = &self.collections[&id];
            let excerpt = &entry.excerpts[index];
            (excerpt.start, excerpt.end)
        };
        let mut body = source_doc
            .buf
            .text()
            .byte_slice(excerpt_span.0..excerpt_span.1)
            .to_string();
        if !body.ends_with('\n') {
            body.push('\n');
        }
        let new_lines = body.lines().count().max(1);
        // Splice the collection buffer at the excerpt's view span; the
        // view is clean, so the spans index it directly.
        {
            let doc = self.docs.get_mut(id).unwrap();
            let _ = doc
                .buf
                .system_edit()
                .replace(Range::charwise(view_start, view_end), &body);
        }
        // The splice's own journal entry feeds the view analysis and is
        // then consumed: the write-back diff must never see it.
        {
            let doc = self.docs.get_mut(id).unwrap();
            let changes: Vec<_> = doc.buf.changes().to_vec();
            self.analysis.edits(id, &changes);
            doc.buf.clear_changes();
        }
        let byte_delta = body.len() as isize - (view_end - view_start) as isize;
        let line_delta = new_lines as isize - view_lines as isize;
        let entry = self.collections.get_mut(&id).unwrap();
        let first_row = entry.excerpts[index].view_line + 1;
        entry.rows.splice(
            first_row..first_row + view_lines,
            std::iter::repeat_n(CollectionRow::Body, new_lines),
        );
        let mut seen = false;
        for excerpt in &mut entry.excerpts {
            if seen {
                excerpt.view_start = (excerpt.view_start as isize + byte_delta) as usize;
                excerpt.view_end = (excerpt.view_end as isize + byte_delta) as usize;
                excerpt.view_line = (excerpt.view_line as isize + line_delta) as usize;
            } else if excerpt.view_start == view_start {
                seen = true;
                excerpt.view_lines = new_lines;
                excerpt.view_end = view_start + body.len();
            }
        }
        let revision = self.docs.get(id).unwrap().buf.revision();
        self.collections.get_mut(&id).unwrap().revision = revision;
    }
}

impl Editor {
    /// Per-row source facts for the renderer (0049 §6): the row's role,
    /// and for body rows the source document + this row's source byte
    /// span + the query hits inside it (source bytes) + whether the
    /// caret sits in this card.
    pub fn collection_row_info(
        &self,
        doc: strop_core::id::DocumentId,
        line: usize,
    ) -> Option<CollectionRowInfo> {
        let collection = self.collections.get(&doc)?;
        let kind = *collection.rows.get(line)?;
        let caret_line = if doc == self.current() {
            self.buf().line_of(self.head())
        } else {
            usize::MAX
        };
        let mut info = CollectionRowInfo {
            kind,
            source: None,
            source_matches: Vec::new(),
            card_active: false,
            source_dirty: false,
            source_readonly: false,
        };
        let index = collection
            .excerpts
            .partition_point(|excerpt| excerpt.view_line <= line);
        if let Some(excerpt) = index
            .checked_sub(1)
            .and_then(|index| collection.excerpts.get(index))
        {
            if let Some(source) = self.docs.get(excerpt.source) {
                info.source_dirty = source.buf.dirty;
                info.source_readonly = source.buf.readonly;
            }
            let active_index = collection
                .excerpts
                .partition_point(|excerpt| excerpt.view_line <= caret_line);
            info.card_active = caret_line != usize::MAX
                && active_index
                    .checked_sub(1)
                    .and_then(|index| collection.excerpts.get(index))
                    .is_some_and(|active| active.source == excerpt.source);
            if kind == CollectionRow::Body
                && line > excerpt.view_line
                && line <= excerpt.view_line + excerpt.view_lines
            {
                let source = self.docs.get(excerpt.source)?;
                let source_line = source.buf.line_of(excerpt.start) + line - excerpt.view_line - 1;
                let start = source.buf.line_start(source_line);
                let end = source.buf.line_end(source_line);
                info.source = Some((excerpt.source, start, end));
                let first = excerpt
                    .matches
                    .partition_point(|(offset, length)| offset.saturating_add(*length) <= start);
                info.source_matches = excerpt.matches[first..]
                    .iter()
                    .copied()
                    .take_while(|(offset, _)| *offset < end)
                    .map(|(offset, length)| {
                        (
                            offset.max(start),
                            offset.saturating_add(length).min(end) - offset.max(start),
                        )
                    })
                    .collect();
            }
        }
        Some(info)
    }

    /// A collection view row's role (0049 §6) for the renderer —
    /// structure as data, never text parsing.
    pub fn collection_row_kind(
        &self,
        doc: strop_core::id::DocumentId,
        line: usize,
    ) -> Option<CollectionRow> {
        self.collections.get(&doc)?.rows.get(line).copied()
    }

    /// The SOURCE line number for a collection view row (0049 §6):
    /// Some(Some(n)) for body rows, Some(None) for chrome (title,
    /// headers — the gutter stays blank there), None for ordinary
    /// buffers.
    pub fn collection_source_lineno(
        &self,
        doc: strop_core::id::DocumentId,
        line: usize,
    ) -> Option<Option<usize>> {
        let collection = self.collections.get(&doc)?;
        let index = collection
            .excerpts
            .partition_point(|excerpt| excerpt.view_line < line);
        if let Some(excerpt) = index
            .checked_sub(1)
            .and_then(|index| collection.excerpts.get(index))
        {
            if line <= excerpt.view_line + excerpt.view_lines {
                let source = self.docs.get(excerpt.source)?;
                return Some(Some(
                    source.buf.line_of(excerpt.start) + line - excerpt.view_line,
                ));
            }
        }
        Some(None) // title row
    }
}

impl Editor {
    pub fn collection_unsaved(&self, document: DocumentId) -> bool {
        self.collections.get(&document).is_some_and(|collection| {
            collection.excerpts.iter().any(|excerpt| {
                self.docs
                    .get(excerpt.source)
                    .is_some_and(|source| source.buf.dirty)
            })
        })
    }
}

impl Editor {
    /// Map a real buffer byte to its authoritative source; chrome has no source byte.
    pub fn source_position(
        &self,
        document: DocumentId,
        byte: usize,
    ) -> Option<(DocumentId, usize)> {
        let view = self.docs.get(document)?;
        let Some(collection) = self.collections.get(&document) else {
            return Some((
                document,
                view.buf.clamp_boundary(byte.min(view.buf.len_bytes())),
            ));
        };
        let after = collection
            .excerpts
            .partition_point(|excerpt| excerpt.view_start <= byte);
        let excerpt = collection.excerpts.get(after.checked_sub(1)?)?;
        if byte >= excerpt.view_end {
            return None;
        }
        let source = self.docs.get(excerpt.source)?;
        let offset = (excerpt.start + byte - excerpt.view_start).min(excerpt.end);
        Some((
            excerpt.source,
            source
                .buf
                .clamp_boundary(offset.min(source.buf.len_bytes())),
        ))
    }
}
