//! The trace↔model correspondence oracle (R12): an independent checker
//! over publication/admission/delivery events. It derives expected text,
//! revisions and staleness from the event stream itself — never from the
//! editor's own bookkeeping — so a mutated editor that disagrees with its
//! declared journal fails the check.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use strop_core::id::{BufferRevision, DocumentId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Document {
    pub id: DocumentId,
    pub revision: BufferRevision,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Batch coordinate system: pre-edit (one atomic ChangeSet) or sequential
/// (undo/redo/system execution order).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Geometry {
    PreEdit,
    Sequential,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Request {
    Worker(u64),
    Lsp { server: u64, request: u64 },
}

/// The owner a request was admitted under: the document/revision it must
/// still match at delivery, plus an opaque scope discriminator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Owner {
    pub document: Option<DocumentId>,
    pub revision: Option<BufferRevision>,
    pub scope: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Open(Document),
    Close {
        document: DocumentId,
        panes: Vec<DocumentId>,
    },
    Publish {
        document: DocumentId,
        base: BufferRevision,
        next: BufferRevision,
        geometry: Geometry,
        edits: Vec<Edit>,
        text: String,
    },
    Refused {
        before: Document,
        after: Document,
        history_before: serde_json::Value,
        history_after: serde_json::Value,
    },
    Admit {
        request: Request,
        owner: Owner,
    },
    Revoke(Request),
    Deliver {
        request: Request,
        owner: Owner,
        published: bool,
        terminal: bool,
    },
    Observe {
        documents: Vec<Document>,
        panes: Vec<DocumentId>,
    },
}

#[derive(Default)]
pub struct Model {
    docs: BTreeMap<DocumentId, Document>,
    seen: BTreeSet<DocumentId>,
    pending: BTreeMap<Request, Owner>,
    requests: BTreeSet<Request>,
}

fn range(text: &str, edit: &Edit) -> Result<(), String> {
    if edit.start > edit.end
        || edit.end > text.len()
        || !text.is_char_boundary(edit.start)
        || !text.is_char_boundary(edit.end)
    {
        return Err("invalid UTF-8 byte geometry".into());
    }
    Ok(())
}

/// The independent text oracle. Pre-edit geometry sorts non-empty
/// replacements and applies them back-to-front, rejecting any equal-start
/// pair (duplicate insertions conflict); end==next-start ranges are
/// adjacent and valid.
pub fn apply_text(before: &str, geometry: Geometry, edits: &[Edit]) -> Result<String, String> {
    let mut result = before.to_owned();
    match geometry {
        Geometry::Sequential => {
            for edit in edits {
                range(&result, edit)?;
                result.replace_range(edit.start..edit.end, &edit.text);
            }
        }
        Geometry::PreEdit => {
            let mut order: Vec<_> = edits
                .iter()
                .filter(|edit| edit.start != edit.end || !edit.text.is_empty())
                .collect();
            order.sort_by_key(|edit| (edit.start, edit.end));
            for edit in &order {
                range(before, edit)?;
            }
            if order
                .windows(2)
                .any(|pair| pair[0].start == pair[1].start || pair[0].end > pair[1].start)
            {
                return Err("conflicting pre-edit replacements".into());
            }
            for edit in order.into_iter().rev() {
                result.replace_range(edit.start..edit.end, &edit.text);
            }
        }
    }
    Ok(result)
}

impl Model {
    fn panes(&self, panes: &[DocumentId]) -> Result<(), String> {
        if panes.iter().any(|id| !self.docs.contains_key(id)) {
            return Err("pane references dead document".into());
        }
        Ok(())
    }

    pub fn step(&mut self, event: Event) -> Result<(), String> {
        match event {
            Event::Open(doc) => {
                if !self.seen.insert(doc.id) {
                    return Err("document incarnation reopened".into());
                }
                self.docs.insert(doc.id, doc);
            }
            Event::Close { document, panes } => {
                if self.docs.remove(&document).is_none() {
                    return Err("closing absent document".into());
                }
                self.panes(&panes)?;
            }
            Event::Publish {
                document,
                base,
                next,
                geometry,
                edits,
                text,
            } => {
                let doc = self
                    .docs
                    .get_mut(&document)
                    .ok_or("publication to absent document")?;
                if doc.revision != base {
                    return Err("publication base mismatch".into());
                }
                let expected = apply_text(&doc.text, geometry, &edits)?;
                if expected != text {
                    return Err("published bytes disagree with edits".into());
                }
                let count = match geometry {
                    Geometry::Sequential => edits.len(),
                    Geometry::PreEdit => edits
                        .iter()
                        .filter(|edit| edit.start != edit.end || !edit.text.is_empty())
                        .count(),
                };
                let expected_revision = base
                    .get()
                    .checked_add(u64::try_from(count).map_err(|_| "edit count overflow")?)
                    .ok_or("revision overflow")?;
                if next.get() != expected_revision {
                    return Err("publication revision disagrees with journal".into());
                }
                doc.text = text;
                doc.revision = next;
            }
            Event::Refused {
                before,
                after,
                history_before,
                history_after,
            } => {
                if before != after || history_before != history_after {
                    return Err("refused edit partially changed state".into());
                }
                if self.docs.get(&before.id) != Some(&before) {
                    return Err("refusal baseline mismatch".into());
                }
            }
            Event::Admit { request, owner } => {
                if !self.requests.insert(request.clone()) {
                    return Err("request identity reused".into());
                }
                if let Some(id) = owner.document {
                    let doc = self.docs.get(&id).ok_or("request for absent document")?;
                    if owner.revision != Some(doc.revision) {
                        return Err("request revision is not current".into());
                    }
                }
                self.pending.insert(request, owner);
            }
            Event::Revoke(request) => {
                self.pending.remove(&request);
            }
            Event::Deliver {
                request,
                owner,
                published,
                terminal,
            } => {
                let owned = self.pending.get(&request) == Some(&owner);
                let fresh = owner
                    .document
                    .map(|id| {
                        self.docs
                            .get(&id)
                            .is_some_and(|doc| Some(doc.revision) == owner.revision)
                    })
                    .unwrap_or(true);
                if published && !(owned && fresh) {
                    return Err("stale or wrong-owner publication".into());
                }
                if owned && terminal {
                    self.pending.remove(&request);
                }
            }
            Event::Observe { documents, panes } => {
                let mut found = BTreeMap::new();
                for doc in documents {
                    if found.insert(doc.id, doc).is_some() {
                        return Err("duplicate observed document".into());
                    }
                }
                if found != self.docs {
                    return Err("unreported document or text transition".into());
                }
                self.panes(&panes)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
