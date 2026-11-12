//! Collecting (0097 §3.1): `Ctrl-O` in a results list becomes a collection
//! with the cursor on the hit that was under the bar.
use std::collections::HashMap;

use super::projection::render;
use super::{
    hits_in_files, Collection, CollectionBuild, CollectionHit, Document, Editor, Excerpt, SourceHit,
};
use crate::editor::picker::Deferred;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::Buffer;

/// The longest query a collection's name spells out.
const NAMED_QUERY: usize = 48;

/// What a collection is called: the list it came from and, for a search,
/// what was searched (`search needle_`, `locations`).
fn label(kind: strop_picker::Kind, query: &str) -> String {
    let kind = kind.title().trim();
    let query = query.trim();
    if kind != "search" || query.is_empty() {
        return kind.to_string();
    }
    match query.char_indices().nth(NAMED_QUERY) {
        Some((cut, _)) => format!("{kind} {}…", &query[..cut]),
        None => format!("{kind} {query}"),
    }
}

impl Editor {
    pub(super) fn collection_sources(
        &self,
    ) -> HashMap<strop_workspace::ResourceLocation, DocumentId> {
        let mut sources = HashMap::new();
        for (id, document) in self.docs.iter() {
            if matches!(
                document.source,
                crate::editor::document::DocumentSource::File
            ) {
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

    /// `ctrl-o` in a result picker: open the included hits as a collection.
    /// A list that is still being worked out is collected when it is ready.
    /// Unopened sources load in their captured filesystem namespace.
    pub(crate) fn open_collection_from_picker(&mut self) {
        let Some(glue) = self.picker.as_mut() else {
            return;
        };
        glue.deferred = None;
        if let Some(error) = &glue.picker.error {
            self.message = format!("nothing collected — {error}");
            return;
        }
        let kind = glue.picker.kind;
        if !matches!(
            kind,
            strop_picker::Kind::Locations
                | strop_picker::Kind::Diagnostics
                | strop_picker::Kind::Search
        ) {
            self.message = "Ctrl-O collects search results, references or diagnostics".into();
            return;
        }
        if glue.picker.streaming || glue.rank_pending.is_some() {
            glue.deferred = Some(Deferred::Collect);
            self.message = "collecting when the list is complete…".into();
            return;
        }
        let current = glue.picker.current();
        let mut picked = None;
        let mut hits: Vec<CollectionHit> = Vec::new();
        for item in glue.picker.accepted() {
            let hit = match &item.payload {
                strop_picker::Payload::Grep {
                    location,
                    line,
                    col,
                    match_len,
                    line_text,
                } => {
                    let span = (kind == strop_picker::Kind::Search)
                        .then(|| (col.saturating_sub(1), *match_len));
                    CollectionHit {
                        location: location.clone(),
                        line: line.saturating_sub(1),
                        span,
                        witness: span.map(|_| crate::editor::picker::ReplacementHit {
                            line: *line,
                            col: *col,
                            match_len: *match_len,
                            text: line_text.clone(),
                        }),
                    }
                }
                strop_picker::Payload::Remote {
                    endpoint,
                    path,
                    line,
                    ..
                } => CollectionHit {
                    location: strop_workspace::ResourceLocation::remote(
                        endpoint.clone(),
                        path.clone(),
                    ),
                    line: line.saturating_sub(1),
                    span: None,
                    witness: None,
                },
                _ => continue,
            };
            if current.is_some_and(|current| std::ptr::eq(current, item)) {
                picked = Some((
                    hit.location.clone(),
                    hit.line,
                    hit.span.map(|(column, _)| column),
                ));
            }
            hits.push(hit);
        }
        let listed = glue.picker.items.len();
        if hits.is_empty() {
            self.message = if listed == 0 {
                "nothing to collect — the list is empty".into()
            } else {
                "nothing to collect — every hit is excluded".into()
            };
            return;
        }
        let title = label(kind, glue.picker.input.text());
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
                        self.message = format!("nothing collected — {error}");
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
            picked,
        };
        if to_load.is_empty() {
            self.build_collection(build);
            return;
        }
        let abandoned = self.collection_build.replace(build).is_some();
        if abandoned {
            // What the abandoned build loaded and this one does not use.
            crate::editor::collections::loans::release_unheld(self);
        }
        self.message = format!("collecting: opening {}…", super::counted(waiting, "file"));
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

    /// Assemble a build, then let go of whatever it (or a build it
    /// replaced) loaded and no longer uses (0101 §6.1).
    fn build_collection(&mut self, build: CollectionBuild) {
        self.assemble_collection(build);
        crate::editor::collections::loans::release_unheld(self);
    }

    fn assemble_collection(&mut self, build: CollectionBuild) {
        let take_focus = build.focus_on_ready
            && self
                .panes
                .get(self.active_pane)
                .is_some_and(|pane| pane.doc == build.origin)
            && self
                .docs
                .get(build.origin)
                .is_some_and(|document| document.buf.revision() == build.revision);
        let CollectionBuild {
            title,
            hits,
            picked,
            ..
        } = build;
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
                crate::editor::picker::checked_hit_range(self.doc(document).buf.text(), witness)
                    .is_none()
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
            self.message = format!(
                "nothing collected — {} changed since the search or could not be opened; search again",
                super::counted(skipped, "hit")
            );
            return;
        }
        let files = by_doc.len();
        let mut excerpts: Vec<Excerpt> = Vec::new();
        for (source, mut hits) in by_doc {
            hits.sort_unstable();
            hits.dedup();
            let buf = &self.docs.get(source).unwrap().buf;
            for SourceHit { line, span: hit } in hits {
                if line >= buf.len_lines() {
                    continue;
                }
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
        let name = format!("collection: {title}");
        // Collecting the same thing again refreshes its collection (0097
        // §3.1): same buffer, new hits, history started over.
        let existing = self.collections.keys().copied().find(|id| {
            self.docs
                .get(*id)
                .is_some_and(|document| document.buf.name.as_deref() == Some(name.as_str()))
        });
        let id = match existing {
            Some(id) => id,
            None => {
                let mut document = Document::output(Buffer::from_text(""));
                // Writable: `write.rs` admits each edit against the excerpts.
                document.buf.clear_readonly();
                // The modeline names the collection, never [scratch] (0049 §6).
                document.buf.name = Some(name);
                let Ok(id) = self.docs.try_insert(document) else {
                    self.message = "document identity space exhausted".into();
                    return;
                };
                id
            }
        };
        let mut collection = Collection {
            title: title.clone(),
            excerpts,
            rows: Vec::new(),
            skipped,
            revision: BufferRevision::new(0),
            mapped: BufferRevision::new(0),
            rerender: false,
            recheck: false,
            voided: false,
            touched: false,
            history_move: None,
            cursor: None,
        };
        let text = render(&self.docs, &self.cwd, &mut collection);
        self.collections.insert(id, collection);
        let _ = self.doc_mut(id).buf.system_edit().replace_all(&text);
        let Some(revision) = self.docs.get(id).map(|document| document.buf.revision()) else {
            return; // the id was minted above; a miss is impossible
        };
        if let Some(collection) = self.collections.get_mut(&id) {
            collection.revision = revision;
            collection.mapped = revision;
        }
        // A refreshed collection's old carets address text that is gone.
        for (index, pane) in self.panes.iter_mut().enumerate() {
            if pane.doc == id && index != self.active_pane {
                pane.sels = Default::default();
                pane.view_top = 0;
            }
        }
        self.changes.quickfix = Some(id);
        strop_trace::record_with(strop_trace::EventKind::JobFinished, || {
            serde_json::json!({
                "service":"collection","result":"built","excerpts":excerpt_count,
                "skipped":skipped,"title":title,
            })
        });
        // The cursor lands on the hit that was under the bar, else the first.
        let hits = self.collection_hits(id);
        let landing = picked
            .and_then(|(location, line, column)| {
                let source = *open_sources.get(&location)?;
                let buffer = &self.docs.get(source)?.buf;
                let line_start = buffer.line_start(line.min(buffer.last_content_line()));
                let at = line_start + column.unwrap_or(0);
                hits.iter()
                    .position(|hit| hit.source == source && hit.at == at)
                    .or_else(|| {
                        hits.iter().position(|hit| {
                            hit.source == source
                                && buffer.line_start(buffer.line_of(hit.at)) == line_start
                        })
                    })
            })
            .unwrap_or(0);
        if let Some(collection) = self.collections.get_mut(&id) {
            collection.cursor = Some(landing);
        }
        if take_focus {
            if self.current() != id {
                // `:q` hands the cursor back to where `Ctrl-O` was pressed.
                let origin = self.jump_record();
                if let Some(document) = self.docs.get_mut(id) {
                    document.set_return_point(origin);
                }
            }
            self.drop_stale_scratch(id);
            self.push_jump();
            self.switch_to(id);
            self.set_head(hits.get(landing).map_or(0, |hit| hit.view));
            self.clamp_cursor();
            self.place_jump_target();
        }
        self.message = hits_in_files(hits.len(), files);
        if skipped > 0 {
            self.message.push_str(&format!(
                "; {} left out (changed since the search, or could not be opened)",
                super::counted(skipped, "hit")
            ));
        }
        self.message.push_str(if take_focus {
            " — ]q next, Enter opens the file, :w writes"
        } else {
            " collected — :copen shows them"
        });
    }
}
