//! LSP process admission and protocol runtime. Every server executes
//! through the already-admitted worker in its actual namespace; neither
//! local nor remote launches bypass worker supervision.

use std::future::Future;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use async_lsp::lsp_types::notification::{LogMessage, PublishDiagnostics, ShowMessage};
use async_lsp::lsp_types::{InitializeParams, InitializedParams};
use async_lsp::router::Router;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::queue::{self, WireEnv};
use super::sync::{self, FlushError};
use super::trace_io::{Direction, Observed};
use super::Client;
use crate::caps::ServerCaps;
use crate::convert::diag_from_lsp;
use crate::protocol::*;
use crate::registry;
use crate::target::Workspace;
use strop_worker_client::Worker;

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
/// and trace through the attach refusal (0033 §3) — never silence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnError {
    /// The workspace root is not an absolute path on its filesystem.
    RootUri,
    /// The runtime, client thread or wire worker could not start; the
    /// message names which.
    Startup(String),
    /// The selected workspace did not supply its required native worker.
    Unavailable(String),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RootUri => write!(f, "the workspace root is not an absolute path"),
            Self::Startup(message) => write!(f, "{message}"),
            Self::Unavailable(namespace) => {
                write!(f, "LSP on {namespace} requires an admitted worker")
            }
        }
    }
}

impl std::error::Error for SpawnError {}

/// A leased service spawn through the worker (0058 WK10): the lease and
/// the wire exec spec. `service: true` admits the relayed stdin the
/// protocol channel rides; no PTY, no env overlay — the server's context
/// is its supervised cwd, exactly like the supervised launches.
struct WorkerLaunch {
    worker: Worker,
    spec: strop_worker_protocol::ExecSpec,
}

fn worker_launch(
    worker: Worker,
    spec: &registry::ServerSpec<'_>,
    root: &Path,
) -> Result<WorkerLaunch, SpawnError> {
    let cwd = os_bytes(root.as_os_str()).ok_or_else(|| {
        SpawnError::Startup(format!(
            "workspace root {:?} is not representable as native wire bytes",
            root
        ))
    })?;
    Ok(WorkerLaunch {
        worker,
        spec: strop_worker_protocol::ExecSpec {
            program: spec.command.as_bytes().to_vec(),
            argv: spec
                .args
                .iter()
                .map(|arg| arg.as_bytes().to_vec())
                .collect(),
            cwd,
            env: Vec::new(),
            service: true,
            pty: None,
        },
    })
}

/// Native wire bytes for a path: Unix keeps arbitrary non-NUL bytes;
/// other platforms require UTF-8 rather than a lossy stand-in.
fn os_bytes(value: &std::ffi::OsStr) -> Option<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(value.as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        value.to_str().map(|text| text.as_bytes().to_vec())
    }
}

