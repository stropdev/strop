//! One analysis actor owns parser state and indentation indexes. Jobs and edit
//! journals arrive in order; parser/query cancellation is cooperative, never a
//! join on the input thread.
use super::{AnalysisEvent, AnalysisKey, AnalysisTarget, FrameAnalysis};
use crate::editor::matching::{match_delimiters, MatchKey, PairMatch};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};
use strop_grammar::MatchCancelled;
use strop_syntax::{HighlightError, Highlighter, IndentGuides};

pub(crate) struct Work {
    pub ticket: Ticket<AnalysisKey>,
    pub rope: ropey::Rope,
    pub cancel: Arc<AtomicBool>,
}
/// A matching-delimiter job (0051 §7 R09): same actor, same slot parser
/// state, same cooperative cancellation as viewport analysis.
pub(crate) struct MatchWork {
    pub ticket: Ticket<MatchKey>,
    pub rope: ropey::Rope,
    pub cancel: Arc<AtomicBool>,
}
pub struct Slot {
    highlighter: Option<Highlighter>,
    syntax_path: Option<std::path::PathBuf>,
    guides: Option<(strop_core::id::BufferRevision, usize, IndentGuides)>,
    layouts: super::layouts::LayoutCache,
}
enum Message {
    Analyze(Work),
    Match(MatchWork),
    Edits(AnalysisTarget, Vec<strop_core::Change>),
    Forget(AnalysisTarget),
}

