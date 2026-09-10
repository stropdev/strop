//! Request admission and the common launcher. Every admitted request
//! ends in exactly one terminal event (R9) — success, empty, error or
//! cancellation — carrying its ORIGINAL stamp and negotiated encoding
use crate::protocol::*;
use std::path::PathBuf;
use strop_core::id::{BufferRevision, ByteColumn, DocumentId, LineIndex};
use strop_workspace::ResourceLocation;

use super::queue::{WireEnv, WireJob, RETRY_DELAY};
use super::sync;
use super::wire::{self, is_content_modified};
use super::{Client, SwitchSourceHeader};
use crate::convert::hover_text;
use async_lsp::lsp_types as lt;

impl Client {
    /// Admit a request against the current open state without sending
    /// anything: the pure half of the replay tape's `lsp.prepare`.
    /// `Ok` carries the owning stamp and captured input; `Err` means
    /// nothing was sent and nothing will arrive.
    pub fn prepare_request(&self, input: RequestInput) -> Result<PendingRequest, RequestRefusal> {
        let state = self.sync.lock();
        let Some(open) = state.documents.get(&input.path) else {
            return Err(RequestRefusal::NotOpen);
        };
        if open.document != input.document || open.revision != input.revision {
            return Err(RequestRefusal::StaleRevision);
        }
        if state.ready && !self.caps.supports(input.kind) {
            return Err(RequestRefusal::Unsupported);
        }
        let request = match self.next_request.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |n| n.checked_add(1),
        ) {
            Ok(value) => value,
            Err(_) => return Err(RequestRefusal::IdentityExhausted),
        };
        let stamp = RequestStamp {
            request: RequestId::new(request),
            server: self.id,
            document: input.document,
            revision: input.revision,
        };
        Ok(PendingRequest {
            stamp,
            input,
            tab_width: None,
        })
    }

    /// Launch an admitted request: onto the ordered wire when ready,
    /// else the pre-init queue flushed by `finish_initialize`.
    pub fn launch_request(&self, request: PendingRequest) {
        let mut state = self.sync.lock();
        if state.ready {
            self.queue.send(WireJob::Request(request));
        } else {
            state.pending_requests.push(request);
        }
    }

    /// Prepare and launch in one step for callers outside the tape
    /// seam (the probe example).
    pub fn request(&self, input: RequestInput) -> Result<RequestStamp, RequestRefusal> {
        let request = self.prepare_request(input)?;
        let stamp = request.stamp;
        self.launch_request(request);
        Ok(stamp)
    }

    /// Formatting admission: no cursor position — the input records
    /// document identity only, and the tab width rides the admission
    /// record so a replay relaunches the identical payload.
    pub fn format(
        &self,
        document: DocumentId,
        revision: BufferRevision,
        path: PathBuf,
        tab_width: usize,
    ) -> Result<RequestStamp, RequestRefusal> {
        let mut request = self.prepare_request(RequestInput {
            document,
            revision,
            path,
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(0),
            line_text: crate::FrozenLine::from(""),
            kind: RequestKind::Format,
            rename_to: None,
        })?;
        request.tab_width = Some(tab_width);
        let stamp = request.stamp;
        self.launch_request(request);
        Ok(stamp)
    }

    /// Rename admission: the new name is recorded on the input so the
    /// replay tape relaunches against the identical payload.
    pub fn rename(
        &self,
        mut input: RequestInput,
        new_name: &str,
    ) -> Result<RequestStamp, RequestRefusal> {
        input.rename_to = Some(new_name.to_owned());
        self.request(input)
    }

    /// Code-action admission at the input's position.
    pub fn code_actions(&self, input: RequestInput) -> Result<RequestStamp, RequestRefusal> {
        self.request(input)
    }
}

