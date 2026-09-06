//! client/spawn.rs — server process spawn, the initialize
//! handshake, and the runtime mainloop wiring.

use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc::Sender;
use tokio::process::Command;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use async_lsp::lsp_types::notification::{DidOpenTextDocument, PublishDiagnostics};
use async_lsp::lsp_types::request::GotoDefinition;
use async_lsp::lsp_types::{
    DidOpenTextDocumentParams, GotoDefinitionParams, InitializeParams, InitializedParams, Position,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Url,
    WorkDoneProgressParams,
};
use async_lsp::router::Router;

use crate::convert::{diag_from_lsp, hover_text};

use crate::caps::ServerCaps;
use crate::protocol::*;
use crate::registry;

use super::trace_io::{Direction, Observed};
use super::wire::request_locations;
use super::{Client, SwitchSourceHeader};

/// Client-side state for the router.
struct ClientState {
    tx: Sender<LspEvent>,
}

impl Client {
    /// Spawn the server described by `spec` at `root`. None when the
    /// command can't be probed (config layers borrow from process-
    /// lifetime tables, hence 'static). The spec's `init_options` (helix
    /// `[language-server.NAME.config]`, 0012) ride on initialize.
    pub fn spawn(
        spec: &registry::ServerSpec<'static>,
        root: &Path,
        tx: Sender<LspEvent>,
    ) -> Option<Self> {
        // probe first with std (no reactor): the server must exist at all
        std::process::Command::new(spec.command)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let root_uri = Url::from_file_path(root).ok()?;
        let (mainloop, socket) = async_lsp::MainLoop::new_client(|_server| {
            let mut router = Router::new(ClientState { tx: tx.clone() });
            router.notification::<PublishDiagnostics>(|st, params| {
                let path = params.uri.to_file_path().unwrap_or_default();
                let diags = params.diagnostics.iter().map(diag_from_lsp).collect();
                let version = params.version;
                let _ = st.tx.send(LspEvent::Diagnostics {
                    path,
                    diags,
                    version,
                });
                std::ops::ControlFlow::Continue(())
            });
            router
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        let handle = rt.handle().clone();
        // filled by the init task below; gates hover/goto requests
        let self_caps = ServerCaps::default();

        // all tokio work (child spawn included) lives on the runtime thread
        let cmd = spec.command.to_string();
        let args = spec.args.to_vec();
        let root_owned = root.to_path_buf();
        let tx_fail = tx.clone();
        let name = spec.name;
        let hint = spec
            .install_hint
            .unwrap_or("install it or fix languages.toml");
        let quitting = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let quitting_mainloop = quitting.clone();

        let thread = std::thread::spawn(move || {
            rt.block_on(async move {
                let child = Command::new(&cmd)
                    .args(&args)
                    .current_dir(&root_owned)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn();
                match child {
                    Ok(mut c) => {
                        let (Some(stdout), Some(stdin)) = (c.stdout.take(), c.stdin.take()) else {
                            let _ = tx_fail.send(LspEvent::Failed { server: name, hint });
                            return;
                        };
                        if let Some(mut stderr) = c.stderr.take() {
                            tokio::spawn(async move {
                                use tokio::io::AsyncReadExt;
                                let mut chunk = [0; 4096];
                                loop {
                                    match stderr.read(&mut chunk).await {
                                        Ok(0) => break,
                                        Ok(bytes) => strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({
                                            "source":"lsp_stderr", "server":name, "bytes":bytes,
                                            "message":strop_trace::preview(&String::from_utf8_lossy(&chunk[..bytes])),
                                        })),
                                        Err(error) => {
                                            strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({
                                                "source":"lsp_stderr_read", "server":name,"message":error.to_string(),
                                            }));
                                            break;
                                        }
                                    }
                                }
                            });
                        }
                        let label = format!("{name}@{}", root_owned.display());
                        strop_trace::record_with(strop_trace::EventKind::JobStarted, || serde_json::json!({"service":"lsp","server":label,"pid":c.id()}));
                        let result = mainloop.run_buffered(
                            Observed::new(stdout, &label, Direction::Rx).compat(),
                            Observed::new(stdin, &label, Direction::Tx).compat_write(),
                        ).await;
                        strop_trace::record_with(strop_trace::EventKind::JobFinished, || serde_json::json!({
                            "service":"lsp","server":label,"error":result.as_ref().err().map(ToString::to_string),
                        }));
                        if !quitting_mainloop.load(std::sync::atomic::Ordering::Relaxed) {
                            let _ = tx_fail.send(LspEvent::Failed { server: name, hint });
                        }
                    }
                    Err(error) => {
                        strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({"source":"lsp_spawn","server":name,"message":error.to_string()}));
                        let _ = tx_fail.send(LspEvent::Failed { server: name, hint });
                    }
                }
            });
        });

