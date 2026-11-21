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
use strop_lsp::{LineSpan, ProtoSymbol, ServerColumn, ServerLocation, ServerPosition};
use strop_syntax::languages::LanguageId;
use strop_syntax::symbols::DeclExtractor;
use strop_workspace::ResourceLocation;

use crate::editor::analysis::AnalysisEvent;

/// One outline job's identity: the document at one revision.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutlineKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub language: LanguageId,
    pub workspace: super::store::Workspace,
}

pub(crate) struct Work {
    pub ticket: Ticket<OutlineKey>,
    pub rope: ropey::Rope,
    pub root: std::path::PathBuf,
    pub shard: Option<Arc<strop_picker::DiskShard>>,
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
    let convert = |declaration: strop_syntax::symbols::Declaration| {
        let (first, last) = declaration.lines();
        let (line, column) = declaration.selection[0];
        ProtoSymbol {
            name: declaration.name,
            container: String::new(),
            kind: declaration.kind,
            location: ServerLocation {
                doc: work.location.clone(),
                position: ServerPosition {
                    line: LineIndex::new(line as usize),
                    column: ServerColumn::new(column as usize),
                },
            },
            span: LineSpan {
                first: LineIndex::new(first - 1),
                last: LineIndex::new(last - 1),
                ranges: Some(strop_lsp::protocol::SymbolRanges {
                    enclosing: declaration.enclosing,
                    selection: declaration.selection,
                }),
            },
        }
    };
    let cancelled = || work.cancel.load(Ordering::Acquire);
    if let Some(shard) = &work.shard {
        let mut remaining = shard.source();
        if shard.schema()
            == strop_syntax::symbols::DeclarationSchema::ByteRangesV2(work.ticket.key.language)
            && remaining.len() == work.rope.len_bytes()
            && work.rope.chunks().all(|chunk| {
                if cancelled() || !remaining.starts_with(chunk.as_bytes()) {
                    return false;
                }
                remaining = &remaining[chunk.len()..];
                true
            })
        {
            return Some(shard.declarations().iter().cloned().map(convert).collect());
        }
    }
    if cancelled() {
        return None;
    }
    let extractor = match extractors.entry(work.ticket.key.language) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(DeclExtractor::for_language(work.ticket.key.language)?)
        }
    };
    let text: std::borrow::Cow<'_, str> = work.rope.slice(..).into();
    Some(
        extractor
            .extract(text.as_bytes(), &cancelled)?
            .into_iter()
            .map(convert)
            .collect(),
    )
}

pub(crate) struct Worker {
    sender: mpsc::Sender<Job>,
}

enum Job {
    Outline(Work),
    Server {
        context: strop_lsp::ReplyContext,
        root: std::path::PathBuf,
        location: ResourceLocation,
        rows: Vec<ProtoSymbol>,
    },
}

impl Worker {
    pub fn start(
        events: mpsc::Sender<AnalysisEvent>,
        mut extract: Extract,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("symbol-outline".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let work = match job {
                        Job::Outline(work) => work,
                        Job::Server {
                            context,
                            root,
                            location,
                            rows,
                        } => {
                            let list = super::store::SymbolList::prepare(
                                rows,
                                &root,
                                &location,
                                Some((context.stamp.server, context.encoding)),
                            );
                            if events
                                .send(AnalysisEvent::ServerOutline(context, Box::new(list)))
                                .is_err()
                            {
                                return;
                            }
                            continue;
                        }
                    };
                    let outcome = if work.cancel.load(Ordering::Acquire) {
                        Outcome::Cancelled(CancelReason::Superseded)
                    } else {
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            extract(&work).map(|rows| {
                                super::store::SymbolList::prepare(
                                    rows,
                                    &work.root,
                                    &work.location,
                                    None,
                                )
                            })
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
    pub fn submit(&self, work: Work) -> Result<(), ()> {
        self.sender.send(Job::Outline(work)).map_err(|_| ())
    }

    pub fn prepare_server(
        &self,
        context: strop_lsp::ReplyContext,
        root: std::path::PathBuf,
        location: ResourceLocation,
        rows: Vec<ProtoSymbol>,
    ) -> Result<(), ()> {
        self.sender
            .send(Job::Server {
                context,
                root,
                location,
                rows,
            })
            .map_err(|_| ())
    }
}
