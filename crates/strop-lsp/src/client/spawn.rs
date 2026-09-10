//! Server process spawn, initialize handshake and runtime mainloop
//! wiring. The wire queue worker starts here, one per connection.
//! A local server runs in the workspace root; a remote server runs on
//! its endpoint inside the remote root through strop-remote's single
//! process policy (0036 RW8): one owned SSH client whose stdin/stdout
//! carry the protocol, with a bounded teardown that never leaks the
//! local ssh process.
use std::future::Future;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::Stdio;
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
pub(crate) struct ClientState {
    tx: Sender<LspEvent>,
    id: ServerId,
    caps: ServerCaps,
    sync: Arc<parking_lot::Mutex<sync::SyncState>>,
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

/// What the runtime thread spawns: a local process described by the
/// spec, or the supervised SSH client of a remote server command.
enum Launch {
    Local {
        cmd: String,
        args: Vec<String>,
        cwd: PathBuf,
    },
    /// A server inside a running container: `docker exec -i` carries
    /// stdio; the local client's death ends the in-container program
    /// (stdin EOF), no SSH and no supervisor anywhere (0037 DC1b).
    Container {
        id: strop_workspace::ContainerId,
        cmd: String,
        args: Vec<String>,
        cwd: PathBuf,
    },
    Remote(RemoteLaunch),
}

/// How long the remote teardown waits for the local ssh to follow the
/// server out before killing it. The server sees stdin EOF when the
/// mainloop drops the write half, exits, and the supervisor reaps the
/// remote group — the kill is a backstop, never the primary mechanism.
const REMOTE_EXIT_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Bounded stderr retained for a remote failure hint.
const STDERR_TAIL_CAP: usize = 8192;

/// The supervised remote launch through strop-remote's ONE process
/// policy: a checked [`strop_remote::RemoteCommand`] (program, inert
/// argv, absolute remote cwd) plus the ssh argv the policy builds —
/// safety options, destination, remote supervision encoding and
/// relayed server stdin. The key turns the supervisor's nonce-marked
/// stderr records into typed outcomes; nothing here restates the
/// policy.
struct RemoteLaunch {
    command: std::process::Command,
    supervision: strop_remote::SupervisionKey,
}

fn remote_launch(
    endpoint: &strop_workspace::RemoteEndpoint,
    spec: &registry::ServerSpec<'_>,
    root: &Path,
) -> Result<RemoteLaunch, SpawnError> {
    let args: Vec<std::ffi::OsString> = spec
        .args
        .iter()
        .map(|arg| std::ffi::OsString::from(arg.as_str()))
        .collect();
    let command = strop_remote::RemoteCommand::new(spec.command, args, root).map_err(|error| {
        SpawnError::Startup(format!("remote command rejected for {endpoint}: {error}"))
    })?;
    // Relayed stdin: the protocol channel to the remote server; the
    // lifetime lease is this client's stdin writer.
    let (mut ssh, supervision) =
        strop_remote::command_supervised(endpoint, &command, strop_remote::StdinMode::Relayed)
            .map_err(|error| SpawnError::Startup(format!("ssh for {endpoint}: {error}")))?;
    // A private process group: ProxyCommand children and any other
    // local descendants die with the group, matching the shared
    // policy's supervision contract for owned stdio clients.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        ssh.process_group(0);
    }
    Ok(RemoteLaunch {
        command: ssh,
        supervision,
    })
}

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
) -> Router<ClientState> {
    let mut router = Router::new(ClientState { tx, id, caps, sync });
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
    /// Spawn the configured server on the given workspace — locally,
    /// or on the remote endpoint inside the remote root — and start
    /// its runtime, wire queue and initialize handshake. Nothing is
    /// executed before this call; executability was settled by
    /// discovery's checks. The spec is only borrowed for the duration
    /// of the call.
    pub fn spawn(
        spec: &registry::ServerSpec<'_>,
        workspace: Workspace,
        tx: Sender<LspEvent>,
    ) -> Result<Self, SpawnError> {
        let root_uri = workspace.uri(workspace.root()).ok_or(SpawnError::RootUri)?;
        let launch = match &workspace {
            Workspace::Local { root } => Launch::Local {
                cmd: spec.command.to_string(),
                args: spec.args.to_vec(),
                cwd: root.clone(),
            },
            Workspace::Remote { endpoint, root } => {
                Launch::Remote(remote_launch(endpoint, spec, root)?)
            }
            Workspace::Container { container, root } => Launch::Container {
                id: container.clone(),
                cmd: spec.command.to_string(),
                args: spec.args.to_vec(),
                cwd: root.clone(),
            },
        };
        let remote = workspace.endpoint().is_some();
        let in_container = matches!(workspace, Workspace::Container { .. });
        let label_workspace = workspace.label();
        // The thread is 'static: it gets owned copies, never borrows
        // into the spawning scope.
        let endpoint_display = workspace.endpoint().map(|e| e.to_string());
        let supervision = match &launch {
            Launch::Remote(remote) => Some(remote.supervision.clone()),
            Launch::Local { .. } | Launch::Container { .. } => None,
        };
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
            move |_server| client_router(tx, id, caps, sync, workspace, name)
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
        // Remote failures carry the ssh stderr tail: "connection
        // refused", "command not found" — the user's actionable fact.
        let stderr_tail = Arc::new(parking_lot::Mutex::new(Vec::new()));
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
                rt.block_on(async move {
                    let mut command = match launch {
                        Launch::Local { cmd, args, cwd } => {
                            let mut command = tokio::process::Command::new(&cmd);
                            command.args(&args).current_dir(&cwd);
                            command
                        }
                        // kill_on_drop: even a panicking runtime thread
                        // cannot leak the local ssh client; the remote
                        // server group is reaped by the supervisor on
                        // stdin EOF.
                        Launch::Remote(remote) => {
                            let mut command = tokio::process::Command::from(remote.command);
                            command.kill_on_drop(true);
                            command
                        }
                        Launch::Container {
                            id, cmd, args, cwd,
                        } => {
                            let mut command = tokio::process::Command::from(
                                strop_containers::exec_command(&id, &cmd, &args, &cwd),
                            );
                            command.kill_on_drop(true);
                            command
                        }
                    };
                    command
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped());
                    command.kill_on_drop(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::CommandExt;
                        command.as_std_mut().process_group(0);
                    }
                    match command.spawn() {
                        Ok(child) => {
                            let mut c = super::process::ServerProcess::new(child);
                            let Ok((stdout, stdin, mut stderr)) = c.take_io() else {
                                let _ = tx_fail.send(LspEvent::Failed {
                                    server: id, name: name.clone(), hint: hint.clone(),
                                });
                                return;
                            };
                            let stderr_drain = {
                                let name_stderr = name.clone();
                                let tail = stderr_tail.clone();
                                tokio::spawn(async move {
                                    use tokio::io::AsyncReadExt;
                                    let mut chunk = [0; 4096];
                                    loop {
                                        match stderr.read(&mut chunk).await {
                                            Ok(0) => break,
                                            Ok(bytes) => {
                                                let mut guard = tail.lock();
                                                guard.extend_from_slice(&chunk[..bytes]);
                                                let overflow = guard.len().saturating_sub(STDERR_TAIL_CAP);
                                                if overflow > 0 {
                                                    guard.drain(..overflow);
                                                }
                                                strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({
                                                    "source":"lsp_stderr", "server":name_stderr, "bytes":bytes,
                                                    "message":strop_trace::preview(&String::from_utf8_lossy(&chunk[..bytes])),
                                                }));
                                            }
                                            Err(error) => {
                                                strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({
                                                    "source":"lsp_stderr_read", "server":name_stderr,"message":error.to_string(),
                                                }));
                                                break;
                                            }
                                        }
                                    }
                                })
                            };
                            let label = format!("{name}@{label_workspace}");
                            strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
                                let mut event = serde_json::json!({"service":"lsp","server":label,"pid":c.id()});
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
                                }).await
                            };
                            closed_mainloop.store(true, std::sync::atomic::Ordering::Relaxed);
                            strop_trace::record_with(strop_trace::EventKind::JobFinished, || serde_json::json!({
                                "service":"lsp","server":label,"error":result.as_ref().err().map(ToString::to_string),
                            }));
                            let grace = if remote { REMOTE_EXIT_GRACE } else { std::time::Duration::ZERO };
                            if let Err(error) = c.finish(Some(stderr_drain), grace).await {
                                strop_trace::record_with(strop_trace::EventKind::Error, || serde_json::json!({
                                    "source":"lsp_process_cleanup", "server":label, "message":error.to_string(),
                                }));
                            }
                            if !quitting_mainloop.load(std::sync::atomic::Ordering::Relaxed) {
                                // The mainloop's own error is the primary
                                // cause (protocol break, server closed the
                                // connection); stderr and supervision
                                // records add the process-level truth. The
                                // generic install hint alone would mask
                                // the real reason.
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
                                    // Typed supervision records name the
                                    // remote exit truthfully (signaled,
                                    // launch failure, supervisor error).
                                    if let Some(key) = &supervision {
                                        if let Some(outcome) =
                                            key.records(&guard).last().map(|o| format!("{o:?}"))
                                        {
                                            detail = format!(" ({outcome}){detail}");
                                        }
                                    }
                                }
                                let hint = format!("{name} exited unexpectedly{detail} — {hint}");
                                let _ = tx_fail.send(LspEvent::Failed {
                                    server: id,
                                    name: name.clone(),
                                    hint,
                                });
                            }
                        }
                        Err(error) => {
                            // The spawn failure names the command and the
                            // io error — silence or a bare "failed" is not
                            // a report (0033 §3).
                            let where_ = if remote || in_container {
                                format!(" on {label_workspace}")
                            } else {
                                String::new()
                            };
                            let reason = format!("cannot run `{cmd}`{where_}: {error}");
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
