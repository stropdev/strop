//! One CPU owner for expensive pure grammar resolution. No thread per key and
//! no join on the editor thread; a frozen rope is the complete read input.
use super::{Operation, ResolutionData, ResolutionEvent, ResolutionKey};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};

pub(super) struct Work {
    pub ticket: Ticket<ResolutionKey>,
    pub rope: ropey::Rope,
    pub cancel: Arc<AtomicBool>,
    pub source_ropes: Vec<ropey::Rope>,
}
enum Message {
    Resolve(Work),
    Edits(strop_core::id::DocumentId, Vec<strop_core::Change>),
}
pub(super) struct ResolutionWorker {
    sender: mpsc::Sender<Message>,
}
impl ResolutionWorker {
    pub fn start(events: mpsc::Sender<ResolutionEvent>) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Message>();
        std::thread::Builder::new()
            .name("grammar-resolution".into())
            .spawn(move || {
                let mut geometry = super::super::analysis::layouts::LayoutCache::default();
                let mut indentation = super::indentation::Cache::default();
                while let Ok(message) = receiver.recv() {
                    let work = match message {
                        Message::Edits(document, changes) => {
                            indentation.edits(document, &changes);
                            continue;
                        }
                        Message::Resolve(work) => work,
                    };
                    let outcome = if work.cancel.load(Ordering::Acquire) {
                        Outcome::Cancelled(CancelReason::Superseded)
                    } else {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            if let Operation::Indentation(request) = &work.ticket.key.operation {
                                let edits =
                                    indentation.resolve(request, &work.source_ropes, || {
                                        work.cancel.load(Ordering::Acquire)
                                    })?;
                                return Ok(ResolutionData {
                                    resolved: Vec::new(),
                                    plan: None,
                                    layouts: Vec::new(),
                                    indentation: Some(edits),
                                });
                            }
                            let buffer = strop_core::Buffer::from_snapshot(work.rope);
                            let Operation::Grammar(mut command) = work.ticket.key.operation.clone()
                            else {
                                unreachable!()
                            };
                            bind_cancellation(&mut command.target, &work.cancel);
                            let resolved = strop_grammar::resolve_many_cancellable(
                                &buffer,
                                &work.ticket.key.cursors,
                                &command,
                                &work.cancel,
                            )
                            .map_err(|error| error.to_string())?;
                            let plan = if command.op.is_some() {
                                strop_grammar::ActionPlan::from_resolved(
                                    &work.ticket.key.cursors,
                                    &resolved,
                                )
                            } else {
                                None
                            };
                            let mut lines = std::collections::BTreeSet::new();
                            for cursor in &work.ticket.key.cursors {
                                lines.insert(strop_core::id::LineIndex::new(
                                    buffer.line_of(*cursor),
                                ));
                            }
                            for value in resolved.iter().flatten() {
                                for byte in [
                                    value.range.start.get(),
                                    value.range.end.get(),
                                    value.motion_target.unwrap_or(value.range.start.get()),
                                ] {
                                    lines.insert(strop_core::id::LineIndex::new(
                                        buffer.line_of(byte),
                                    ));
                                }
                            }
                            let layouts = geometry
                                .prepare(
                                    &buffer,
                                    &super::super::analysis::AnalysisTarget::Document(
                                        work.ticket.key.document,
                                    ),
                                    work.ticket.key.revision,
                                    work.ticket.key.tab,
                                    lines,
                                    || work.cancel.load(Ordering::Acquire),
                                )
                                .ok_or_else(|| "grammar resolution cancelled".to_string())?;
                            Ok::<_, String>(ResolutionData {
                                resolved,
                                plan,
                                layouts,
                                indentation: None,
                            })
                        }));
                        match result {
                            _ if work.cancel.load(Ordering::Acquire) => {
                                Outcome::Cancelled(CancelReason::Superseded)
                            }
                            Ok(Ok(data)) => Outcome::Success(data),
                            Ok(Err(error)) => {
                                Outcome::failed(FailureKind::InvalidInput, error.to_string())
                            }
                            Err(_) => {
                                Outcome::failed(FailureKind::Panic, "grammar resolution failed")
                            }
                        }
                    };
                    if events
                        .send(ResolutionEvent::Completed(Box::new(Completion {
                            ticket: work.ticket,
                            outcome,
                        })))
                        .is_err()
                    {
                        return;
                    }
                }
                let _ = events.send(ResolutionEvent::Stopped);
            })?;
        Ok(Self { sender })
    }
    pub fn resolve(&self, work: Work) -> Result<(), Box<Work>> {
        self.sender
            .send(Message::Resolve(work))
            .map_err(|error| match error.0 {
                Message::Resolve(work) => Box::new(work),
                Message::Edits(..) => unreachable!(),
            })
    }
    pub fn edits(&self, document: strop_core::id::DocumentId, changes: &[strop_core::Change]) {
        let _ = self.sender.send(Message::Edits(document, changes.to_vec()));
    }
}
fn bind_cancellation(target: &mut strop_grammar::Target, flag: &Arc<AtomicBool>) {
    use strop_grammar::{Motion, Target};
    match target {
        Target::Motion(Motion::Search(query) | Motion::SearchBackward(query)) => {
            *query = query.cancellable(flag.clone());
        }
        Target::SurroundAdd { inner, .. } => bind_cancellation(inner, flag),
        _ => {}
    }
}
