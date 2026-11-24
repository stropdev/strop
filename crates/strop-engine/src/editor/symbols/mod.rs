//! Document symbols (`Space s`, `:symbols`), kept per document, source and
//! buffer revision and fetched ahead of the key (0099 §4.1).
//!
//! Two sources, never mixed in one list: the language server that serves
//! the file, or — when none does — an outline from the linked tree-sitter
//! grammar. Either way the list is a build (`build.rs`): started there,
//! delivered on the event loop, accepted by [`store::SymbolStore::deliver`]
//! and only then shown or kept. Nothing waits for one (0099 §5): a press with no kept
//! list starts (or adopts) a build and returns.
//!
//! The breadcrumb (0107) is the same list read at the cursor: a scope
//! index derived from the kept list of the current document, carried
//! through edits until the next exact list replaces it (`scope.rs`).
//!
//! A workspace-symbol row's jump carries its evidence, checked against the
//! kept list before the open lands (`jump.rs`).
//!
//! Owner: `Editor::analysis.symbols`. The functions here, in `build.rs` and
//! in `jump.rs` take the editor; they are this component's whole surface.

pub(crate) mod build;
pub(crate) mod jump;
pub(crate) mod outline;
pub(crate) mod scope;
pub(crate) mod store;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use strop_core::id::DocumentId;
use strop_core::worker::CancelReason;
use strop_lsp::ServerId;
use strop_syntax::languages::LanguageId;
use strop_syntax::symbols::DeclExtractor;

use super::analysis::AnalysisEvent;
use super::{Editor, Mode};
use build::{record_finish, show, start_outline, start_server};
use scope::Scopes;
use store::{Current, Producer, SymbolList, SymbolStore};

pub(crate) struct SymbolState {
    pub(crate) store: SymbolStore,
    /// A ready server that explicitly lacks documentSymbol cannot own Space s.
    pub(crate) incapable: std::collections::HashSet<ServerId>,
    /// Distinct across picker instances too: closing and reopening Space S
    /// must not resurrect a queued reply.
    pub(crate) workspace_generation: u64,
    pub(crate) jump: Option<jump::WorkspaceJump>,
    worker: Option<outline::Worker>,
    /// The outline build's cancel flag, while one is in flight.
    cancel: Option<Arc<AtomicBool>>,
    /// The document, revision and source `warm` last decided for. An equal
    /// state needs no second decision: the after-event check is O(1).
    settled: Option<Current>,
    /// How the outline thread extracts; taken when the thread starts.
    extract: Option<outline::Extract>,
    /// The breadcrumb's index for the current document (0107 §6.2).
    pub(crate) scope: Option<Scopes>,
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
            incapable: Default::default(),
            workspace_generation: 0,
            jump: None,
            worker: None,
            cancel: None,
            settled: None,
            extract: Some(extract),
            scope: None,
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
    /// Its (cancelled) result still arrives and is dropped as stale. The
    /// breadcrumb's index follows the edit, or is dropped when it cannot.
    pub(crate) fn edited(&mut self, document: DocumentId, changes: &[strop_core::Change]) {
        let follows = match self.scope.as_mut() {
            Some(scope) if scope.document == document => scope.apply(changes),
            _ => true,
        };
        if !follows {
            self.scope = None;
        }
        let outline = self
            .store
            .building()
            .is_some_and(|build| build.document == document && build.producer == Producer::Outline);
        if outline {
            self.cancel_outline();
        }
    }

    pub(crate) fn closed(&mut self, document: DocumentId) {
        self.edited(document, &[]);
        if self
            .scope
            .as_ref()
            .is_some_and(|scope| scope.document == document)
        {
            self.scope = None;
        }
        self.store.close(document);
        if self
            .settled
            .is_some_and(|settled| settled.document == document)
        {
            self.settled = None;
        }
    }

    pub(crate) fn server_gone(&mut self, server: ServerId) {
        self.incapable.remove(&server);
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
    let doc = editor.docs.get(document)?;
    if doc.buf.path.is_none() && doc.remote_metadata().is_none() {
        return None;
    }
    let path = doc.syntax_path()?;
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
        if ready && !editor.analysis.symbols.incapable.contains(&binding.server) {
            return Some(Producer::Server(binding.server));
        }
    }
    outline_language(editor, document).map(|_| Producer::Outline)
}

/// Display paths are relative to the serving project, or to the pinned root.
/// There is no native path clone on a cache probe.
fn display_root(editor: &Editor, document: DocumentId) -> (&std::path::Path, store::Workspace) {
    if let Some(binding) = editor.lsp_state.bindings.get(&document) {
        let workspace = editor
            .workspaces
            .resolve_path(&binding.target, &binding.root);
        return (
            &binding.root,
            workspace.map(|(id, ctx)| (id, ctx.incarnation)),
        );
    }
    if let Some(remote) = editor
        .docs
        .get(document)
        .and_then(|doc| doc.remote_metadata())
    {
        let path = remote.file.path();
        let fs = strop_workspace::Filesystem::Remote(remote.file.endpoint().clone());
        return match editor.workspaces.resolve_path(&fs, path) {
            Some((id, ctx)) => (&ctx.root, Some((id, ctx.incarnation))),
            None => (path.parent().unwrap_or(path), None),
        };
    }
    let workspace = editor
        .workspaces
        .resolve_path(&strop_workspace::Filesystem::Local, &editor.cwd);
    (
        &editor.cwd,
        workspace.map(|(id, ctx)| (id, ctx.incarnation)),
    )
}

