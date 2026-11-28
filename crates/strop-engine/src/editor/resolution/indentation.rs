//! A deferred indentation runs on the owner of the document's syntax tree
//! (#309): the analysis actor, which already keeps that tree current through
//! the edit journal. There is no second parser here.
use super::super::analysis::{worker::Trees, AnalysisTarget};
use super::super::structural_indent::{self, Batch, Edit, Trigger};
use super::super::Editor;
use super::{Operation, ResolutionData, ResolutionEvent, ResolutionKey, ResolutionPurpose};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use strop_core::frontend_input::Key;
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};

/// One deferred indentation, ready to run on the tree's owner.
pub(super) struct Job {
    pub ticket: Ticket<ResolutionKey>,
    pub batch: Batch,
    pub ropes: Vec<ropey::Rope>,
    pub cancel: Arc<AtomicBool>,
    pub events: mpsc::Sender<ResolutionEvent>,
}

impl Job {
    pub(super) fn run(self, trees: &mut Trees<'_>) {
        let cancelled = || self.cancel.load(Ordering::Acquire);
        let outcome = if cancelled() {
            Outcome::Cancelled(CancelReason::Superseded)
        } else {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                resolve(&self.batch, &self.ropes, trees, &cancelled)
            }));
            match result {
                _ if cancelled() => Outcome::Cancelled(CancelReason::Superseded),
                Ok(Ok(edits)) => Outcome::Success(ResolutionData {
                    resolved: Vec::new(),
                    plan: None,
                    layouts: Vec::new(),
                    indentation: Some(edits),
                    substitution: None,
                }),
                Ok(Err(error)) => Outcome::failed(FailureKind::InvalidInput, error),
                Err(_) => Outcome::failed(FailureKind::Panic, "indentation failed"),
            }
        };
        let _ = self
            .events
            .send(ResolutionEvent::Completed(Box::new(Completion {
                ticket: self.ticket,
                outcome,
            })));
    }
}

