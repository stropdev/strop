//! Source-backed editable collections. Journal edits publish immediately;
//! generated chrome is protected and source undo groups commit at action boundaries.

mod context;
mod editing;
mod history;
mod journal;
mod navigation;
mod projection;
#[cfg(test)]
mod tests;
mod updates;
mod view_positions;

use projection::render;

use std::collections::{HashMap, HashSet};

use super::document::Document;
use super::Editor;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::Buffer;

/// What a collection view row IS (0049 §6): the renderer styles chrome
/// from this — never by parsing row text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionRow {
    Title,
    /// A file card's top border; carries the excerpt index it precedes.
    CardTop(usize),
    /// An omitted-lines gap between two excerpts of one file.
    Gap,
    /// A file card's bottom border.
    CardBottom,
    Body,
}

/// One excerpt: a whole-line span of a source document, remapped through
/// the source's change journal like any other saved anchor.
#[derive(Debug, Clone)]
pub(crate) struct Excerpt {
    pub source: DocumentId,
    /// Source byte span (whole lines), remapped on every source mutation.
    pub start: usize,
    pub end: usize,
    /// Context radius around the query's source hits.
    pub context: usize,
    /// Original hit line anchors, remapped with source edits.
    pub hit_anchors: Vec<usize>,
    /// Header line index in the view (the title is line 0).
    pub view_line: usize,
    /// Source line count as rendered.
    pub view_lines: usize,
    /// Editable source body byte span in the view, excluding synthetic newline.
    pub view_start: usize,
    pub view_end: usize,
    /// Query hit spans in SOURCE bytes within this excerpt (the picker's
    /// match evidence paints in the view, 0050 §7).
    pub matches: Vec<(usize, usize)>,
}

#[derive(Debug, Clone)]
pub(crate) struct Collection {
    pub title: String,
    pub excerpts: Vec<Excerpt>,
    pub skipped: usize,
    /// Source saves in flight from a collection `:w`/`:wq`; the view
    /// closes only when every one confirms (0049 §5).
    pub pending_saves: HashSet<DocumentId>,
    pub close_when_saved: bool,
    /// Query provenance: matches are counted independently of merged excerpts.
    pub match_count: usize,
    /// The buffer revision at last sync — the cheap no-change check that
    /// keeps motions from materializing rope text on the input path.
    pub revision: BufferRevision,
    /// Row roles parallel to the view text (0049 §6), rebuilt at render.
    pub rows: Vec<CollectionRow>,
    pub pending_commit: Vec<(DocumentId, BufferRevision)>,
}

/// A source location plus the exact search witness when this came from Search.
#[derive(Debug)]
pub(crate) struct CollectionHit {
    location: strop_workspace::ResourceLocation,
    line: usize,
    span: Option<(usize, usize)>,
    witness: Option<super::picker::ReplacementHit>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SourceHit {
    line: usize,
    span: Option<(usize, usize)>,
}

/// An in-flight collection build: hits plus the count of background
/// source loads still outstanding (0044 v2 async source loading).
#[derive(Debug)]

pub(crate) struct CollectionBuild {
    pub title: String,
    /// (path, line, match col+len in source bytes when known)
    pub hits: Vec<CollectionHit>,
    pub waiting: usize,
    pub owner: strop_core::worker::WorkerId,
    pub origin: DocumentId,
    pub revision: BufferRevision,
    pub focus_on_ready: bool,
}

impl Editor {
    fn collection_sources(&self) -> HashMap<strop_workspace::ResourceLocation, DocumentId> {
        let mut sources = HashMap::new();
        for (id, document) in self.docs.iter() {
            if matches!(document.source, super::document::DocumentSource::File) {
                for path in document
                    .buf
                    .path
                    .as_deref()
                    .into_iter()
                    .chain(document.buf.file_identity())
                {
                    sources.insert(
                        strop_workspace::ResourceLocation::local(self.cwd.join(path)),
                        id,
                    );
                }
            }
            if let Some(location) = document
                .file_target(&self.cwd)
                .and_then(|target| target.resource_location())
            {
                sources.insert(location, id);
            }
        }
        sources
    }

