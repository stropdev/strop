//! Document symbols (`Space s`, `:symbols`), kept per document, source and
//! buffer revision and fetched ahead of the key (0099 §4.1).
//!
//! Two sources, never mixed in one list: the language server that serves
//! the file, or — when none does — an outline from the linked tree-sitter
//! grammar. Either way the list is a build: started here, delivered on the
//! event loop, accepted by [`store::SymbolStore::deliver`] and only then
//! shown or kept. Nothing waits for one (0099 §5): a press with no kept
//! list starts (or adopts) a build and returns.
//!
//! Owner: `Editor::analysis.symbols`. The functions here take the editor;
//! they are this component's whole surface.

pub(crate) mod outline;
pub(crate) mod store;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};
use strop_lsp::{ProtoSymbol, ReplyContext, RequestKind, ServerId};
use strop_syntax::languages::LanguageId;
use strop_syntax::symbols::DeclExtractor;

use super::analysis::AnalysisEvent;
use super::{trace, Editor, Mode};
use outline::{OutlineKey, Work};
use store::{Build, BuildId, Current, Delivery, Producer, SymbolStore};

pub(crate) struct SymbolState {
    pub(crate) store: SymbolStore,
    worker: Option<outline::Worker>,
    /// The outline build's cancel flag, while one is in flight.
    cancel: Option<Arc<AtomicBool>>,
    /// The document, revision and source `warm` last decided for. An equal
    /// state needs no second decision: the after-event check is O(1).
    settled: Option<(DocumentId, BufferRevision, Option<Producer>)>,
    /// How the outline thread extracts; taken when the thread starts.
    extract: Option<outline::Extract>,
}

impl Default for SymbolState {
    fn default() -> Self {
        let mut extractors = HashMap::new();
        Self::with_extract(Box::new(move |work| {
            outline::extract(&mut extractors, work)
        }))
    }
}

impl SymbolState {
    pub(crate) fn with_extract(extract: outline::Extract) -> Self {
        Self {
            store: SymbolStore::default(),
            worker: None,
            cancel: None,
            settled: None,
            extract: Some(extract),
        }
    }

    pub(crate) fn pending(&self) -> bool {
        self.store.building().is_some()
    }

    fn cancel_outline(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
    }

    /// The text changed: an outline being built for it is unwanted work.
    /// Its (cancelled) result still arrives and is dropped as stale.
    pub(crate) fn edited(&mut self, document: DocumentId) {
        let outline = self
            .store
            .building()
            .is_some_and(|build| build.document == document && build.producer == Producer::Outline);
        if outline {
            self.cancel_outline();
        }
    }

    pub(crate) fn closed(&mut self, document: DocumentId) {
        self.edited(document);
        self.store.close(document);
        if self
            .settled
            .is_some_and(|(settled, _, _)| settled == document)
        {
            self.settled = None;
        }
    }

    pub(crate) fn server_gone(&mut self, server: ServerId) {
        self.store.retire_server(server);
    }

    /// Shutdown: no build is waited for, and the thread ends with its
    /// channel.
    pub(crate) fn stop(&mut self) {
        self.cancel_outline();
        self.store.abandon();
        self.worker = None;
    }

    fn worker(
        &mut self,
        events: &mpsc::Sender<AnalysisEvent>,
    ) -> std::io::Result<&outline::Worker> {
        if self.worker.is_none() {
            let extract = self
                .extract
                .take()
                .ok_or_else(|| std::io::Error::other("the outline thread stopped"))?;
            self.worker = Some(outline::Worker::start(events.clone(), extract)?);
        }
        self.worker
            .as_ref()
            .ok_or_else(|| std::io::Error::other("the outline thread stopped"))
    }
}

/// The grammar an outline of `document` would use: a file-backed buffer
/// whose language has a symbols query. A table lookup.
fn outline_language(editor: &Editor, document: DocumentId) -> Option<LanguageId> {
    editor.lsp_document_location(document)?;
    let path = editor.docs.get(document)?.syntax_path()?;
    let spec = strop_syntax::languages::detect(path, None)?;
    DeclExtractor::supports(spec.id).then_some(spec.id)
}

/// Who answers for `document` now: the ready server it is open on, else
/// the outline, else nobody.
fn source(editor: &Editor, document: DocumentId) -> Option<Producer> {
    if let Some(binding) = editor.lsp_state.bindings.get(&document) {
        let ready = editor
            .lsp_servers
            .iter()
            .any(|server| server.id == binding.server && server.ready);
        if ready {
            return Some(Producer::Server(binding.server));
        }
    }
    outline_language(editor, document).map(|_| Producer::Outline)
}

