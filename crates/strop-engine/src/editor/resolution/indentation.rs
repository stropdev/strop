//! A deferred indentation runs on the owner of the document's syntax tree
//! (#309): the analysis actor, which already keeps that tree current through
//! the edit journal. There is no second parser here.
use super::super::analysis::worker::Trees;
use super::super::structural_indent::{self, Batch, Edit};
use super::{ResolutionData, ResolutionEvent, ResolutionKey};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
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