fn matches_display(editor: &Editor, document: DocumentId, list: &SymbolList) -> bool {
    let (root, workspace) = display_root(editor, document);
    root == list.root.path
        && workspace == list.workspace
        && matches_document(editor, document, list)
}

fn matches_document(editor: &Editor, document: DocumentId, list: &SymbolList) -> bool {
    let Some(doc) = editor.docs.get(document) else {
        return false;
    };
    if let Some(remote) = doc.remote_metadata() {
        return matches!(&list.location.filesystem, strop_workspace::Filesystem::Remote(e) if e == remote.file.endpoint())
            && remote.file.path() == list.location.path;
    }
    let Some(path) = doc.buf.path.as_deref() else {
        return false;
    };
    list.location.local_path().is_some_and(|location| {
        path == location || (path.is_relative() && location.strip_prefix(&editor.cwd) == Ok(path))
    })
}

pub(crate) fn current_of(editor: &Editor, document: DocumentId) -> Option<Current> {
    let doc = editor.docs.get(document)?;
    Some(Current {
        revision: doc.buf.revision(),
        document,
        producer: source(editor, document),
        workspace: display_root(editor, document).1,
    })
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
    let state = Current {
        document,
        revision,
        producer: source,
        workspace: display_root(editor, document).1,
    };
    if editor.analysis.symbols.settled == Some(state) {
        return;
    }
    editor.analysis.symbols.settled = Some(state);
    let Some(producer) = source else {
        return;
    };
    let kept = editor
        .analysis
        .symbols
        .store
        .lookup(document, producer, revision);
    if kept
        .as_ref()
        .is_some_and(|list| matches_display(editor, document, list))
    {
        return;
    }
    match producer {
        Producer::Outline => start_outline(editor, document, false),
        Producer::Server(_) => start_server(editor, false),
    }
}

/// After every event, right after [`warm`] (0107 §6.2): keep the
/// breadcrumb's index on the current document's kept list. O(1) when the
/// document, revision and source are what the exact index was built for;
/// otherwise drop an index for another document or source, and build one
/// when the store holds the list for the text as it is. Starts no job.
pub(crate) fn follow(editor: &mut Editor) {
    if let Some(owner) = editor.picker.as_ref().and_then(|g| g.document_symbols) {
        if editor.docs.is_empty()
            || editor.current() != owner.document
            || current_of(editor, owner.document) != Some(owner)
        {
            editor.close_picker();
        }
    }
    if editor.docs.is_empty() {
        editor.analysis.symbols.scope = None;
        return;
    }
    let document = editor.current();
    let revision = editor.buf().revision();
    let Some(producer) = source(editor, document) else {
        editor.analysis.symbols.scope = None;
        return;
    };
    let symbols = &mut editor.analysis.symbols;
    if let Some(scope) = &symbols.scope {
        if (scope.document, scope.producer) != (document, producer) {
            symbols.scope = None;
        } else if scope.revision == revision && scope.exact {
            return;
        }
    }
    let list = symbols.store.lookup(document, producer, revision);
    match (list, editor.docs.get(document)) {
        (Some(list), Some(doc)) => {
            let rope = doc.buf.text();
            symbols.scope = Some(Scopes::build(document, producer, revision, list.rows, rope));
        }
        // carried through every edit since its list: it still describes
        // this text; anything else is a revision it never saw
        _ => {
            if symbols
                .scope
                .as_ref()
                .is_some_and(|scope| scope.revision != revision)
            {
                symbols.scope = None;
            }
        }
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
        if let Some(symbols) = kept.filter(|list| matches_display(editor, document, list)) {
            record_finish("kept", symbols.rows.len());
            show(editor, producer, symbols);
            return;
        }
        // Adopt the background build that is this very request. A build a
        // press is already waiting for is not adopted twice: pressing
        // again asks again, as it always did.
        let workspace = display_root(editor, document).1;
        if let Some(build) = editor.analysis.symbols.store.building_mut() {
            if !build.open
                && build.workspace == workspace
                && (build.document, build.revision, build.producer)
                    == (document, revision, producer)
            {
                build.open = true;
                return;
            }
        }
    }
    // A user request never queues behind a build.
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

/// A fixture's kind by its LSP name, through the one kind table.
#[cfg(test)]
pub(crate) fn lsp_kind(name: &str) -> strop_core::symbol_kind::SymbolKind {
    strop_core::symbol_kind::KINDS
        .iter()
        .find(|spec| spec.lsp.contains(&name))
        .map_or(strop_core::symbol_kind::SymbolKind::Other, |spec| spec.kind)
}

#[cfg(test)]
mod race_tests;
#[cfg(test)]
mod scope_tests;
#[cfg(test)]
mod tests;