/// The target's parser slot, created on first use and rebound when the
/// syntax identity changes. Guides/layouts survive a rebind: they key
/// on revision, not on the path.
fn slot_for<'a>(
    slots: &'a mut HashMap<AnalysisTarget, Slot>,
    target: &AnalysisTarget,
    syntax_path: &Option<std::path::PathBuf>,
    rope: &ropey::Rope,
) -> &'a mut Slot {
    let slot = slots.entry(target.clone()).or_insert_with(|| Slot {
        highlighter: syntax_path
            .as_ref()
            .and_then(|path| Highlighter::for_path(path, rope)),
        syntax_path: syntax_path.clone(),
        guides: None,
        layouts: super::layouts::LayoutCache::default(),
    });
    if slot.syntax_path != *syntax_path {
        slot.highlighter = syntax_path
            .as_ref()
            .and_then(|path| Highlighter::for_path(path, rope));
        slot.syntax_path = syntax_path.clone();
    }
    slot
}
pub(crate) struct Worker {
    sender: mpsc::Sender<Message>,
}
impl Worker {
    pub fn start(events: mpsc::Sender<AnalysisEvent>) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("display-analysis".into())
            .spawn(move || {
                let mut slots: HashMap<AnalysisTarget, Slot> = HashMap::new();
                while let Ok(message) = receiver.recv() {
                    match message {
                        Message::Edits(target, edits) => {
                            if let Some(slot) = slots.get_mut(&target) {
                                for change in edits {
                                    if let Some(highlighter) = slot.highlighter.as_mut() {
                                        highlighter.apply_edits(&[change.edit], change.revision);
                                    }
                                    if let Some((revision, _, _)) = slot
                                        .guides
                                        .as_mut()
                                        .filter(|(_, _, index)| index.unaffected_by(&change.edit))
                                    {
                                        *revision = change.revision;
                                    } else {
                                        slot.guides = None;
                                    }
                                }
                            }
                        }
                        Message::Forget(target) => {
                            slots.remove(&target);
                        }
                        Message::Analyze(work) => {
                            let key = &work.ticket.key;
                            let cancelled = || work.cancel.load(Ordering::Acquire);
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    if cancelled() {
                                        return Err(HighlightError::Cancelled);
                                    }
                                    let slot = slot_for(
                                        &mut slots,
                                        &key.target,
                                        &key.syntax_path,
                                        &work.rope,
                                    );
                                    let spans = match slot.highlighter.as_mut() {
                                        Some(highlighter) => highlighter.highlight_while(
                                            &work.rope,
                                            key.revision,
                                            key.first,
                                            key.last,
                                            cancelled,
                                        )?,
                                        None => Vec::new(),
                                    };
                                    let first_line = work
                                        .rope
                                        .byte_to_line(key.first.min(work.rope.len_bytes()));
                                    let last_line = work
                                        .rope
                                        .byte_to_line(key.last.min(work.rope.len_bytes()))
                                        .saturating_add(1)
                                        .min(work.rope.len_lines());
                                    let mut buffer =
                                        strop_core::Buffer::from_snapshot(work.rope.clone());
                                    let layouts = slot
                                        .layouts
                                        .prepare(
                                            &buffer,
                                            &key.target,
                                            key.revision,
                                            key.tab,
                                            (first_line..last_line)
                                                .map(strop_core::id::LineIndex::new),
                                            cancelled,
                                        )
                                        .ok_or(HighlightError::Cancelled)?;
                                    let installed =
                                        buffer.install_line_layouts(buffer.revision(), &layouts);
                                    debug_assert!(
                                        installed,
                                        "worker layouts belong to their frozen rope"
                                    );
                                    let guides = if key.guides {
                                        if !slot.guides.as_ref().is_some_and(
                                            |(revision, tab, _)| {
                                                *revision == key.revision && *tab == key.tab
                                            },
                                        ) {
                                            let index =
                                                IndentGuides::build(&work.rope, key.tab, cancelled)
                                                    .ok_or(HighlightError::Cancelled)?;
                                            slot.guides = Some((key.revision, key.tab, index));
                                        }
                                        match &slot.guides {
                                            Some((_, _, index)) => index
                                                .frame(first_line, last_line, key.left, key.right),
                                            None => unreachable!("guide index installed above"),
                                        }
                                    } else {
                                        strop_syntax::GuideFrame::default()
                                    };
                                    let search = if let Some(query) = &key.search {
                                        match super::search::summarize(
                                            &buffer,
                                            key,
                                            query,
                                            &work.cancel,
                                        ) {
                                            Err(strop_grammar::QueryError::Cancelled) => {
                                                return Err(HighlightError::Cancelled)
                                            }
                                            result => {
                                                Some(result.map_err(|error| error.to_string()))
                                            }
                                        }
                                    } else {
                                        None
                                    };
                                    Ok(FrameAnalysis {
                                        spans,
                                        guides,
                                        search,
                                        layouts,
                                    })
                                }));
                            let outcome = match result {
                                Ok(Ok(frame)) => Outcome::Success(frame),
                                Ok(Err(HighlightError::Cancelled)) => {
                                    Outcome::Cancelled(CancelReason::Superseded)
                                }
                                Ok(Err(error)) => {
                                    Outcome::failed(FailureKind::Io, error.to_string())
                                }
                                Err(_) => {
                                    Outcome::failed(FailureKind::Panic, "display analysis failed")
                                }
                            };
                            if events
                                .send(AnalysisEvent::Completed(Box::new(Completion {
                                    ticket: work.ticket,
                                    outcome,
                                })))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Message::Match(work) => {
                            let key = &work.ticket.key;
                            let cancelled = || work.cancel.load(Ordering::Acquire);
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    if cancelled() {
                                        return Err(HighlightError::Cancelled);
                                    }
                                    let buffer =
                                        strop_core::Buffer::from_snapshot(work.rope.clone());
                                    match match_delimiters(
                                        &buffer, key.caret, key.insert, cancelled,
                                    ) {
                                        Ok(pair) => {
                                            Ok(pair
                                                .map(|(first, second)| PairMatch { first, second }))
                                        }
                                        Err(MatchCancelled) => Err(HighlightError::Cancelled),
                                    }
                                }));
                            let outcome = match result {
                                Ok(Ok(pair)) => Outcome::Success(pair),
                                Ok(Err(HighlightError::Cancelled)) => {
                                    Outcome::Cancelled(CancelReason::Superseded)
                                }
                                Ok(Err(error)) => {
                                    Outcome::failed(FailureKind::Io, error.to_string())
                                }
                                Err(_) => {
                                    Outcome::failed(FailureKind::Panic, "display analysis failed")
                                }
                            };
                            if events
                                .send(AnalysisEvent::Matched(Box::new(Completion {
                                    ticket: work.ticket,
                                    outcome,
                                })))
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
                drop(slots);
                let _ = events.send(AnalysisEvent::Stopped);
            })?;
        Ok(Self { sender })
    }
    pub fn analyze(&self, work: Work) -> Result<(), Box<Work>> {
        self.sender
            .send(Message::Analyze(work))
            .map_err(|error| match error.0 {
                Message::Analyze(work) => Box::new(work),
                _ => unreachable!("sent an analysis request"),
            })
    }
    pub fn match_work(&self, work: MatchWork) -> Result<(), Box<MatchWork>> {
        self.sender
            .send(Message::Match(work))
            .map_err(|error| match error.0 {
                Message::Match(work) => Box::new(work),
                _ => unreachable!("sent a match request"),
            })
    }
    pub fn edits(&self, target: AnalysisTarget, edits: Vec<strop_core::Change>) -> bool {
        self.sender.send(Message::Edits(target, edits)).is_ok()
    }
    pub fn forget(&self, target: AnalysisTarget) -> bool {
        self.sender.send(Message::Forget(target)).is_ok()
    }
}