        let tx2 = tx.clone();
        let sock = socket.clone();
        let caps = self_caps.clone();
        let initialized = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let init_flag = initialized.clone();
        let pending = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let pending_init = pending.clone();
        let pending_reqs = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let pending_reqs_init = pending_reqs.clone();
        let params = InitializeParams {
            #[allow(deprecated)] // root_uri is what every server still honors
            root_uri: Some(root_uri),
            initialization_options: spec.init_options.cloned(),
            capabilities: async_lsp::lsp_types::ClientCapabilities {
                text_document: Some(async_lsp::lsp_types::TextDocumentClientCapabilities {
                    synchronization: Some(Default::default()),
                    publish_diagnostics: Some(Default::default()),
                    hover: Some(Default::default()),
                    definition: Some(Default::default()),
                    ..Default::default()
                }),
                // offer utf-8 (we are byte-native), accept utf-16 (spec
                // default) — the answer drives every column conversion
                general: Some(async_lsp::lsp_types::GeneralClientCapabilities {
                    position_encodings: Some(vec![
                        async_lsp::lsp_types::PositionEncodingKind::UTF8,
                        async_lsp::lsp_types::PositionEncodingKind::UTF16,
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let handle_spawn = handle.clone();
        handle_spawn.spawn(async move {
            let init = sock
                .request::<async_lsp::lsp_types::request::Initialize>(params)
                .await;
            match init {
                Ok(resp) => {
                    caps.set(resp.capabilities);
                    let _ = sock.notify::<async_lsp::lsp_types::notification::Initialized>(
                        InitializedParams {},
                    );
                    // queued opens flush now — after Initialized, always
                    init_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    let queued: Vec<_> = std::mem::take(&mut *pending_init.lock());
                    for (path, lang, text) in queued {
                        if let Ok(uri) = Url::from_file_path(&path) {
                            let item = TextDocumentItem {
                                uri,
                                language_id: lang,
                                version: 1,
                                text,
                            };
                            let _ = sock.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
                                text_document: item,
                            });
                        }
                    }
                    // requests that arrived pre-init fire now (caps are
                    // known — the gate below runs for real this time)
                    let pending_reqs: Vec<_> = std::mem::take(&mut *pending_reqs_init.lock());
                    for PendingRequest {
                        path,
                        line,
                        col,
                        req_revision,
                        kind,
                    } in pending_reqs
                    {
                        let Ok(uri) = Url::from_file_path(&path) else {
                            continue;
                        };
                        match kind {
                            QueuedRequest::Goto => {
                                let params = GotoDefinitionParams {
                                    text_document_position_params: TextDocumentPositionParams {
                                        text_document: TextDocumentIdentifier { uri },
                                        position: Position {
                                            line: line as u32,
                                            character: col as u32,
                                        },
                                    },
                                    work_done_progress_params: WorkDoneProgressParams::default(),
                                    partial_result_params: Default::default(),
                                };
                                let resp = sock.request::<GotoDefinition>(params).await;
                                if let Ok(Some(resp)) = resp {
                                    use async_lsp::lsp_types::GotoDefinitionResponse as R;
                                    let loc = match resp {
                                        R::Scalar(l) => Some(l),
                                        R::Array(v) => v.into_iter().next(),
                                        R::Link(v) => v.into_iter().next().map(|l| {
                                            async_lsp::lsp_types::Location {
                                                uri: l.target_uri,
                                                range: l.target_selection_range,
                                            }
                                        }),
                                    };
                                    if let Some(l) = loc {
                                        if let Ok(path) = l.uri.to_file_path() {
                                            let _ = tx2.send(LspEvent::GotoLocation {
                                                path,
                                                line: l.range.start.line as usize,
                                                col: l.range.start.character as usize,
                                                req_revision,
                                            });
                                        }
                                    }
                                }
                            }
                            QueuedRequest::Hover => {
                                let params = async_lsp::lsp_types::HoverParams {
                                    text_document_position_params: TextDocumentPositionParams {
                                        text_document: TextDocumentIdentifier { uri },
                                        position: Position {
                                            line: line as u32,
                                            character: col as u32,
                                        },
                                    },
                                    work_done_progress_params: WorkDoneProgressParams::default(),
                                };
                                let resp = sock
                                    .request::<async_lsp::lsp_types::request::HoverRequest>(params)
                                    .await;
                                if let Ok(Some(h)) = resp {
                                    let _ = tx2.send(LspEvent::HoverText {
                                        text: hover_text(&h),
                                    });
                                }
                            }
                            QueuedRequest::Locations(kind) => {
                                let tdp = TextDocumentPositionParams {
                                    text_document: TextDocumentIdentifier { uri },
                                    position: Position {
                                        line: line as u32,
                                        character: col as u32,
                                    },
                                };
                                request_locations(
                                    sock.clone(),
                                    tx2.clone(),
                                    kind,
                                    tdp,
                                    req_revision,
                                )
                                .await;
                            }
                            QueuedRequest::SwitchHeader => {
                                let resp = sock
                                    .request::<SwitchSourceHeader>(TextDocumentIdentifier { uri })
                                    .await;
                                match resp {
                                    Ok(Some(target)) => {
                                        if let Ok(path) = target.to_file_path() {
                                            let _ = tx2.send(LspEvent::GotoLocation {
                                                path,
                                                line: 0,
                                                col: 0,
                                                req_revision,
                                            });
                                        }
                                    }
                                    Ok(None) => {
                                        let _ = tx2.send(LspEvent::Note {
                                            text: "no header/source counterpart".into(),
                                        });
                                    }
                                    Err(e) => {
                                        let _ = tx2.send(LspEvent::Note {
                                            text: format!("switch source/header: {e}"),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    let _ = tx2.send(LspEvent::Ready { server: name });
                }
                Err(_) => {
                    let _ = tx2.send(LspEvent::Failed { server: name, hint });
                }
            }
        });

        Some(Self {
            socket,
            handle,
            tx,
            thread: std::sync::Arc::new(std::sync::Mutex::new(Some(thread))),
            root: root.to_path_buf(),
            caps: self_caps,
            quitting,
            pending_opens: pending,
            pending_requests: pending_reqs,
            initialized,
            versions: std::sync::Arc::new(
                parking_lot::Mutex::new(std::collections::HashMap::new()),
            ),
        })
    }
}