fn current_of(editor: &Editor, document: DocumentId) -> Option<Current> {
    let doc = editor.docs.get(document)?;
    Some(Current {
        revision: doc.buf.revision(),
        server: editor
            .lsp_state
            .bindings
            .get(&document)
            .map(|binding| binding.server),
    })
}

fn record_start(build: &Build) {
    strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
        serde_json::json!({
            "service": "symbols",
            "request": match build.producer {
                Producer::Outline => "outline",
                Producer::Server(_) => "server",
            },
            "document": {"slot": build.document.index(), "generation": build.document.generation()},
            "revision": build.revision.get(),
            "open": build.open,
        })
    });
}

fn record_finish(result: &'static str, symbols: usize) {
    strop_trace::record_with(
        strop_trace::EventKind::JobFinished,
        || serde_json::json!({"service": "symbols", "result": result, "symbols": symbols}),
    );
}

/// After every event (0099 §4.1): when the current document has no kept
/// list at its revision and nothing is being built, start one build.
/// O(1) when nothing changed; never in Insert mode, so typing does not
/// start a build per character.
pub(crate) fn warm(editor: &mut Editor) {
    if editor.finishing || editor.docs.is_empty() || editor.mode == Mode::Insert {
        return;
    }
    if editor.analysis.symbols.store.building().is_some() {
        return;
    }
    let document = editor.current();
    let revision = editor.buf().revision();
    let source = source(editor, document);
    let state = (document, revision, source);
    if editor.analysis.symbols.settled == Some(state) {
        return;
    }
    editor.analysis.symbols.settled = Some(state);
    let Some(producer) = source else {
        return;
    };
    if editor
        .analysis
        .symbols
        .store
        .has(document, producer, revision)
    {
        return;
    }
    match producer {
        Producer::Outline => start_outline(editor, document, false),
        Producer::Server(_) => start_server(editor, false),
    }
}

/// `Space s` / `:symbols`. A kept list opens now. Otherwise the press
/// adopts the build already running for this exact state, or starts one —
/// ahead of any background build — and the picker opens when it lands.
pub(crate) fn open(editor: &mut Editor) {
    let document = editor.current();
    let revision = editor.buf().revision();
    let source = source(editor, document);
    if let Some(producer) = source {
        let kept = editor
            .analysis
            .symbols
            .store
            .lookup(document, producer, revision);
        if let Some(symbols) = kept {
            record_finish("kept", symbols.len());
            show(editor, producer, symbols);
            return;
        }
        if let Some(build) = editor.analysis.symbols.store.building_mut() {
            if (build.document, build.revision, build.producer) == (document, revision, producer) {
                build.open = true;
                return;
            }
        }
    }
    // A user request never queues behind a background build.
    editor.analysis.symbols.cancel_outline();
    editor.analysis.symbols.store.abandon();
    match source {
        Some(Producer::Outline) => start_outline(editor, document, true),
        // A server serves the file, or nothing does: the request path
        // words every refusal.
        _ => {
            editor.lsp_state.navigation = None;
            editor.cancel_open(CancelReason::Superseded);
            start_server(editor, true);
        }
    }
}

/// Ask the current document's server. A background build is silent: a
/// refusal changes no message.
fn start_server(editor: &mut Editor, open: bool) {
    let silent = (!open).then(|| editor.message.clone());
    let prepared = editor.lsp_prepare(RequestKind::DocumentSymbols, None);
    if let Some(message) = silent.clone() {
        editor.message = message;
    }
    let Some((server, prepared)) = prepared else {
        return;
    };
    let stamp = prepared.stamp;
    let build = Build {
        id: BuildId::Server(stamp),
        document: stamp.document,
        revision: stamp.revision,
        producer: Producer::Server(server),
        open,
    };
    // Own the reply before the request leaves: a replayed reply is
    // validated against exactly this build.
    editor.analysis.symbols.store.begin(build);
    record_start(&build);
    if let Err(error) = editor.lsp_launch(server, prepared) {
        editor.analysis.symbols.store.abandon();
        if silent.is_none() {
            editor.message = error;
        }
    }
}

