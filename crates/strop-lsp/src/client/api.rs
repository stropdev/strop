//! Request admission and the common launcher. Every admitted request
//! ends in exactly one terminal event (R9) — success, empty, error or
//! cancellation — carrying its ORIGINAL stamp and negotiated encoding
use crate::protocol::*;
use strop_core::id::LineIndex;
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
        Ok(PendingRequest { stamp, input })
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
