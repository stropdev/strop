//! The outline thread (0099 §4.1, §5.3): declarations of a frozen rope,
//! from the linked tree-sitter grammar, for files no language server
//! serves. It is its own thread — never the display-analysis actor — so
//! highlighting does not queue behind a parse, and it runs one job at a
//! time. Cancellation is a flag the extraction polls; nothing joins it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use strop_core::id::{BufferRevision, DocumentId, LineIndex};
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};
use strop_lsp::{ProtoSymbol, ServerColumn, ServerLocation, ServerPosition};
use strop_syntax::languages::LanguageId;
use strop_syntax::symbols::DeclExtractor;
use strop_workspace::ResourceLocation;

use crate::editor::analysis::AnalysisEvent;

/// One outline job's identity: the document at one revision.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutlineKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
}

pub(crate) struct Work {
    pub ticket: Ticket<OutlineKey>,
    pub rope: ropey::Rope,
    pub language: LanguageId,
    /// Where the rows point: the document's own location.
    pub location: ResourceLocation,
    pub cancel: Arc<AtomicBool>,
}

/// How a job turns a frozen rope into rows. Production passes
/// [`extract`]; a test passes one that waits, to hold a build in flight.
pub(crate) type Extract = Box<dyn FnMut(&Work) -> Option<Vec<ProtoSymbol>> + Send>;

/// The declarations of `work.rope`, in file order. `None`: cancelled, or
/// the grammar gave no tree.
pub(crate) fn extract(
    extractors: &mut HashMap<LanguageId, DeclExtractor>,
    work: &Work,
) -> Option<Vec<ProtoSymbol>> {
    let extractor = match extractors.entry(work.language) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(DeclExtractor::for_language(work.language)?)
        }
    };
    let mut text = Vec::with_capacity(work.rope.len_bytes());
    for chunk in work.rope.chunks() {
        text.extend_from_slice(chunk.as_bytes());
    }
    let cancelled = || work.cancel.load(Ordering::Acquire);
    let declarations = extractor.extract(&text, &cancelled)?;
    Some(
        declarations
            .into_iter()
            .map(|declaration| ProtoSymbol {
                name: declaration.name,
                container: String::new(),
                kind: declaration.kind.lsp_name().into(),
                location: ServerLocation {
                    doc: work.location.clone(),
                    position: ServerPosition {
                        line: LineIndex::new(declaration.line.saturating_sub(1)),
                        column: ServerColumn::new(declaration.col.saturating_sub(1)),
                    },
                },
            })
            .collect(),
    )
}

pub(crate) struct Worker {
    sender: mpsc::Sender<Work>,
}

impl Worker {
    pub fn start(
        events: mpsc::Sender<AnalysisEvent>,
        mut extract: Extract,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Work>();
        std::thread::Builder::new()
            .name("symbol-outline".into())
            .spawn(move || {
                while let Ok(work) = receiver.recv() {
                    let outcome = if work.cancel.load(Ordering::Acquire) {
                        Outcome::Cancelled(CancelReason::Superseded)
                    } else {
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            extract(&work)
                        })) {
                            Ok(Some(symbols)) => Outcome::Success(symbols),
                            Ok(None) if work.cancel.load(Ordering::Acquire) => {
                                Outcome::Cancelled(CancelReason::Superseded)
                            }
                            Ok(None) => {
                                Outcome::failed(FailureKind::InvalidInput, "no syntax tree")
                            }
                            Err(_) => Outcome::failed(FailureKind::Panic, "symbol outline failed"),
                        }
                    };
                    let completion = Completion {
                        ticket: work.ticket,
                        outcome,
                    };
                    if events
                        .send(AnalysisEvent::Outlined(Box::new(completion)))
                        .is_err()
                    {
                        return;
                    }
                }
            })?;
        Ok(Self { sender })
    }

    /// Queue one job; `Err` returns it when the thread is gone.
    pub fn submit(&self, work: Work) -> Result<(), Box<Work>> {
        self.sender.send(work).map_err(|error| Box::new(error.0))
    }
}
