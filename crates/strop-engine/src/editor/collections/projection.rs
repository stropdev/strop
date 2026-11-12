//! Source-to-view rendering and incremental projection updates.
use super::{Collection, CollectionRow, CollectionRowInfo, Document, Editor};
use strop_core::id::{Arena, DocumentId, DocumentKind};

/// The canonical rendering (0049 §6, worded by 0097 §3.4): a title row
/// saying what was collected and how much, then ONE CARD PER FILE — top
/// border with the path and its hit count, excerpt bodies with a
/// "⋮ N lines not shown" gap between disjoint spans, and a bottom border. Row roles are recorded in `collection.rows` for the renderer.
/// Each excerpt's `view_line` is the chrome row before its body.
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
    let mut hits = 0;
    for excerpt in &collection.excerpts {
        if let Some(source) = docs.get(excerpt.source) {
            hits += super::quickfix::excerpt_hits(excerpt, &source.buf).len();
        }
    }
    let left_out = match collection.skipped {
        0 => String::new(),
        skipped => format!(" · {} left out", super::counted(skipped, "hit")),
    };
    let mut text = format!(
        "{} — {}{left_out}\n",
        collection.title,
        super::hits_in_files(hits, files)
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
        let hits: usize = collection.excerpts[at..=card_end]
            .iter()
            .map(|excerpt| super::quickfix::excerpt_hits(excerpt, &doc.buf).len())
            .sum();
        text.push_str(&format!("╭─ {path} ── {}\n", super::counted(hits, "hit")));
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
                text.push_str(&format!(
                    "⋮ {} not shown\n",
                    super::counted(gap_lines, "line")
                ));
                rows.push(CollectionRow::Gap);
                line += 1;
            }
            let start = collection.excerpts[i].start;
            let end = collection.excerpts[i].end;
            let body = doc.buf.text().byte_slice(start..end).to_string();
            let excerpt = &mut collection.excerpts[i];
            excerpt.view_line = line - 1; // the chrome row before the body
            excerpt.view_start = text.len();
            // An emptied excerpt shows no rows; an unterminated last line
            // gets the terminator its source lacks.
            excerpt.view_lines = body.lines().count();
            text.push_str(&body);
            if !body.is_empty() && !body.ends_with('\n') {
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
        self.collection_settle(id);
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
                info.source_dirty = source.buf.is_dirty();
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

    pub fn collection_unsaved(&self, document: DocumentId) -> bool {
        self.collections.get(&document).is_some_and(|collection| {
            collection.excerpts.iter().any(|excerpt| {
                self.docs
                    .get(excerpt.source)
                    .is_some_and(|source| source.buf.is_dirty())
            })
        })
    }

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
