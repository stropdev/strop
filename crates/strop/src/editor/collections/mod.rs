//! Editable code collections (0044): picker results as one real buffer of
//! source excerpts. Edits to an excerpt write back to its source document
//! through the change-plan gateway at action boundaries; generated headers
//! are protected, and a source that moved on refuses by name.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use strop_core::id::{Arena, BufferRevision, DocumentId, DocumentKind};
use strop_core::{Buffer, Range};
use strop_workspace::ResourceLocation;

use super::changes::{ChangePlan, ChangeProducer, PlannedDocument};
use super::document::Document;
use super::Editor;

/// One excerpt: a whole-line span of a source document, remapped through
/// the source's change journal like any other saved anchor.
#[derive(Debug, Clone)]
pub(crate) struct Excerpt {
    pub source: DocumentId,
    /// Source byte span (whole lines), remapped on every source mutation.
    pub start: usize,
    pub end: usize,
    /// FNV-1a of the span's bytes at build/regeneration — the write-back
    /// staleness check.
    pub fingerprint: u64,
    /// Header line index in the shadow text (the title is line 0).
    pub view_line: usize,
    /// Source line count as rendered into the shadow.
    pub view_lines: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct Collection {
    pub title: String,
    pub excerpts: Vec<Excerpt>,
    /// The canonical rendering as of the last sync. The sync diff is
    /// shadow vs current — no hidden state.
    pub shadow: String,
    /// The buffer revision at last sync — the cheap no-change check that
    /// keeps motions from materializing rope text on the input path.
    pub revision: BufferRevision,
}

fn fingerprint(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    hash
}

/// The canonical rendering: title, then per excerpt a header line and its
/// source text. Re-anchors each excerpt's view span as it is emitted. An
/// associated function over disjoint fields, so the map entry can be
/// re-anchored while the editor borrows `docs` and `cwd`.
fn render(
    docs: &Arena<DocumentKind, Document>,
    cwd: &std::path::Path,
    collection: &mut Collection,
) -> String {
    let mut text = format!(
        "collection: {} — {} excerpt(s) (edits write back at action boundaries; q closes)\n",
        collection.title,
        collection.excerpts.len()
    );
    let mut line = 1;
    for excerpt in &mut collection.excerpts {
        let source = &docs.get(excerpt.source).unwrap().buf;
        let path = source
            .path
            .as_ref()
            .map(|path| path.strip_prefix(cwd).unwrap_or(path).display().to_string())
            .unwrap_or_else(|| "[scratch]".into());
        let first = source.line_of(excerpt.start) + 1;
        text.push_str(&format!("── {path}:{first} ──\n"));
        let body = source
            .text()
            .byte_slice(excerpt.start..excerpt.end)
            .to_string();
        excerpt.view_line = line;
        excerpt.view_lines = body.lines().count().max(1);
        excerpt.fingerprint = fingerprint(&body);
        text.push_str(&body);
        if !body.ends_with('\n') {
            text.push('\n');
        }
        line += 1 + excerpt.view_lines;
    }
    text
}

impl Editor {
    /// `ctrl-o` in a result picker: open the listed hits as an editable
    /// collection. Hits whose files are not open local documents are
    /// counted and skipped with a message; remote hits carry no local
    /// payload, so they are skipped the same way.
    pub(crate) fn open_collection_from_picker(&mut self) {
        let Some(glue) = &self.picker else {
            return;
        };
        let kind = glue.picker.kind;
        if !matches!(
            kind,
            strop_picker::Kind::Locations
                | strop_picker::Kind::Diagnostics
                | strop_picker::Kind::Grep
        ) {
            self.message = "collections come from a results list".into();
            return;
        }
        let items: Vec<(std::path::PathBuf, usize)> = glue
            .picker
            .items
            .iter()
            .filter_map(|item| match &item.payload {
                strop_picker::Payload::Grep { path, line, .. } => {
                    Some((path.clone(), line.saturating_sub(1)))
                }
                _ => None,
            })
            .collect();
        let title = kind.title().trim().to_string();
        self.close_picker();
        let mut by_doc: HashMap<DocumentId, Vec<usize>> = HashMap::new();
        let mut skipped = 0;
        for (path, line) in items {
            let absolute = if path.is_absolute() {
                path
            } else {
                self.cwd.join(path)
            };
            let Some(document) = self
                .docs
                .iter()
                .find_map(|(id, doc)| (doc.buf.path.as_ref() == Some(&absolute)).then_some(id))
            else {
                skipped += 1;
                continue;
            };
            by_doc.entry(document).or_default().push(line);
        }
        if by_doc.is_empty() {
            self.message = "no open local buffers among the results — open them first".into();
            return;
        }
        let mut excerpts = Vec::new();
        for (source, mut lines) in by_doc {
            lines.sort_unstable();
            lines.dedup();
            let buf = &self.docs.get(source).unwrap().buf;
            // Merge adjacent lines into one excerpt so an edit never
            // applies twice to overlapping spans.
            let mut spans: Vec<(usize, usize)> = Vec::new();
            for line in lines {
                if line >= buf.len_lines() {
                    continue;
                }
                match spans.last_mut() {
                    Some((_, end)) if line <= *end => *end = line + 1,
                    _ => spans.push((line, line + 1)),
                }
            }
            for (start_line, end_line) in spans {
                let start = buf.line_start(start_line);
                let end = if end_line >= buf.len_lines() {
                    buf.len_bytes()
                } else {
                    buf.line_start(end_line)
                };
                let text = buf.text().byte_slice(start..end).to_string();
                excerpts.push(Excerpt {
                    source,
                    start,
                    end,
                    fingerprint: fingerprint(&text),
                    view_line: 0,
                    view_lines: 0,
                });
            }
        }
        excerpts.sort_by_key(|excerpt| (excerpt.source, excerpt.start));
        let excerpt_count = excerpts.len();
        let id = self.docs.insert(Document::output(Buffer::from_text("")));
        let mut collection = Collection {
            title,
            excerpts,
            shadow: String::new(),
            revision: BufferRevision::new(0),
        };
        let text = render(&self.docs, &self.cwd, &mut collection);
        collection.shadow = text.clone();
        self.collections.insert(id, collection);
        let _ = self.doc_mut(id).buf.system_edit().replace_all(&text);
        self.docs.get_mut(id).unwrap().buf.readonly = false;
        let revision = self.docs.get(id).unwrap().buf.revision();
        self.collections.get_mut(&id).unwrap().revision = revision;
        self.drop_stale_scratch(id);
        self.switch_to(id);
        self.set_head(0);
        self.message = match skipped {
            0 => format!("collection: {excerpt_count} excerpt(s)"),
            _ => format!("collection built; {skipped} hit(s) skipped (not open local buffers)"),
        };
    }

