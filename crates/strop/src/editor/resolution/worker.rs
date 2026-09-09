//! One CPU owner for expensive pure grammar resolution. No thread per key and
//! no join on the editor thread; a frozen rope is the complete read input.
use super::{ResolutionData, ResolutionEvent, ResolutionKey};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};

pub(super) struct Work {
    pub ticket: Ticket<ResolutionKey>,
    pub rope: ropey::Rope,
    pub cancel: Arc<AtomicBool>,
}
pub(super) struct ResolutionWorker {
    sender: mpsc::Sender<Work>,
}
impl ResolutionWorker {
    pub fn start(events: mpsc::Sender<ResolutionEvent>) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Work>();
        std::thread::Builder::new()
            .name("grammar-resolution".into())
            .spawn(move || {
                let mut geometry = super::super::analysis::layouts::LayoutCache::default();
                while let Ok(work) = receiver.recv() {
                    let outcome = if work.cancel.load(Ordering::Acquire) {
                        Outcome::Cancelled(CancelReason::Superseded)
                    } else {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let buffer = strop_core::Buffer::from_snapshot(work.rope);
                            let mut command = work.ticket.key.command.clone();
                            bind_cancellation(&mut command.target, &work.cancel);
                            let resolved = strop_grammar::resolve_many_cancellable(
                                &buffer,
                                &work.ticket.key.cursors,
                                &command,
                                &work.cancel,
                            )?;
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
                                .ok_or(strop_grammar::QueryError::Cancelled)?;
                            Ok::<_, strop_grammar::QueryError>(ResolutionData {
                                resolved,
                                plan,
                                layouts,
                            })
                        }));
                        match result {
                            _ if work.cancel.load(Ordering::Acquire) => {
                                Outcome::Cancelled(CancelReason::Superseded)
                            }
                            Ok(Ok(data)) => Outcome::Success(data),
                            Ok(Err(strop_grammar::QueryError::Cancelled)) => {
                                Outcome::Cancelled(CancelReason::Superseded)
                            }
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
        self.sender.send(work).map_err(|error| Box::new(error.0))
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