/// Launch one admitted request on the wire worker. Called in admission
/// order, so every earlier frame is already on the wire.
pub(crate) fn launch(env: &WireEnv, request: PendingRequest) {
    let kind = request.input.kind;
    let path = request.input.path.clone();
    let stamp = request.stamp;
    let encoding = env.caps.encoding();
    let context = ReplyContext {
        stamp,
        encoding,
        kind,
    };
    if !env.caps.supports(kind) {
        return note(
            env,
            context,
            format!("{} is not supported by this language server", kind.label()),
        );
    }
    let Some(uri) = env.workspace.uri(&request.input.path) else {
        return note(
            env,
            context,
            "cannot map the document path onto a file URI".into(),
        );
    };
    let Ok(line) = u32::try_from(request.input.line.get()) else {
        return note(env, context, "line is out of protocol range".into());
    };
    let server_col = crate::to_server_col_slice(
        request.input.line_text.as_slice(),
        request.input.byte_col,
        encoding,
    );
    let Ok(character) = u32::try_from(server_col.get()) else {
        return note(env, context, "column is out of protocol range".into());
    };
    let tdp = lt::TextDocumentPositionParams {
        text_document: lt::TextDocumentIdentifier { uri },
        position: lt::Position { line, character },
    };
    let tab_width = request.tab_width;
    let rename_to = request.input.rename_to.clone();
    let handle = env.handle.clone();
    let env = env.clone();
    handle.spawn(async move {
        if !sync::owns(&env, &stamp, &path) {
            return note(
                &env,
                context,
                "cancelled — the document changed or closed".into(),
            );
        }
        match kind {
            RequestKind::Hover => hover(env, tdp, context, path).await,
            RequestKind::Goto => goto(env, tdp, context, path).await,
            RequestKind::Locations(kind) => {
                wire::request_locations(&env, kind, tdp, context, path).await
            }
            RequestKind::SwitchHeader => switch_header(env, tdp, context).await,
            RequestKind::Format => match tab_width {
                Some(tab_width) => format(env, tdp.text_document.uri, context, tab_width).await,
                None => note(
                    &env,
                    context,
                    "format request is missing its tab width".into(),
                ),
            },
            RequestKind::Rename => match rename_to {
                Some(new_name) => rename(env, tdp, context, new_name).await,
                None => note(
                    &env,
                    context,
                    "rename request is missing its new name".into(),
                ),
            },
            RequestKind::CodeAction => code_actions(env, tdp, context).await,
            RequestKind::DocumentSymbols => {
                document_symbols(env, tdp.text_document, context, path).await
            }
        }
    });
}

fn note(env: &WireEnv, context: ReplyContext, text: String) {
    let _ = env.tx.send(LspEvent::Note { context, text });
}

async fn hover(
    env: WireEnv,
    tdp: lt::TextDocumentPositionParams,
    context: ReplyContext,
    path: std::path::PathBuf,
) {
    let params = lt::HoverParams {
        text_document_position_params: tdp,
        work_done_progress_params: Default::default(),
    };
    let mut response = env
        .socket
        .request::<lt::request::HoverRequest>(params.clone())
        .await;
    if is_content_modified(&response) {
        tokio::time::sleep(RETRY_DELAY).await;
        if !sync::owns(&env, &context.stamp, &path) {
            return note(
                &env,
                context,
                "cancelled — the document changed or closed".into(),
            );
        }
        response = env
            .socket
            .request::<lt::request::HoverRequest>(params)
            .await;
    }
    match response {
        Ok(Some(hover)) => {
            let text = hover_text(&hover);
            if text.is_empty() {
                note(&env, context, "no hover information at the cursor".into());
            } else {
                let _ = env.tx.send(LspEvent::HoverText { context, text });
            }
        }
        Ok(None) => note(&env, context, "no hover information at the cursor".into()),
        Err(error) => note(&env, context, format!("hover failed: {error}")),
    }
}