/// Grace for a worker-owned service to follow its stdin EOF out before
/// explicit revocation; the worker attests the exit classification.
const REMOTE_EXIT_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Bounded stderr retained for a worker service's failure hint.
const STDERR_TAIL_CAP: usize = 8192;

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
    /// Spawn a configured server through this workspace's admitted
    /// native worker. The spec is borrowed only for this call; absence
    /// refuses typed instead of launching another process supervisor.
    pub fn spawn(
        spec: &registry::ServerSpec<'_>,
        workspace: Workspace,
        tx: Sender<LspEvent>,
        lease: Option<Worker>,
    ) -> Result<Self, SpawnError> {
        let root_uri = workspace.uri(workspace.root()).ok_or(SpawnError::RootUri)?;
        let label_workspace = workspace.label();
        let worker = lease.ok_or_else(|| SpawnError::Unavailable(label_workspace.clone()))?;
        let launch = worker_launch(worker, spec, workspace.root())?;
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
        let tx_fail = tx.clone();
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
            workspace: workspace.clone(),
            sync: sync.clone(),
            quitting: quitting.clone(),
            closed,
        };
        let queue = queue::start(env)
            .ok_or_else(|| SpawnError::Startup("cannot start the LSP wire worker".into()))?;
        let (stop_signal, stopping) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("strop-lsp-client".into())
            .spawn(move || {
                let name = name_loop;
                let hint = hint_loop;
                rt.block_on(run_worker_launch(
                    launch,
                    mainloop,
                    stopping,
                    tx_fail,
                    id,
                    name,
                    hint,
                    label_workspace,
                    endpoint_display,
                    quitting_mainloop,
                    closed_mainloop,
                ));
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

/// Run one worker-leased server to its settlement (0058 WK10): the exec
/// rides the lease's supervised process — the worker beside the files
/// owns spawn, pipes, wait and teardown. Spawn/admission failures are
/// the client's typed error in the failure event; the exit is the
/// worker's classified terminal status (code, signal, `Lost`), never a
/// guessed code. Teardown order matches the supervised launches: the
/// mainloop's dropped write half delivers stdin EOF, the grace waits,
/// then [`super::worker_io::WorkerLease::settle`] revokes.
#[allow(clippy::too_many_arguments)]
async fn run_worker_launch(
    wlaunch: WorkerLaunch,
    mainloop: async_lsp::MainLoop<Router<ClientState>>,
    stopping: tokio::sync::oneshot::Receiver<()>,
    tx_fail: Sender<LspEvent>,
    id: ServerId,
    name: String,
    hint: String,
    label_workspace: String,
    endpoint_display: Option<String>,
    quitting: Arc<std::sync::atomic::AtomicBool>,
    closed: Arc<std::sync::atomic::AtomicBool>,
) {
    let cmd = String::from_utf8_lossy(&wlaunch.spec.program).into_owned();
    // The exec request's own token: admission only — the admitted exec
    // lives under its own lease on the worker side.
    let (request_token, _hold) = strop_core::worker::CancelToken::standalone();
    let handle = match wlaunch.worker.exec(&request_token, wlaunch.spec) {
        Ok(handle) => handle,
        Err(error) => {
            strop_trace::record_with(strop_trace::EventKind::Error, || {
                serde_json::json!({
                    "source":"lsp_spawn","server":name,"command":&cmd,"message":error.to_string(),
                })
            });
            let reason = format!("cannot run `{cmd}` on {label_workspace}: {error}");
            let _ = tx_fail.send(LspEvent::Failed {
                server: id,
                name,
                hint: format!("{reason} — {hint}"),
            });
            return;
        }
    };
    let io = super::worker_io::start(handle, wlaunch.worker.clone(), STDERR_TAIL_CAP);
    let super::worker_io::WorkerIo {
        stdout,
        stdin,
        stderr_tail,
        lease,
    } = io;
    let label = format!("{name}@{label_workspace}");
    strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
        let mut event = serde_json::json!({"service":"lsp","server":label,"worker":true});
        if let Some(endpoint) = &endpoint_display {
            event["remote"] = serde_json::json!(endpoint);
        }
        event
    });
    let result = {
        let mut run = std::pin::pin!(mainloop.run_buffered(
            Observed::new(stdout, &label, Direction::Rx).compat(),
            Observed::new(stdin, &label, Direction::Tx).compat_write(),
        ));
        let mut stopping = std::pin::pin!(stopping);
        std::future::poll_fn(|context| {
            if stopping.as_mut().poll(context).is_ready() {
                return std::task::Poll::Ready(Ok(()));
            }
            run.as_mut().poll(context)
        })
        .await
    };
    closed.store(true, std::sync::atomic::Ordering::Relaxed);
    strop_trace::record_with(strop_trace::EventKind::JobFinished, || {
        serde_json::json!({
            "service":"lsp","server":label,"error":result.as_ref().err().map(ToString::to_string),
        })
    });
    // Teardown: the dropped write half already delivered stdin EOF;
    // settle waits the grace, then revokes the lease (TERM/grace/KILL
    // on the worker side) and collects the classified exit.
    lease.settle(REMOTE_EXIT_GRACE);
    if !quitting.load(std::sync::atomic::Ordering::Relaxed) {
        let mut detail = match &result {
            Err(error) => format!(": {error}"),
            Ok(()) => String::new(),
        };
        {
            let guard = stderr_tail.lock();
            let text = String::from_utf8_lossy(&guard).trim().to_string();
            if !text.is_empty() {
                detail = format!("{detail}; stderr: {text}");
            }
            if let Some(status) = lease.status() {
                detail = format!(" ({status:?}){detail}");
            }
        }
        let hint = format!("{name} exited unexpectedly{detail} — {hint}");
        let _ = tx_fail.send(LspEvent::Failed {
            server: id,
            name,
            hint,
        });
    }
}