    /// `ctrl-o` in a result picker: open the listed hits as an editable
    /// collection. Unopened sources load in their captured filesystem namespace.
    pub(crate) fn open_collection_from_picker(&mut self) {
        let Some(glue) = &self.picker else {
            return;
        };
        if let Some(error) = &glue.picker.error {
            self.message = format!("collection refused: {error}");
            return;
        }
        if glue.picker.streaming || glue.rank_pending.is_some() {
            self.message = "results are still updating — retry Ctrl-O when ready".into();
            return;
        }
        let kind = glue.picker.kind;
        if !matches!(
            kind,
            strop_picker::Kind::Locations
                | strop_picker::Kind::Diagnostics
                | strop_picker::Kind::Search
        ) {
            self.message = "collections come from a results list".into();
            return;
        }
        let mut hits: Vec<CollectionHit> = Vec::new();
        for item in glue.picker.accepted() {
            match &item.payload {
                strop_picker::Payload::Grep {
                    location,
                    line,
                    col,
                    match_len,
                    line_text,
                } => {
                    let hit = (kind == strop_picker::Kind::Search)
                        .then(|| (col.saturating_sub(1), *match_len));
                    hits.push(CollectionHit {
                        location: location.clone(),
                        line: line.saturating_sub(1),
                        span: hit,
                        witness: hit.map(|_| super::picker::ReplacementHit {
                            line: *line,
                            col: *col,
                            match_len: *match_len,
                            text: line_text.clone(),
                        }),
                    });
                }
                strop_picker::Payload::Remote {
                    endpoint,
                    path,
                    line,
                    ..
                } => hits.push(CollectionHit {
                    location: strop_workspace::ResourceLocation::remote(
                        endpoint.clone(),
                        path.clone(),
                    ),
                    line: line.saturating_sub(1),
                    span: None,
                    witness: None,
                }),
                _ => {}
            }
        }
        if hits.is_empty() {
            self.message = "collection: no included source matches".into();
            return;
        }
        let title = kind.title().trim().to_string();
        let owner = glue.id.0;
        self.close_picker();
        // Unopened sources load in the background (never switching focus);
        // the build assembles when the last one lands.
        let mut to_load = Vec::new();
        let open_sources = self.collection_sources();
        let mut requested = std::collections::HashSet::new();
        for hit in &hits {
            if !open_sources.contains_key(&hit.location) && requested.insert(hit.location.clone()) {
                match crate::files::FileTarget::from_location(&hit.location) {
                    Ok(target) => to_load.push(target),
                    Err(error) => {
                        self.message = format!("collection refused: {error}");
                        return;
                    }
                }
            }
        }
        let waiting = to_load.len();
        let build = CollectionBuild {
            title,
            hits,
            waiting,
            owner,
            origin: self.current(),
            revision: self.buf().revision(),
            focus_on_ready: true,
        };
        if to_load.is_empty() {
            self.build_collection(build);
            return;
        }
        self.collection_build = Some(build);
        self.message = format!("collection: loading {waiting} source(s)…");
        for target in to_load {
            self.request_target(
                target,
                crate::editor::io::OpenIntent::CollectionSource { owner },
            );
        }
    }

    /// A background source load landed (or failed): the pending build
    /// counts down and assembles when its sources are all in.
    pub(crate) fn collection_source_ready(&mut self, owner: strop_core::worker::WorkerId) {
        let Some(build) = self
            .collection_build
            .as_mut()
            .filter(|build| build.owner == owner)
        else {
            strop_trace::record_with(
                strop_trace::EventKind::JobFinished,
                || serde_json::json!({"service":"collection","result":"ready-without-build"}),
            );
            return;
        };
        build.waiting = build.waiting.saturating_sub(1);
        strop_trace::record_with(
            strop_trace::EventKind::JobFinished,
            || serde_json::json!({"service":"collection","result":"source-ready","waiting":build.waiting}),
        );
        if build.waiting == 0 {
            let build = self.collection_build.take().unwrap();
            self.build_collection(build);
        }
    }

