//! Server process spawn, initialize handshake and runtime mainloop
//! wiring. The wire queue worker starts here, one per connection.
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use async_lsp::lsp_types::notification::PublishDiagnostics;
use async_lsp::lsp_types::{InitializeParams, InitializedParams, Url};
use async_lsp::router::Router;
use tokio::process::Command;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::queue::{self, WireEnv};
use super::sync::{self, FlushError};
use super::trace_io::{Direction, Observed};
use super::Client;
use crate::caps::ServerCaps;
use crate::convert::diag_from_lsp;
use crate::protocol::*;
use crate::registry;

struct ClientState {
    tx: Sender<LspEvent>,
    id: ServerId,
    caps: ServerCaps,
    sync: Arc<parking_lot::Mutex<sync::SyncState>>,
}

/// Why a client could not start. Every variant reaches the modeline
/// and trace through the attach refusal (0033 §3) — never silence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnError {
    /// The workspace root is not an absolute local path.
    RootUri,
    /// The runtime, client thread or wire worker could not start; the
    /// message names which.
    Startup(String),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RootUri => write!(f, "the workspace root is not an absolute local path"),
            Self::Startup(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SpawnError {}

impl Client {
    /// Spawn the configured server; start its runtime, wire queue and
    /// initialize handshake. Nothing is executed before this call —
    /// executability was settled by discovery's metadata check. The
    /// spec only borrows the caller's configuration tables for the
    /// duration of the call.
    pub fn spawn(
        spec: &registry::ServerSpec<'_>,
        root: &Path,
        tx: Sender<LspEvent>,
    ) -> Result<Self, SpawnError> {
        let root_uri = Url::from_file_path(root).map_err(|_| SpawnError::RootUri)?;
        let id = ServerId::allocate();
        let self_caps = ServerCaps::default();
        let sync = Arc::new(parking_lot::Mutex::new(sync::SyncState::default()));
        let (mainloop, socket) = async_lsp::MainLoop::new_client(|_server| {
            let mut router = Router::new(ClientState {
                tx: tx.clone(),
                id,
                caps: self_caps.clone(),
                sync: sync.clone(),
            });
            router.notification::<PublishDiagnostics>(|st, params| {
                let Ok(path) = params.uri.to_file_path() else {
                    return std::ops::ControlFlow::Continue(());
                };
                let context = st.sync.lock().diagnostic_context(
                    &path,
                    params.version.map(WireVersion::new),
                    st.id,
                    st.caps.encoding(),
                );
                if let Some(context) = context {
                    let diags = params.diagnostics.iter().map(diag_from_lsp).collect();
                    let _ = st.tx.send(LspEvent::Diagnostics {
                        context,
                        path,
                        diags,
                    });
                }
                std::ops::ControlFlow::Continue(())
            });
            router
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                SpawnError::Startup(format!("cannot build the LSP runtime: {error}"))
            })?;
        let handle = rt.handle().clone();
        // All tokio work, including child spawn, lives on the runtime thread.
        let cmd = spec.command.to_string();
        let args = spec.args.to_vec();
        let root_owned = root.to_path_buf();
        let tx_fail = tx.clone();
        let name = spec.name.to_string();
        let hint = spec
            .install_hint
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("install `{cmd}` or fix the command in languages.toml"));
        let name_loop = name.clone();
        let hint_loop = hint.clone();
        let quitting = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let quitting_mainloop = quitting.clone();
        // Set once the mainloop ends: the wire worker stops framing.
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let closed_mainloop = closed.clone();
        // The wire worker shares the same synchronized open-document
        // table, socket and runtime as the handle. Starting it before
        // the runtime thread means a failure below leaks no thread, no
        // wire worker and no server process.
        let env = WireEnv {
            id,
            name: name.clone(),
            hint: hint.clone(),
            socket: socket.clone(),
            handle: handle.clone(),
            tx: tx.clone(),
            caps: self_caps.clone(),
            root: root.to_path_buf(),
            sync: sync.clone(),
            quitting: quitting.clone(),
            closed,
        };
        let queue = queue::start(env)
            .ok_or_else(|| SpawnError::Startup("cannot start the LSP wire worker".into()))?;
        let thread = std::thread::Builder::new()
            .name("strop-lsp-client".into())
            .spawn(move || {
                let name = name_loop;
                let hint = hint_loop;
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
                                let _ = tx_fail.send(LspEvent::Failed {
                                    server: id, name: name.clone(), hint: hint.clone(),
                                });
                                return;
                            };
                            if let Some(mut stderr) = c.stderr.take() {
                                let name_stderr = name.clone();
                                tokio::spawn(async move {
                                    use tokio::io::AsyncReadExt;
                                    let mut chunk = [0; 4096];
                                    loop {
                                        match stderr.read(&mut chunk).await {
                                            Ok(0) => break,
                                            Ok(bytes) => strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({
                                                "source":"lsp_stderr", "server":name_stderr, "bytes":bytes,
                                                "message":strop_trace::preview(&String::from_utf8_lossy(&chunk[..bytes])),
                                            })),
                                            Err(error) => {
                                                strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({
                                                    "source":"lsp_stderr_read", "server":name_stderr,"message":error.to_string(),
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
                            closed_mainloop.store(true, std::sync::atomic::Ordering::Relaxed);
                            strop_trace::record_with(strop_trace::EventKind::JobFinished, || serde_json::json!({
                                "service":"lsp","server":label,"error":result.as_ref().err().map(ToString::to_string),
                            }));
                            if !quitting_mainloop.load(std::sync::atomic::Ordering::Relaxed) {
                                let _ = tx_fail.send(LspEvent::Failed {
                                    server: id, name: name.clone(), hint: hint.clone(),
                                });
                            }
                        }
                        Err(error) => {
                            // The spawn failure names the command and the
                            // io error — silence or a bare "failed" is not
                            // a report (0033 §3).
                            let reason = format!("cannot run `{cmd}`: {error}");
                            strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({
                                "source":"lsp_spawn","server":name,"command":&cmd,"message":error.to_string(),
                            }));
                            let _ = tx_fail.send(LspEvent::Failed {
                                server: id, name: name.clone(), hint: format!("{reason} — {hint}"),
                            });
                        }
                    }
                });
            })
            .map_err(|error| {
                SpawnError::Startup(format!("cannot start the LSP client thread: {error}"))
            })?;
        let client = Self {
            id,
            next_request: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            sync,
            socket,
            handle,
            tx,
            thread: Arc::new(std::sync::Mutex::new(Some(thread))),
            root: root.to_path_buf(),
            caps: self_caps,
            quitting,
            queue,
        };
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
                // Offer utf-8 first, accept the spec default utf-16.
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
        let initializing = client.clone();
        client.handle.spawn(async move {
            let init = initializing
                .socket
                .request::<async_lsp::lsp_types::request::Initialize>(params)
                .await;
            match init {
                Ok(response) => {
                    initializing.caps.set(response.capabilities);
                    let initialized = initializing
                        .socket
                        .notify::<async_lsp::lsp_types::notification::Initialized>(
                        InitializedParams {},
                    );
                    let flushed = initializing.finish_initialize();
                    if initialized.is_err() || flushed.is_err() {
                        let reason = match flushed {
                            Err(FlushError::VersionExhausted) => {
                                "document versions exhausted".to_string()
                            }
                            _ => String::new(),
                        };
                        let hint = if reason.is_empty() {
                            hint
                        } else {
                            format!("{hint} ({reason})")
                        };
                        let _ = initializing.tx.send(LspEvent::Failed {
                            server: id,
                            name: name.clone(),
                            hint,
                        });
                        return;
                    }
                    let _ = initializing.tx.send(LspEvent::Ready {
                        server: id,
                        name: name.clone(),
                    });
                }
                Err(_) => {
                    let _ = initializing.tx.send(LspEvent::Failed {
                        server: id,
                        name: name.clone(),
                        hint: hint.clone(),
                    });
                }
            }
        });
        Ok(client)
    }
}