fn resolve(
    batch: &Batch,
    ropes: &[ropey::Rope],
    trees: &mut Trees<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Edit>, String> {
    if batch.sources.len() != ropes.len() {
        return Err("indentation source snapshot missing".into());
    }
    let mut edits = Vec::new();
    for (source, rope) in batch.sources.iter().zip(ropes) {
        if cancelled() {
            return Err("indentation cancelled".into());
        }
        let buffer = strop_core::Buffer::from_snapshot(rope.clone());
        let cursors = source.cursors();
        let positions = structural_indent::probes(&buffer, &cursors, batch.trigger);
        let contexts = match source.request.language {
            Some(language) if structural_indent::structural(&source.request) => trees
                .indentation_contexts(
                    source.document,
                    language,
                    rope,
                    source.revision,
                    &positions,
                    cancelled,
                )
                .map_err(|error| error.to_string())?,
            _ => structural_indent::previous_line(&positions),
        };
        edits.extend(source.project(structural_indent::plan(
            &buffer,
            &cursors,
            &source.request,
            &contexts,
        ))?);
    }
    Batch::order(&mut edits)?;
    Ok(edits)
}

impl Editor {
    pub(crate) fn resolve_indentation(&mut self, trigger: Trigger) -> Option<Vec<Edit>> {
        let Some(request) = self.indentation_request(trigger) else {
            self.message = "indentation requires an editable source position".into();
            return None;
        };
        let cursors = self.all_cursors();
        let operation = Operation::Indentation(request.clone());
        if let Some((key, ready)) = &self.resolution.ready {
            if self.resolution_matches(key, &operation, &cursors) {
                return ready.as_ref().ok()?.indentation.clone();
            }
        }
        let mut edits = Vec::new();
        for source in &request.sources {
            let buffer = &self.docs.get(source.document)?.buf;
            let cursors = source.cursors();
            let positions = structural_indent::probes(buffer, &cursors, trigger);
            let Some(contexts) = self.inline_indentation(source, &positions) else {
                self.defer_operation(
                    operation,
                    self.all_cursors(),
                    ResolutionPurpose::Indentation,
                );
                return None;
            };
            let buffer = &self.docs.get(source.document)?.buf;
            match source.project(structural_indent::plan(
                buffer,
                &cursors,
                &source.request,
                &contexts,
            )) {
                Ok(projected) => edits.extend(projected),
                Err(error) => {
                    self.message = error;
                    return None;
                }
            }
        }
        if let Err(error) = Batch::order(&mut edits) {
            self.message = error;
            return None;
        }
        Some(edits)
    }

    /// Indent contexts the input path can answer under a fixed bound: no
    /// grammar, a tiny unambiguous prefix, the published tree at this
    /// revision (#309), or a file too large to parse. `None` defers to the
    /// tree's owner.
    fn inline_indentation(
        &mut self,
        source: &structural_indent::Source,
        positions: &[strop_core::id::ByteOffset],
    ) -> Option<Vec<strop_syntax::IndentContext>> {
        let buffer = &self.docs.get(source.document)?.buf;
        let language = match source.request.language {
            Some(language) if structural_indent::structural(&source.request) => language,
            _ => return Some(structural_indent::previous_line(positions)),
        };
        let small = positions
            .iter()
            .all(|at| buffer.line_end(buffer.line_of(at.get())) <= 4096);
        if let Some(contexts) = small
            .then(|| structural_indent::inline_contexts(buffer, positions))
            .flatten()
        {
            return Some(contexts);
        }
        if buffer.len_bytes() > strop_syntax::PARSE_LIMIT {
            if self.analysis.large_notice(source.document) {
                self.message = "file too large to parse: indenting from the previous line".into();
            }
            return Some(structural_indent::previous_line(positions));
        }
        // The tree is native state; the tape records the answer.
        let revision = buffer.revision();
        let tree = self
            .analysis
            .tree(source.document, revision)
            .filter(|tree| tree.language() == language);
        let looked = self.tape.call(
            "indent.contexts",
            &(source.document, revision, positions),
            || {
                tree.and_then(|tree| {
                    tree.indentation_contexts(buffer.text(), positions, strop_syntax::INPUT_BUDGET)
                })
            },
        );
        match looked {
            Ok(contexts) => contexts,
            Err(error) => {
                self.message = format!("indentation: {error}");
                None
            }
        }
    }

    pub(super) fn finish_indentation(&mut self, trigger: Trigger) {
        match trigger {
            Trigger::Enter => self.feed_insert(Key::Enter),
            Trigger::Closer(kind) => self.feed_insert(Key::Char(kind.closing_character())),
            Trigger::Label => self.feed_insert(Key::Char(':')),
            Trigger::Below => self.open_below(),
            Trigger::Above => self.open_above(),
        }
    }

    /// A deferred indentation runs on the analysis actor, the owner of
    /// the documents' syntax trees (#309), and reports here.
    pub(super) fn launch_indentation(
        &mut self,
        ticket: Ticket<ResolutionKey>,
        batch: Batch,
        cancel: Arc<AtomicBool>,
    ) {
        let ropes = batch
            .sources
            .iter()
            .filter_map(|source| {
                // the actor's tree follows a document's journal only once
                // the document is registered
                self.analysis
                    .register(AnalysisTarget::Document(source.document));
                self.docs.get(source.document).map(|doc| doc.buf.snapshot())
            })
            .collect();
        let job = Job {
            ticket: ticket.clone(),
            batch,
            ropes,
            cancel,
            events: self.resolution.tx.clone(),
        };
        let sent = match self.analysis.worker() {
            Some(worker) => worker.syntax(Box::new(move |trees| job.run(trees))).is_ok(),
            None => false,
        };
        if !sent {
            self.handle_resolution(ResolutionEvent::Completed(Box::new(Completion {
                ticket,
                outcome: Outcome::failed(
                    FailureKind::Disconnected,
                    "display analysis worker stopped",
                ),
            })));
        }
    }
}