    fn build_collection(&mut self, build: CollectionBuild) {
        let take_focus = build.focus_on_ready
            && self
                .panes
                .get(self.active_pane)
                .is_some_and(|pane| pane.doc == build.origin)
            && self
                .docs
                .get(build.origin)
                .is_some_and(|document| document.buf.revision() == build.revision);
        let CollectionBuild { title, hits, .. } = build;
        let mut by_doc: HashMap<DocumentId, Vec<SourceHit>> = HashMap::new();
        let open_sources = self.collection_sources();
        let mut skipped = 0;
        for CollectionHit {
            location,
            line,
            span: hit,
            witness,
        } in hits
        {
            let Some(&document) = open_sources.get(&location) else {
                skipped += 1;
                continue;
            };
            if witness.as_ref().is_some_and(|witness| {
                super::picker::checked_hit_range(self.doc(document).buf.text(), witness).is_none()
            }) {
                skipped += 1;
                continue;
            }
            by_doc
                .entry(document)
                .or_default()
                .push(SourceHit { line, span: hit });
        }
        if by_doc.is_empty() {
            self.message = format!("collection refused: {skipped} source match(es) stale or unavailable; refresh Search");
            return;
        }
        let mut excerpts: Vec<Excerpt> = Vec::new();
        let mut match_count = 0;
        for (source, mut hits) in by_doc {
            hits.sort_unstable();
            hits.dedup();
            let buf = &self.docs.get(source).unwrap().buf;
            for SourceHit { line, span: hit } in hits {
                if line >= buf.len_lines() {
                    continue;
                }
                match_count += 1;
                let hit_start = buf.line_start(line);
                let start = buf.line_start(line.saturating_sub(2));
                let end_line = line.saturating_add(3);
                let end = if end_line >= buf.len_lines() {
                    buf.len_bytes()
                } else {
                    buf.line_start(end_line)
                };
                let span = hit.map(|(column, length)| (hit_start + column, length));
                if let Some(previous) = excerpts
                    .last_mut()
                    .filter(|e| e.source == source && e.end >= start)
                {
                    previous.end = previous.end.max(end);
                    if !previous.hit_anchors.contains(&hit_start) {
                        previous.hit_anchors.push(hit_start);
                    }
                    previous.matches.extend(span);
                } else {
                    excerpts.push(Excerpt {
                        source,
                        start,
                        end,
                        context: 2,
                        hit_anchors: vec![hit_start],
                        view_line: 0,
                        view_lines: 0,
                        view_start: 0,
                        view_end: 0,
                        matches: span.into_iter().collect(),
                    });
                }
            }
        }
        // Cards present in path order, not document-id order (0049 §6).
        excerpts.sort_by_cached_key(|excerpt| {
            let label = self
                .docs
                .get(excerpt.source)
                .map(|document| document.label(&self.cwd))
                .unwrap_or_default();
            (label, excerpt.source.index(), excerpt.start)
        });
        let excerpt_count = excerpts.len();
        let title_for_trace = title.clone();
        let id = self.docs.insert(Document::output(Buffer::from_text("")));
        // The modeline names the collection, never [scratch] (0049 §6).
        self.docs.get_mut(id).unwrap().buf.name = Some(format!("collection: {title}"));
        let mut collection = Collection {
            title,
            excerpts,
            pending_saves: HashSet::new(),
            close_when_saved: false,
            rows: Vec::new(),
            pending_commit: Vec::new(),
            match_count,
            skipped,
            revision: BufferRevision::new(0),
        };
        let text = render(&self.docs, &self.cwd, &mut collection);
        self.collections.insert(id, collection);
        let _ = self.doc_mut(id).buf.system_edit().replace_all(&text);
        self.docs.get_mut(id).unwrap().buf.readonly = false;
        let revision = self.docs.get(id).unwrap().buf.revision();
        self.collections.get_mut(&id).unwrap().revision = revision;
        strop_trace::record_with(strop_trace::EventKind::JobFinished, || {
            serde_json::json!({
                "service":"collection","result":"built","excerpts":excerpt_count,
                "skipped":skipped,"title":title_for_trace,
            })
        });
        if take_focus {
            self.drop_stale_scratch(id);
            self.switch_to(id);
            self.set_head(0);
        }
        self.message = match skipped {
            0 => format!("collection: {excerpt_count} excerpt(s)"),
            _ => format!("collection built; {skipped} hit(s) skipped (not open local buffers)"),
        };
        if !take_focus {
            self.message.push_str(" — ready in Space b");
        }
    }
}

/// What the renderer needs per collection row (0049 §6).
pub struct CollectionRowInfo {
    pub kind: CollectionRow,
    /// Body rows: (source document, source byte start, source byte end).
    pub source: Option<(strop_core::id::DocumentId, usize, usize)>,
    /// Query hit spans within the row, in SOURCE bytes (0050 §7).
    pub source_matches: Vec<(usize, usize)>,
    /// The caret sits inside this row's card (focus chrome).
    pub card_active: bool,
    pub source_dirty: bool,
    pub source_readonly: bool,
}