fn start_outline(editor: &mut Editor, document: DocumentId, open: bool) {
    let (Some(language), Some(location), Some(doc)) = (
        outline_language(editor, document),
        editor.lsp_document_location(document),
        editor.docs.get(document),
    ) else {
        return;
    };
    let revision = doc.buf.revision();
    let rope = doc.buf.snapshot();
    let request = match editor.worker_ids.allocate() {
        Ok(request) => request,
        Err(error) => {
            if open {
                editor.message = error.message;
            }
            return;
        }
    };
    let ticket = Ticket {
        request,
        key: OutlineKey { document, revision },
    };
    let build = Build {
        id: BuildId::Outline(request),
        document,
        revision,
        producer: Producer::Outline,
        open,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    editor.analysis.symbols.store.begin(build);
    editor.analysis.symbols.cancel = Some(cancel.clone());
    record_start(&build);
    let failed = |editor: &mut Editor, ticket, kind, message: String| {
        outline_done(
            editor,
            Completion {
                ticket,
                outcome: Outcome::failed(kind, message),
            },
        );
    };
    match editor.tape.request("symbols.outline", &ticket) {
        Ok(true) => {}
        // replay: the build is owned, its recorded result is injected
        Ok(false) => return,
        Err(error) => return failed(editor, ticket, FailureKind::Protocol, error.to_string()),
    }
    let work = Work {
        ticket,
        rope,
        language,
        location,
        cancel,
    };
    let events = editor.analysis.tx.clone();
    let submitted = match editor.analysis.symbols.worker(&events) {
        Ok(worker) => worker.submit(work).map_err(|work| (work.ticket, None)),
        Err(error) => Err((work.ticket, Some(error.to_string()))),
    };
    if let Err((ticket, error)) = submitted {
        let message = error.unwrap_or_else(|| "the outline thread stopped".into());
        failed(editor, ticket, FailureKind::Disconnected, message);
    }
}

/// An outline job ended.
pub(crate) fn outline_done(
    editor: &mut Editor,
    completion: Completion<OutlineKey, Vec<ProtoSymbol>>,
) {
    let id = BuildId::Outline(completion.ticket.request);
    let current = current_of(editor, completion.ticket.key.document);
    let (symbols, failure) = match completion.outcome {
        Outcome::Success(symbols) => (Some(symbols), None),
        Outcome::Failed { failure, .. } => (None, Some(failure.message)),
        Outcome::Cancelled(_) => (None, None),
    };
    settle(editor, id, current, symbols, failure);
}

/// A server answered a `documentSymbol` request, with symbols or with a
/// note (an error, or "no symbols").
pub(crate) fn server_reply(
    editor: &mut Editor,
    context: &ReplyContext,
    symbols: Option<Vec<ProtoSymbol>>,
    note: Option<String>,
) {
    let current = current_of(editor, context.stamp.document);
    settle(
        editor,
        BuildId::Server(context.stamp),
        current,
        symbols,
        note,
    );
}

/// The one place a build's result lands (0099 §5.4).
fn settle(
    editor: &mut Editor,
    id: BuildId,
    current: Option<Current>,
    symbols: Option<Vec<ProtoSymbol>>,
    failure: Option<String>,
) {
    let outline = matches!(id, BuildId::Outline(_));
    let delivery = editor.analysis.symbols.store.deliver(id, current, symbols);
    if outline && !matches!(delivery, Delivery::Unowned) {
        editor.analysis.symbols.cancel = None;
    }
    match delivery {
        Delivery::Unowned => trace::services::rejected("symbols", "build superseded"),
        Delivery::Stale(build) => {
            trace::services::rejected("symbols", "document, revision or server changed");
            record_finish("stale", 0);
            if build.open {
                editor.message = "document symbols: the buffer changed — press again".into();
            }
        }
        Delivery::Failed(build) => {
            record_finish("failed", 0);
            if build.open {
                editor.message = failure.unwrap_or_else(|| "document symbols cancelled".into());
            }
        }
        Delivery::Fresh(build, symbols) => {
            record_finish("fresh", symbols.len());
            if build.open && !editor.docs.is_empty() && editor.current() == build.document {
                show(editor, build.producer, symbols);
            }
        }
    }
}

/// Open the picker on one list.
fn show(editor: &mut Editor, producer: Producer, symbols: Arc<Vec<ProtoSymbol>>) {
    if symbols.is_empty() {
        editor.message = "no symbols in this document".into();
        return;
    }
    // The picker has no query yet: the first rows are the whole list.
    // `documentSymbol` carries no query, so the list is kept and
    // narrowed locally as the field changes (0063 §3).
    let root = editor
        .lsp_state
        .bindings
        .get(&editor.current())
        .map(|binding| binding.root.clone())
        .unwrap_or_else(|| editor.cwd.clone());
    let items = super::lsp::document_symbol_items(&symbols, None, &root);
    editor.open_picker(strop_picker::Kind::Symbols);
    if let Some(glue) = editor.picker.as_mut() {
        glue.symbols_candidates = symbols;
        glue.picker.append(items);
    }
    // Items landed after the first (empty) ranking: rank again.
    editor.request_picker_ranking();
    if producer == Producer::Outline {
        editor.message = "symbols from the syntax tree: no language server serves this file".into();
    }
}

#[cfg(test)]
mod race_tests;
#[cfg(test)]
mod tests;
