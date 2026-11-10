//! LSP client startup: the production router, the client handle and
//! the initialize handshake. The server process itself is the caller's
//! `Command`, run to settlement by [`super::process`].

use std::ops::ControlFlow;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use async_lsp::lsp_types::notification::{LogMessage, PublishDiagnostics, ShowMessage};
use async_lsp::lsp_types::{InitializeParams, InitializedParams};
use async_lsp::router::Router;

use super::queue::{self, WireEnv};
use super::sync::{self, FlushError};
use super::Client;
use crate::caps::ServerCaps;
use crate::convert::diag_from_lsp;
use crate::protocol::*;
use crate::registry;
use crate::target::Workspace;

pub(crate) struct ClientState {
    tx: Sender<LspEvent>,
    id: ServerId,
    caps: ServerCaps,
    sync: Arc<parking_lot::Mutex<sync::SyncState>>,
    /// The server's languages.toml config block — answered to
    /// `workspace/configuration` pulls (0043 follow-on: the block used
    /// to be serialized into initializationOptions and ignored by every
    /// server that reads settings the standard way).
    config: Option<serde_json::Value>,
}

/// Why a client could not start. Every variant reaches the modeline
/// and trace through the attach refusal (0033 §3) — never silence. A
/// server that cannot run is not one: that is a `Failed` event, raised
/// on the client's own thread where the process spawns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnError {
    /// The workspace root is not an absolute path on its filesystem.
    RootUri,
    /// The runtime, client thread or wire queue could not start; the
    /// message names which.
    Startup(String),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RootUri => write!(f, "the workspace root is not an absolute path"),
            Self::Startup(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SpawnError {}

/// The production client router: diagnostics, server messages, and a
/// tolerant catch-all. A free function (not a closure inline in
/// `spawn`) so tests drive the exact handlers a spawned client uses.
pub(crate) fn client_router(
    tx: Sender<LspEvent>,
    id: ServerId,
    caps: ServerCaps,
    sync: Arc<parking_lot::Mutex<sync::SyncState>>,
    workspace: Workspace,
    name: String,
    config: Option<serde_json::Value>,
) -> Router<ClientState> {
    let mut router = Router::new(ClientState {
        tx,
        id,
        caps,
        sync,
        config,
    });
    let diag_workspace = workspace;
    router.notification::<PublishDiagnostics>(move |st, params| {
        // The URI names a file on the server's own filesystem;
        // decode there, never against the local disk.
        let Some(path) = diag_workspace.decode(&params.uri) else {
            return ControlFlow::Continue(());
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
                doc: strop_workspace::ResourceLocation {
                    filesystem: diag_workspace.target(),
                    path,
                },
                diags,
            });
        }
        ControlFlow::Continue(())
    });
    // window/showMessage is user-facing and reaches the status line;
    // window/logMessage is server logging and stays in the trace.
    // Neither may kill the connection: pyright sends logMessage on
    // every startup, and async-lsp's default catch-all breaks the
    // mainloop on any notification the client did not register.
    router.notification::<ShowMessage>(move |st, params| {
        strop_trace::record_with(strop_trace::EventKind::LspMessage, || {
            serde_json::json!({"service":"lsp","server":st.id,"method":"window/showMessage",
                "message":strop_trace::preview(&params.message)})
        });
        let _ = st.tx.send(LspEvent::ServerMessage {
            server: st.id,
            name: name.clone(),
            text: params.message,
        });
        ControlFlow::Continue(())
    });
    router.notification::<LogMessage>(move |st, params| {
        strop_trace::record_with(strop_trace::EventKind::LspMessage, || {
            serde_json::json!({"service":"lsp","server":st.id,"method":"window/logMessage",
                "message":strop_trace::preview(&params.message)})
        });
        ControlFlow::Continue(())
    });
    // Servers that read settings the standard way pull them via
    // workspace/configuration; the languages.toml config block answers,
    // section-scoped when the pull names one.
    router.request::<async_lsp::lsp_types::request::WorkspaceConfiguration, _>(|st, params| {
        let config = st.config.clone();
        async move {
            let answer: Vec<serde_json::Value> = params
                .items
                .iter()
                .map(|item| match (&config, &item.section) {
                    (Some(config), Some(section)) => config
                        .get(section)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                    (Some(config), None) => config.clone(),
                    (None, _) => serde_json::Value::Null,
                })
                .collect();
            Ok(answer)
        }
    });
    // The spec permits notifications a client does not handle; the
    // correct response is to ignore them. Trace and continue.
    router.unhandled_notification(|st, notif| {
        strop_trace::record_with(strop_trace::EventKind::LspMessage, || {
            serde_json::json!({"service":"lsp","server":st.id,"method":notif.method,"ignored":true})
        });
        ControlFlow::Continue(())
    });
    router
}