async fn goto(
    env: WireEnv,
    tdp: lt::TextDocumentPositionParams,
    context: ReplyContext,
    path: std::path::PathBuf,
) {
    let params = lt::GotoDefinitionParams {
        text_document_position_params: tdp,
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let mut response = env
        .socket
        .request::<lt::request::GotoDefinition>(params.clone())
        .await;
    if is_content_modified(&response) {
        tokio::time::sleep(RETRY_DELAY).await;
        if !sync::owns(&env, &context.stamp, &path) {
            return note(
                &env,
                context,
                "cancelled — the document changed or closed".into(),
            );
        }
        response = env
            .socket
            .request::<lt::request::GotoDefinition>(params)
            .await;
    }
    match response {
        Ok(Some(response)) => match wire::first_location(response) {
            Some(location) => match wire::to_server_location(location, &env.workspace) {
                Ok(location) => {
                    let _ = env.tx.send(LspEvent::GotoLocation { context, location });
                }
                Err(uri) => note(
                    &env,
                    context,
                    format!(
                        "definition target is not a file on {}: {uri}",
                        env.workspace.label()
                    ),
                ),
            },
            None => note(&env, context, "no definition found".into()),
        },
        Ok(None) => note(&env, context, "no definition found".into()),
        Err(error) => note(&env, context, format!("goto definition failed: {error}")),
    }
}

async fn switch_header(env: WireEnv, tdp: lt::TextDocumentPositionParams, context: ReplyContext) {
    match env
        .socket
        .request::<SwitchSourceHeader>(tdp.text_document)
        .await
    {
        Ok(Some(uri)) => match env.workspace.decode(&uri) {
            Some(path) => {
                let location = ServerLocation {
                    doc: ResourceLocation {
                        filesystem: env.workspace.target(),
                        path,
                    },
                    position: ServerPosition {
                        line: LineIndex::new(0),
                        column: ServerColumn::new(0),
                    },
                };
                let _ = env.tx.send(LspEvent::GotoLocation { context, location });
            }
            None => note(
                &env,
                context,
                format!(
                    "header/source counterpart is not a file on {}: {uri}",
                    env.workspace.label()
                ),
            ),
        },
        Ok(None) => note(&env, context, "no header/source counterpart".into()),
        Err(error) => note(
            &env,
            context,
            format!("switch source/header failed: {error}"),
        ),
    }
}

/// `textDocument/formatting`: the reply's whole-document edits in
/// server-domain positions. A null reply is an explicit empty edit set,
/// like the locations family's empty lists.
async fn format(env: WireEnv, uri: lt::Url, context: ReplyContext, tab_width: usize) {
    let Ok(tab_size) = u32::try_from(tab_width) else {
        return note(&env, context, "tab width is out of protocol range".into());
    };
    let params = lt::DocumentFormattingParams {
        text_document: lt::TextDocumentIdentifier { uri },
        options: lt::FormattingOptions {
            tab_size,
            insert_spaces: true,
            ..Default::default()
        },
        work_done_progress_params: Default::default(),
    };
    match env.socket.request::<lt::request::Formatting>(params).await {
        Ok(edits) => {
            let edits = edits
                .unwrap_or_default()
                .into_iter()
                .map(wire::server_edit)
                .collect();
            let _ = env.tx.send(LspEvent::Edits { context, edits });
        }
        Err(error) => note(&env, context, format!("format failed: {error}")),
    }
}

/// `textDocument/rename`: both `changes` and `documentChanges` decode
/// to per-resource edit groups; resource operations and foreign
/// versions refuse the whole reply with a note — never a partial set.
async fn rename(
    env: WireEnv,
    tdp: lt::TextDocumentPositionParams,
    context: ReplyContext,
    new_name: String,
) {
    let params = lt::RenameParams {
        text_document_position: tdp,
        new_name,
        work_done_progress_params: Default::default(),
    };
    match env.socket.request::<lt::request::Rename>(params).await {
        Ok(Some(edit)) => match wire::workspace_edits(&env, edit) {
            Ok(edits) => {
                let _ = env.tx.send(LspEvent::WorkspaceEdits { context, edits });
            }
            Err(refusal) => note(
                &env,
                context,
                refusal.note_text("rename", &env.workspace.label()),
            ),
        },
        // A null reply is an explicit empty edit set.
        Ok(None) => {
            let _ = env.tx.send(LspEvent::WorkspaceEdits {
                context,
                edits: Vec::new(),
            });
        }
        Err(error) => note(&env, context, format!("rename failed: {error}")),
    }
}

/// `textDocument/codeAction` at a zero-width range with an empty
/// diagnostics context: the actions available at the cursor.
async fn code_actions(env: WireEnv, tdp: lt::TextDocumentPositionParams, context: ReplyContext) {
    let params = lt::CodeActionParams {
        text_document: tdp.text_document,
        range: lt::Range {
            start: tdp.position,
            end: tdp.position,
        },
        context: lt::CodeActionContext {
            diagnostics: Vec::new(),
            only: None,
            trigger_kind: None,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    match env
        .socket
        .request::<lt::request::CodeActionRequest>(params)
        .await
    {
        Ok(Some(items)) => {
            let actions = items
                .into_iter()
                .map(|item| match item {
                    lt::CodeActionOrCommand::Command(command) => ProtoAction {
                        title: command.title,
                        edits: None,
                        has_external_command: true,
                    },
                    lt::CodeActionOrCommand::CodeAction(action) => {
                        // An action whose edit carries file operations,
                        // unverifiable versions or undecodable targets
                        // cannot be applied: it stays listed by title
                        // with `edits: None`, and the editor marks it
                        // inapplicable.
                        let edits = action
                            .edit
                            .and_then(|edit| wire::workspace_edits(&env, edit).ok());
                        ProtoAction {
                            title: action.title,
                            edits,
                            has_external_command: action.command.is_some(),
                        }
                    }
                })
                .collect();
            let _ = env.tx.send(LspEvent::ActionList { context, actions });
        }
        // A null reply is an explicit empty action list.
        Ok(None) => {
            let _ = env.tx.send(LspEvent::ActionList {
                context,
                actions: Vec::new(),
            });
        }
        Err(error) => note(&env, context, format!("code action failed: {error}")),
    }
}

/// `textDocument/documentSymbol`: both reply shapes flatten into
/// [`ProtoSymbol`] rows (0047 §1). Hierarchical trees join ancestors
/// with ` :: ` as the container path; the jump position is the
/// selection range's start (the identifier, not the block).
async fn document_symbols(
    env: WireEnv,
    text_document: lt::TextDocumentIdentifier,
    context: ReplyContext,
    path: std::path::PathBuf,
) {
    let params = lt::DocumentSymbolParams {
        text_document,
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = env
        .socket
        .request::<lt::request::DocumentSymbolRequest>(params)
        .await;
    let location = |line: u32, character: u32| ServerLocation {
        doc: ResourceLocation {
            filesystem: env.workspace.target(),
            path: path.clone(),
        },
        position: ServerPosition {
            line: LineIndex::new(line as usize),
            column: ServerColumn::new(character as usize),
        },
    };
    match response {
        Ok(Some(lt::DocumentSymbolResponse::Flat(informations))) => {
            let symbols = informations
                .into_iter()
                .filter_map(|info| {
                    let location = wire::to_server_location(info.location, &env.workspace).ok()?;
                    Some(ProtoSymbol {
                        name: info.name,
                        container: info.container_name.unwrap_or_default(),
                        kind: symbol_kind_label(info.kind).into(),
                        location,
                    })
                })
                .collect::<Vec<_>>();
            let _ = env.tx.send(LspEvent::Symbols { context, symbols });
        }
        Ok(Some(lt::DocumentSymbolResponse::Nested(tree))) => {
            let mut symbols = Vec::new();
            flatten_symbols(&tree, "", &mut |name, container, kind, line, character| {
                symbols.push(ProtoSymbol {
                    name,
                    container,
                    kind,
                    location: location(line, character),
                });
            });
            let _ = env.tx.send(LspEvent::Symbols { context, symbols });
        }
        Ok(None) => note(&env, context, "no symbols in this document".into()),
        Err(error) => note(&env, context, format!("document symbols failed: {error}")),
    }
}

fn flatten_symbols(
    symbols: &[lt::DocumentSymbol],
    ancestors: &str,
    emit: &mut impl FnMut(String, String, String, u32, u32),
) {
    for symbol in symbols {
        let container = if ancestors.is_empty() {
            String::new()
        } else {
            ancestors.to_string()
        };
        let path = if ancestors.is_empty() {
            symbol.name.clone()
        } else {
            format!("{ancestors} :: {}", symbol.name)
        };
        emit(
            symbol.name.clone(),
            container,
            symbol_kind_label(symbol.kind).into(),
            symbol.selection_range.start.line,
            symbol.selection_range.start.character,
        );
        if let Some(children) = symbol.children.as_deref() {
            flatten_symbols(children, &path, emit);
        }
    }
}

/// SymbolKind's LSP name for the picker row.
fn symbol_kind_label(kind: lt::SymbolKind) -> &'static str {
    match kind {
        lt::SymbolKind::FILE => "File",
        lt::SymbolKind::MODULE => "Module",
        lt::SymbolKind::NAMESPACE => "Namespace",
        lt::SymbolKind::PACKAGE => "Package",
        lt::SymbolKind::CLASS => "Class",
        lt::SymbolKind::METHOD => "Method",
        lt::SymbolKind::PROPERTY => "Property",
        lt::SymbolKind::FIELD => "Field",
        lt::SymbolKind::CONSTRUCTOR => "Constructor",
        lt::SymbolKind::ENUM => "Enum",
        lt::SymbolKind::INTERFACE => "Interface",
        lt::SymbolKind::FUNCTION => "Function",
        lt::SymbolKind::VARIABLE => "Variable",
        lt::SymbolKind::CONSTANT => "Constant",
        lt::SymbolKind::STRING => "String",
        lt::SymbolKind::NUMBER => "Number",
        lt::SymbolKind::BOOLEAN => "Boolean",
        lt::SymbolKind::ARRAY => "Array",
        lt::SymbolKind::OBJECT => "Object",
        lt::SymbolKind::KEY => "Key",
        lt::SymbolKind::NULL => "Null",
        lt::SymbolKind::ENUM_MEMBER => "EnumMember",
        lt::SymbolKind::STRUCT => "Struct",
        lt::SymbolKind::EVENT => "Event",
        lt::SymbolKind::OPERATOR => "Operator",
        lt::SymbolKind::TYPE_PARAMETER => "TypeParameter",
        _ => "Symbol",
    }
}