    /// Every normal-mode action boundary in a collection buffer is a
    /// write-back attempt — gated on the buffer revision so motions and
    /// in-progress insert typing never materialize rope text.
    pub(crate) fn maybe_sync_collection(&mut self) {
        if self.docs.is_empty() {
            return;
        }
        let id = self.current();
        let revision = self.buf().revision();
        let Some(stored) = self.collections.get(&id).map(|c| c.revision) else {
            return;
        };
        if revision == stored {
            return;
        }
        let current = self.buf().text().to_string();
        if current == self.collections[&id].shadow {
            self.collections.get_mut(&id).unwrap().revision = revision;
            return;
        }
        // Write-back against a working copy; the entry stays in the map
        // so the change-journal remap still tracks its anchors mid-apply.
        let working = self.collections[&id].clone();
        if let Err(reason) = self.collection_write_back(&working, &current) {
            self.message = reason;
        }
        let open = working
            .excerpts
            .iter()
            .all(|excerpt| self.docs.get(excerpt.source).is_some());
        if !open {
            self.message = "collection: a source buffer was closed — view dropped".into();
            self.collections.remove(&id);
            return;
        }
        // The source is authoritative: regenerate the view and reset the
        // shadow whether or not the write-back landed.
        let text = {
            let entry = self.collections.get_mut(&id).unwrap();
            render(&self.docs, &self.cwd, entry)
        };
        {
            let entry = self.collections.get_mut(&id).unwrap();
            entry.shadow = text.clone();
        }
        let _ = self.doc_mut(id).buf.system_edit().replace_all(&text);
        let revision = self.docs.get(id).unwrap().buf.revision();
        self.collections.get_mut(&id).unwrap().revision = revision;
    }

    /// Diff shadow vs current (one contiguous changed region — one user
    /// action), map it into exactly one excerpt, and apply through the
    /// change-plan gateway. Anything wider refuses; the caller refreshes.
    fn collection_write_back(
        &mut self,
        collection: &Collection,
        current: &str,
    ) -> Result<(), String> {
        let shadow_lines: Vec<&str> = collection.shadow.split_inclusive('\n').collect();
        let current_lines: Vec<&str> = current.split_inclusive('\n').collect();
        let prefix = shadow_lines
            .iter()
            .zip(&current_lines)
            .take_while(|(a, b)| a == b)
            .count();
        let shared = shadow_lines.len().min(current_lines.len());
        let suffix = shadow_lines
            .iter()
            .rev()
            .zip(current_lines.iter().rev())
            .take(shared - prefix)
            .take_while(|(a, b)| a == b)
            .count();
        let shadow_middle = prefix..shadow_lines.len() - suffix;
        let current_middle = prefix..current_lines.len() - suffix;
        if shadow_middle.is_empty() && current_middle.is_empty() {
            return Ok(());
        }
        // The changed line region must sit fully inside one excerpt's
        // source-line span: [view_line + 1, view_line + view_lines].
        let mut hit = None;
        for (index, excerpt) in collection.excerpts.iter().enumerate() {
            let lo = excerpt.view_line + 1;
            let hi = excerpt.view_line + excerpt.view_lines + 1; // exclusive
            if shadow_middle.start >= lo && shadow_middle.end <= hi {
                hit = Some(index);
                break;
            }
            // Any overlap that is not full containment crosses a boundary.
            if shadow_middle.start < hi && shadow_middle.end > lo {
                return Err("edit spans excerpt boundaries — refused; view refreshed".into());
            }
        }
        let Some(index) = hit else {
            return Err("edit touches the title or a header — refused; view refreshed".into());
        };
        let excerpt = &collection.excerpts[index];
        let source = self
            .docs
            .get(excerpt.source)
            .ok_or_else(|| "collection: a source buffer was closed".to_string())?;
        let present = source
            .buf
            .text()
            .byte_slice(excerpt.start..excerpt.end)
            .to_string();
        if fingerprint(&present) != excerpt.fingerprint {
            return Err(
                "collection: the source changed elsewhere — refused; view refreshed".into(),
            );
        }
        let mut replacement: String = current_lines[current_middle].concat();
        if !replacement.is_empty() && !replacement.ends_with('\n') {
            replacement.push('\n');
        }
        let plan = ChangePlan {
            producer: ChangeProducer::CollectionEdit,
            documents: vec![PlannedDocument {
                location: ResourceLocation::local(source.buf.path.clone().unwrap_or_default()),
                document: excerpt.source,
                base: source.buf.revision(),
                edits: vec![strop_core::Replacement::new(
                    Range::charwise(excerpt.start, excerpt.end),
                    replacement,
                )],
            }],
            refused: Vec::new(),
        };
        self.apply_change_plan(plan);
        Ok(())
    }
}