impl Client {
    /// Start a client for `spec` whose server is `command`: program,
    /// arguments and working directory already chosen by the caller
    /// ([`registry::ServerSpec::local_command`] locally, an `ssh` client
    /// remotely). Only this call runs on the caller's thread; the
    /// process spawns on the client's own runtime thread, and a server
    /// that cannot run arrives as `LspEvent::Failed`. Stdio is piped
    /// here whatever the command said.
    pub fn spawn(
        spec: &registry::ServerSpec<'_>,
        workspace: Workspace,
        tx: Sender<LspEvent>,
        command: std::process::Command,
    ) -> Result<Self, SpawnError> {
        let root_uri = workspace.uri(workspace.root()).ok_or(SpawnError::RootUri)?;
        let label_workspace = workspace.label();
        // The thread is 'static: it gets owned copies, never borrows
        // into the spawning scope.
        let endpoint_display = workspace.endpoint().map(|e| e.to_string());
        let id = ServerId::allocate();
        let self_caps = ServerCaps::default();
        let sync = Arc::new(parking_lot::Mutex::new(sync::SyncState::default()));
        let diag_workspace = workspace.clone();
        let name = spec.name.to_string();
        let (mainloop, socket) = async_lsp::MainLoop::new_client({
            let tx = tx.clone();
            let caps = self_caps.clone();
            let sync = sync.clone();
            let workspace = diag_workspace.clone();
            let name = name.clone();
            let config = spec.init_options.cloned();
            move |_server| client_router(tx, id, caps, sync, workspace, name, config)
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
        let hint = spec
            .install_hint
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("install `{cmd}` or fix the command in languages.toml"));
        let quitting = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Set once the mainloop ends: the wire worker stops framing.
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let outbound = Arc::new(super::outbound::Outbound::default());
        let launch = super::process::Launch {
            id,
            name: name.clone(),
            program: cmd,
            hint: hint.clone(),
            label_workspace,
            endpoint: endpoint_display,
            tx: tx.clone(),
            quitting: quitting.clone(),
            closed: closed.clone(),
            outbound: outbound.clone(),
        };
        let completion_registry = Arc::new(super::completion::Registry::default());
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
            workspace: workspace.clone(),
            sync: sync.clone(),
            quitting: quitting.clone(),
            closed: closed.clone(),
            outbound: outbound.clone(),
            completion: completion_registry.clone(),
        };
        let queue = queue::start(env)
            .ok_or_else(|| SpawnError::Startup("cannot start the LSP wire worker".into()))?;
        let completion = Arc::new(super::completion::CompletionClient::new(
            completion_registry,
            super::completion::Environment {
                server: id,
                socket: socket.clone(),
                handle: handle.clone(),
                caps: self_caps.clone(),
                sync: sync.clone(),
                workspace: workspace.clone(),
                outbound,
                queue: queue.control(),
            },
        ));
        let (stop_signal, stopping) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("strop-lsp-client".into())
            .spawn(move || {
                rt.block_on(super::process::run(launch, command, mainloop, stopping));
            })
            .map_err(|error| {
                SpawnError::Startup(format!("cannot start the LSP client thread: {error}"))
            })?;
        let root_name = workspace
            .root()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "root".into());
        let client = Self {
            id,
            next_request: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            sync,
            socket,
            handle,
            tx,
            workspace,
            thread: Arc::new(std::sync::Mutex::new(Some(thread))),
            caps: self_caps,
            quitting,
            queue,
            completion,
            stop: Arc::new(super::ServiceStop(parking_lot::Mutex::new(Some(
                stop_signal,
            )))),
        };
        let params = InitializeParams {
            #[allow(deprecated)] // root_uri is what every server still honors
            root_uri: Some(root_uri.clone()),
            // pyright (and others) discover project config through
            // workspace folders — rootUri alone has been insufficient
            // since LSP 3.6 (field report: pyrightconfig.json unseen).
            workspace_folders: Some(vec![async_lsp::lsp_types::WorkspaceFolder {
                name: root_name,
                uri: root_uri,
            }]),
            initialization_options: spec.init_options.cloned(),
            capabilities: async_lsp::lsp_types::ClientCapabilities {
                text_document: Some(async_lsp::lsp_types::TextDocumentClientCapabilities {
                    synchronization: Some(Default::default()),
                    publish_diagnostics: Some(Default::default()),
                    hover: Some(Default::default()),
                    definition: Some(Default::default()),
                    completion: Some(crate::completion::client_capabilities()),
                    ..Default::default()
                }),
                // Servers that read settings pull them via
                // workspace/configuration — advertised so the pull comes
                // (languages.toml's config block answers it below).
                workspace: Some(async_lsp::lsp_types::WorkspaceClientCapabilities {
                    workspace_folders: Some(true),
                    configuration: Some(true),
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
            let init = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                initializing
                    .socket
                    .request::<async_lsp::lsp_types::request::Initialize>(params),
            )
            .await;
            match init {
                Ok(Ok(response)) => {
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
                // The connection ended under the handshake: the process
                // run reports why (spawn failure, exit status, stderr) —
                // one failure, the real one.
                Ok(Err(_)) if closed.load(std::sync::atomic::Ordering::Relaxed) => {}
                outcome => {
                    initializing.stop.halt();
                    // A bare install hint masks the real reason: the
                    // server's own refusal or a timeout names it instead.
                    let reason = match outcome {
                        Ok(Err(error)) => format!("initialize refused: {error}"),
                        _ => "initialize timed out".to_string(),
                    };
                    let _ = initializing.tx.send(LspEvent::Failed {
                        server: id,
                        name: name.clone(),
                        hint: format!("{reason} — {hint}"),
                    });
                }
            }
        });
        Ok(client)
    }
}
