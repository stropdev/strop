//! Replacement preparation is a finite owned job: group the included dataset,
//! read unopened local sources, validate witnesses and prepare exact diff rows.
//! Publication installs real buffers and refuses observations that moved meanwhile.
use super::{render, ReviewBuffer, ReviewRow};
use crate::editor::changes::{ChangePlan, ChangeProducer, PlannedDocument};
use crate::editor::io::{IoEvent, Opened};
use crate::editor::picker::search::{SearchScope, SearchStamp};
use crate::editor::picker::{checked_hit_range, ReplacementHit};
use crate::editor::{Document, Editor};
use crate::files::FileTarget;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, CancelReason, Completion, Outcome, Ticket};
use strop_core::{Buffer, Replacement};
use strop_workspace::ResourceLocation;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PreparationKey {
    pub(crate) stamp: SearchStamp,
    scope: SearchScope,
    proposal_epoch: usize,
    versions: Vec<(DocumentId, BufferRevision)>,
}
pub(crate) struct Preparation {
    pub ticket: Ticket<PreparationKey>,
    pub focus_ready: bool,
    pub focus_epoch: u64,
}
struct Snapshot {
    document: DocumentId,
    revision: BufferRevision,
    path: PathBuf,
    identity: Option<PathBuf>,
    text: ropey::Rope,
    readonly: bool,
}
struct Input {
    scope: SearchScope,
    catalog: strop_picker::Catalog,
    workset: strop_picker::WorksetSnapshot,
    replacement: Arc<str>,
    snapshots: Vec<Snapshot>,
}
#[derive(serde::Serialize, serde::Deserialize)]
enum Source {
    Existing {
        document: DocumentId,
        revision: BufferRevision,
    },
    Loaded(Box<Opened>),
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Target {
    location: ResourceLocation,
    source: Source,
    edits: Vec<Replacement>,
    diff: ReviewBuffer,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PreparedReview {
    targets: Vec<Target>,
    refused: Vec<(ResourceLocation, String)>,
    summary: String,
}

impl Editor {
    pub(crate) fn prepare_search_review(&mut self) {
        let Some(glue) = self.picker.as_ref() else {
            return;
        };
        let Some(context) = glue.search.as_ref() else {
            self.message = "Search has no captured scope".into();
            return;
        };
        if glue.picker.streaming || glue.rank_pending.is_some() || glue.picker.error.is_some() {
            self.message =
                "Search dataset is incomplete; finish or refresh it before Review".into();
            return;
        }
        if glue
            .query
            .as_ref()
            .is_none_or(|query| query.content.is_none())
        {
            self.message = "Review needs a content search expression".into();
            return;
        }
        if glue.picker.rows.len() <= glue.picker.excluded_count() {
            self.message = "Review has no included matches".into();
            return;
        }
        let stamp = context.stamp;
        let scope = context.scope.clone();
        let catalog = glue.picker.items.clone();
        let workset = glue.picker.workset_snapshot();
        let replacement: Arc<str> = glue.picker.replace_input.text.as_str().into();
        let snapshots: Vec<_> = self
            .docs
            .iter()
            .filter_map(|(document, doc)| {
                if !matches!(doc.source, crate::editor::document::DocumentSource::File) {
                    return None;
                }
                Some(Snapshot {
                    document,
                    revision: doc.buf.revision(),
                    path: self.cwd.join(doc.buf.path.as_ref()?),
                    identity: doc.buf.file_identity().map(ToOwned::to_owned),
                    text: doc.buf.text().clone(),
                    readonly: doc.buf.readonly,
                })
            })
            .collect();
        let versions = snapshots
            .iter()
            .map(|snapshot| (snapshot.document, snapshot.revision))
            .collect();
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        self.cancel_review_preparation();
        self.close_picker();
        let ticket = Ticket {
            request,
            key: PreparationKey {
                stamp,
                scope: scope.clone(),
                proposal_epoch: self.review.seq,
                versions,
            },
        };
        self.review.preparing = Some(Preparation {
            ticket: ticket.clone(),
            focus_ready: true,
            focus_epoch: self.focus_epoch,
        });
        self.message = "preparing replacement review".into();
        match self.tape.request("search.review", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_review_prepared(Completion {
                    ticket,
                    outcome: Outcome::failed(worker::FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let input = Input {
            scope,
            catalog,
            workset,
            replacement,
            snapshots,
        };
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "search-review",
            move |outcome| {
                let _ = tx.send(IoEvent::Review(Box::new(Completion { ticket, outcome })));
            },
            move |cancel| prepare(input, &cancel),
        );
        self.worker_handles.insert(request, handle);
    }

    pub(crate) fn cancel_review_preparation(&mut self) {
        if let Some(pending) = self.review.preparing.take() {
            if let Some(handle) = self.worker_handles.remove(&pending.ticket.request) {
                handle.cancel(CancelReason::Superseded);
            }
        }
    }

    pub(crate) fn handle_review_prepared(
        &mut self,
        completion: Completion<PreparationKey, PreparedReview>,
    ) {
        if self
            .review
            .preparing
            .as_ref()
            .map(|pending| &pending.ticket)
            != Some(&completion.ticket)
        {
            return;
        }
        let focus = self.review.preparing.take().is_some_and(|pending| {
            pending.focus_ready && pending.focus_epoch == self.focus_epoch && !self.picker_open()
        });
        self.worker_handles.remove(&completion.ticket.request);
        let key = completion.ticket.key;
        if self.search_stamp(key.stamp.session) != Some(key.stamp)
            || self.review.seq != key.proposal_epoch
        {
            return;
        }
        let prepared = match completion.outcome {
            Outcome::Success(prepared) => prepared,
            Outcome::Cancelled(_) => {
                self.message = "replacement review cancelled".into();
                return;
            }
            Outcome::Failed { failure, .. } => {
                self.message = format!("replacement review failed: {}", failure.message);
                return;
            }
        };
        let mut plan = ChangePlan {
            producer: ChangeProducer::Replace,
            documents: Vec::new(),
            refused: prepared.refused,
        };
        let mut body = ReviewBuffer::default();
        body.line(&prepared.summary, ReviewRow::Context);
        for target in prepared.targets {
            let document = match target.source {
                Source::Existing { document, revision } => {
                    if self.docs.get(document).is_none_or(|doc| {
                        doc.buf.revision() != revision
                            || doc.buf.readonly
                            || !doc.matches_target(&FileTarget::Local(target.location.path.clone()))
                    }) {
                        plan.refused.push((
                            target.location,
                            "source changed or closed while preparing; review again".into(),
                        ));
                        continue;
                    }
                    document
                }
                Source::Loaded(opened) => {
                    if self
                        .docs
                        .iter()
                        .any(|(_, doc)| doc.matches_target(&opened.canonical))
                    {
                        plan.refused.push((
                            target.location,
                            "source opened while preparing; review again".into(),
                        ));
                        continue;
                    }
                    let id = self.docs.insert(opened.document);
                    self.mru.push(id);
                    self.generation += 1;
                    self.resolve_indent_for(id);
                    id
                }
            };
            if self.doc(document).buf.dirty {
                body.line(
                    &format!(
                        "{} — unsaved source buffer is authoritative",
                        target.location.label()
                    ),
                    ReviewRow::Context,
                );
            }
            plan.documents.push(PlannedDocument {
                document,
                base: self.doc(document).buf.revision(),
                location: target.location,
                edits: target.edits,
            });
            body.line("", ReviewRow::Context);
            body.append(target.diff);
        }
        self.present_prepared_search_review(plan, body, key.stamp, focus);
    }
}

fn prepare(input: Input, cancel: &worker::CancelToken) -> Outcome<PreparedReview> {
    let mut groups: BTreeMap<PathBuf, Vec<ReplacementHit>> = BTreeMap::new();
    let mut excluded = 0;
    for (index, item) in input.catalog.iter().enumerate() {
        if index % 128 == 0 && cancel.is_cancelled() {
            return Outcome::Cancelled(CancelReason::Superseded);
        }
        if input.workset.is_excluded(&item.payload) {
            excluded += 1;
            continue;
        }
        if let strop_picker::Payload::Grep {
            path,
            line,
            col,
            match_len,
            line_text,
        } = &item.payload
        {
            groups
                .entry(input.scope.root.path.join(path))
                .or_default()
                .push(ReplacementHit {
                    line: *line,
                    col: *col,
                    match_len: *match_len,
                    text: line_text.clone(),
                });
        }
    }
    let mut lookup = HashMap::new();
    for (index, snapshot) in input.snapshots.iter().enumerate() {
        lookup.insert(snapshot.path.clone(), index);
        if let Some(identity) = &snapshot.identity {
            lookup.insert(identity.clone(), index);
        }
    }
    // Canonical aliases name one edit target, never two sequential applications
    // against the same base. Exact duplicate hits are removed below.
    let mut sources: BTreeMap<PathBuf, Vec<ReplacementHit>> = BTreeMap::new();
    for (path, hits) in groups {
        if cancel.is_cancelled() {
            return Outcome::Cancelled(CancelReason::Superseded);
        }
        let identity = lookup
            .get(&path)
            .and_then(|index| input.snapshots[*index].identity.clone())
            .or_else(|| std::fs::canonicalize(&path).ok())
            .unwrap_or(path);
        sources.entry(identity).or_default().extend(hits);
    }
    let mut result = PreparedReview {
        targets: Vec::new(), refused: Vec::new(),
        summary: format!("{}: {} included matches, {excluded} excluded; Apply edits buffers, :save-change persists files",
            input.scope.root.label(), input.catalog.len().saturating_sub(excluded)),
    };
    for (path, mut hits) in sources {
        if cancel.is_cancelled() {
            return Outcome::Cancelled(CancelReason::Superseded);
        }
        let location = ResourceLocation::local(path.clone());
        let snapshot = lookup.get(&path).copied().or_else(|| {
            std::fs::canonicalize(&path)
                .ok()
                .and_then(|path| lookup.get(&path).copied())
        });
        let (buffer, source) = if let Some(index) = snapshot {
            let snapshot = &input.snapshots[index];
            if snapshot.readonly {
                result
                    .refused
                    .push((location, "buffer is read-only".into()));
                continue;
            }
            (
                Buffer::from_snapshot(snapshot.text.clone()),
                Source::Existing {
                    document: snapshot.document,
                    revision: snapshot.revision,
                },
            )
        } else {
            match Buffer::open(&path) {
                Ok(buffer) => {
                    let canonical = buffer
                        .file_identity()
                        .map_or_else(|| path.clone(), ToOwned::to_owned);
                    let snapshot = Buffer::from_snapshot(buffer.text().clone());
                    (
                        snapshot,
                        Source::Loaded(Box::new(Opened {
                            document: Document::new(buffer),
                            canonical: FileTarget::Local(canonical),
                        })),
                    )
                }
                Err(error) => {
                    result.refused.push((location, error.to_string()));
                    continue;
                }
            }
        };
        hits.sort_unstable_by_key(|hit| (hit.line, hit.col, hit.match_len));
        hits.dedup();
        let mut edits = Vec::new();
        let mut stale = 0;
        for (index, hit) in hits.iter().enumerate() {
            if index % 128 == 0 && cancel.is_cancelled() {
                return Outcome::Cancelled(CancelReason::Superseded);
            }
            if let Some(range) = checked_hit_range(buffer.text(), hit) {
                edits.push(Replacement::new(range, input.replacement.as_ref()));
            } else {
                stale += 1;
            }
        }
        if stale > 0 {
            result.refused.push((
                location.clone(),
                format!("{stale} source match(es) changed since search"),
            ));
        }
        if edits.is_empty() {
            continue;
        }
        let label = path
            .strip_prefix(&input.scope.root.path)
            .unwrap_or(&path)
            .display()
            .to_string();
        match render::file_diff(&label, &buffer, buffer.revision(), &edits) {
            Ok(diff) => result.targets.push(Target {
                location,
                source,
                edits,
                diff,
            }),
            Err(error) => result.refused.push((location, error.to_string())),
        }
    }
    if cancel.is_cancelled() {
        Outcome::Cancelled(CancelReason::Superseded)
    } else {
        Outcome::Success(result)
    }
}
