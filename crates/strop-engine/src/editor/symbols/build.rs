//! Building a document's symbol list: the server request or the outline
//! job, their replies, and the one place a result lands ([`settle`]).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{Completion, FailureKind, Outcome, Ticket};
use strop_lsp::{ProtoSymbol, ReplyContext, RequestKind};
use strop_syntax::languages::LanguageId;

use super::outline::{OutlineKey, Work};
use super::store::{self, Build, BuildId, Current, Delivery, Producer, SymbolList};
use super::{current_of, display_root, matches_document, outline_language};
use crate::editor::{trace, Editor};

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

pub(super) fn record_finish(result: &'static str, symbols: usize) {
    strop_trace::record_with(
        strop_trace::EventKind::JobFinished,
        || serde_json::json!({"service": "symbols", "result": result, "symbols": symbols}),
    );
}

/// Ask the current document's server. A background build is silent: a
/// refusal changes no message.
pub(super) fn start_server(editor: &mut Editor, open: bool) {
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
        workspace: display_root(editor, stamp.document).1,
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

pub(super) fn start_outline(editor: &mut Editor, document: DocumentId, open: bool) {
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
    let workspace = display_root(editor, document).1;
    let ticket = Ticket {
        request,
        key: OutlineKey {
            document,
            revision,
            language,
            workspace,
        },
    };
    let build = Build {
        id: BuildId::Outline(request),
        document,
        revision,
        producer: Producer::Outline,
        workspace,
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
    let shard = location.local_path().and_then(|path| {
        let worker = editor.picker_source.as_ref()?;
        let root = worker.inventory_root(&editor.cwd)?;
        worker.disk_shard(&root, path)
    });
    let root = display_root(editor, document).0.to_owned();
    let work = Work {
        ticket,
        rope,
        root,
        shard,
        location,
        cancel,
    };
    let events = editor.analysis.tx.clone();
    let ticket = work.ticket.clone();
    let submitted = match editor.analysis.symbols.worker(&events) {
        Ok(worker) => worker.submit(work).map_err(|()| None),
        Err(error) => Err(Some(error.to_string())),
    };
    if let Err(error) = submitted {
        let message = error.unwrap_or_else(|| "the outline thread stopped".into());
        failed(editor, ticket, FailureKind::Disconnected, message);
    }
}

/// An outline job ended.
pub(crate) fn outline_done(editor: &mut Editor, completion: Completion<OutlineKey, SymbolList>) {
    let id = BuildId::Outline(completion.ticket.request);
    use strop_kernel::admission as a;
    let key = &completion.ticket.key;
    // Native display root and frozen live document. Syntax extraction
    // has no child binding, project declaration or scan generation.
    let stamp = |workspace: store::Workspace,
                 document: DocumentId,
                 revision: BufferRevision,
                 language: LanguageId,
                 query| {
        let (root, root_incarnation) = workspace.map_or((0, 0), |(id, incarnation)| {
            (
                ((id.index() as u64) << 32) | u64::from(id.generation()),
                incarnation,
            )
        });
        a::AnswerStamp {
            resource: a::Resource {
                root,
                root_incarnation,
                path: document.index() as u64,
                path_incarnation: document.generation().into(),
            },
            content: revision.get(),
            language: language as u64,
            schema: 2, // DeclarationSchema::ByteRangesV2, converted to UTF-8 positions
            query,
            child: 0,
            declaration: 0,
            scan: 0,
        }
    };
    let answer = a::Answer {
        stamp: stamp(
            key.workspace,
            key.document,
            key.revision,
            key.language,
            completion.ticket.request.get(),
        ),
        origin: a::Origin::Buffer,
        producer: a::Producer::Syntax(key.language as u64),
    };
    let current = current_of(editor, key.document).filter(|current| {
        let Some(build) = editor.analysis.symbols.store.building() else {
            return false;
        };
        let BuildId::Outline(request) = build.id else {
            return false;
        };
        let Some(language) = outline_language(editor, current.document) else {
            return false;
        };
        let target = a::Target {
            stamp: stamp(
                current.workspace,
                build.document,
                current.revision,
                language,
                request.get(),
            ),
            surface: a::Surface::BufferPending,
            designated: a::Designation::Syntax(language as u64),
        };
        key.workspace == current.workspace
            && current.producer == Some(Producer::Outline)
            && a::admit(Some(&target), &answer)
    });
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
    let stamp = context.stamp;
    let id = BuildId::Server(stamp);
    let current = current_of(editor, stamp.document);
    let Some(rows) = symbols else {
        settle(editor, id, current, None, note);
        return;
    };
    if context.kind != RequestKind::DocumentSymbols
        || editor.analysis.symbols.store.building().map(|b| b.id) != Some(id)
    {
        trace::services::rejected("symbols", "unowned server reply");
        return;
    }
    if !current.is_some_and(|c| {
        c.document == stamp.document
            && c.revision == stamp.revision
            && c.producer == Some(Producer::Server(stamp.server))
    }) {
        settle(editor, id, current, None, None);
        return;
    }
    let Some(binding) = editor.lsp_state.bindings.get(&stamp.document) else {
        return;
    };
    let root = binding.root.clone();
    let location = strop_workspace::ResourceLocation {
        filesystem: binding.target.clone(),
        path: binding.path.clone(),
    };
    match editor.tape.request("symbols.prepare_server", context) {
        Ok(false) => return,
        Err(error) => return settle(editor, id, current, None, Some(error.to_string())),
        Ok(true) => {}
    }
    let events = editor.analysis.tx.clone();
    let result = editor.analysis.symbols.worker(&events).and_then(|worker| {
        worker
            .prepare_server(*context, root, location, rows)
            .map_err(|()| std::io::Error::other("the symbol worker stopped"))
    });
    if let Err(error) = result {
        settle(editor, id, current, None, Some(error.to_string()));
    }
}

pub(crate) fn server_prepared(editor: &mut Editor, context: ReplyContext, list: SymbolList) {
    let current = current_of(editor, context.stamp.document);
    settle(
        editor,
        BuildId::Server(context.stamp),
        current,
        Some(list),
        None,
    );
}

/// The one place a build's result lands (0099 §5.4).
fn settle(
    editor: &mut Editor,
    id: BuildId,
    current: Option<Current>,
    symbols: Option<SymbolList>,
    failure: Option<String>,
) {
    let outline = matches!(id, BuildId::Outline(_));
    let current = current.filter(|c| {
        symbols.as_ref().is_none_or(|list| {
            let (root, _) = display_root(editor, c.document);
            root == list.root.path && matches_document(editor, c.document, list)
        })
    });
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
            record_finish("fresh", symbols.rows.len());
            if build.open && !editor.docs.is_empty() && editor.current() == build.document {
                show(editor, build.producer, symbols);
            }
        }
    }
}

/// Open the picker on one list.
pub(super) fn show(editor: &mut Editor, producer: Producer, symbols: SymbolList) {
    if symbols.rows.is_empty() {
        editor.message = "no symbols in this document".into();
        return;
    }
    let mut picker = strop_picker::Picker::new(strop_picker::Kind::Symbols, Vec::new(), false);
    picker.items = symbols.catalog;
    picker.rows = strop_picker::Rows::Prepared(symbols.listing);
    let mut glue = crate::editor::PickerGlue::diagnostics(picker);
    glue.document_symbols = current_of(editor, editor.current());
    glue.ranked_query = Some(String::new());
    editor.set_picker(glue);
    if producer == Producer::Outline {
        editor.message = "symbols from the syntax tree: no language server serves this file".into();
    }
}
