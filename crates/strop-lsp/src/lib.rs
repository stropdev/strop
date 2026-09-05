//! strop-lsp: the LSP client. async-lsp transport (0009 §2.1), tokio on
//! a worker thread, the editor sees a channel of typed events — never an
//! async type, never an await in the input path (0001 §5.6).
pub mod languages;
pub mod registry;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use tokio::process::Command;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use async_lsp::lsp_types::{
    notification::{DidChangeTextDocument, DidOpenTextDocument, PublishDiagnostics},
    request::{GotoDefinition, HoverRequest},
    Diagnostic, DidChangeTextDocumentParams, DidOpenTextDocumentParams, GotoDefinitionParams,
    HoverParams, InitializeParams, InitializedParams, Position, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, Url, VersionedTextDocumentIdentifier,
    WorkDoneProgressParams,
};
use async_lsp::router::Router;
use async_lsp::ServerSocket;

/// One diagnostic, in UTF-8 buffer coordinates (the encoding boundary
/// lives here — 0009 §2.6).
#[derive(Debug, Clone)]
pub struct Diag {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub severity: u8, // 1 error, 2 warning, 3 info, 4 hint
    pub message: String,
}

impl Diag {
    pub fn severity_char(&self) -> char {
        match self.severity {
            1 => 'E',
            2 => 'W',
            3 => 'I',
            _ => 'H',
        }
    }
}

/// Events the editor drains from the channel.
/// Requests that can queue pre-init (0.6.1: gd on a freshly opened
/// project raced clangd's init and died silently).
#[derive(Debug, Clone, Copy)]
pub enum QueuedRequest {
    Goto,
    Hover,
    SwitchHeader,
}

/// One pre-init request: target doc, position, kind.
pub struct PendingRequest {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub kind: QueuedRequest,
}

/// Position encoding negotiated with the server (LSP 3.17): strop is
/// byte-native and offers `utf-8` first; a server that only speaks
/// UTF-16 (the spec default) gets converted columns both ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

/// Byte column → server column for one line's text.
pub fn to_server_col(line: &str, byte_col: usize, enc: PositionEncoding) -> usize {
    match enc {
        PositionEncoding::Utf8 => byte_col,
        PositionEncoding::Utf16 => line
            .get(..byte_col)
            .unwrap_or(line)
            .chars()
            .map(|c| c.len_utf16())
            .sum(),
    }
}

/// Server column → byte column for one line's text.
pub fn to_byte_col(line: &str, server_col: usize, enc: PositionEncoding) -> usize {
    match enc {
        PositionEncoding::Utf8 => server_col,
        PositionEncoding::Utf16 => {
            let mut units = 0;
            for (i, c) in line.char_indices() {
                if units >= server_col {
                    return i;
                }
                units += c.len_utf16();
            }
            line.len()
        }
    }
}

pub enum LspEvent {
    Diagnostics {
        path: PathBuf,
        diags: Vec<Diag>,
    },
    Ready {
        server: &'static str,
    },
    Failed {
        server: &'static str,
        hint: &'static str,
    },
    HoverText {
        text: String,
    },
    GotoLocation {
        path: PathBuf,
        line: usize,
        col: usize,
    },
    /// A user-facing note that is neither ready nor failure (e.g.
    /// "no header counterpart").
    Note {
        text: String,
    },
}

/// Client-side state for the router.
struct ClientState {
    tx: Sender<LspEvent>,
}

/// Server capabilities from the Initialize result (0009 §2.5): requests
/// are gated on them — servers lie about what they do, and unsupported
/// requests are quiet no-ops, never error spam. Cloned between the
/// `Client` handle and the init task that fills it in.
#[derive(Clone, Default)]
pub struct ServerCaps(Arc<Mutex<Option<async_lsp::lsp_types::ServerCapabilities>>>);

impl ServerCaps {
    fn set(&self, caps: async_lsp::lsp_types::ServerCapabilities) {
        let Ok(mut guard) = self.0.lock() else { return };
        *guard = Some(caps);
    }

    /// Hover supported? Capabilities not yet arrived (or a poisoned
    /// lock) count as no — requests never race server startup.
    pub fn hover(&self) -> bool {
        use async_lsp::lsp_types::HoverProviderCapability;
        let Ok(guard) = self.0.lock() else {
            return false;
        };
        matches!(
            guard.as_ref().and_then(|c| c.hover_provider.as_ref()),
            Some(HoverProviderCapability::Simple(true)) | Some(HoverProviderCapability::Options(_))
        )
    }

    /// Goto-definition supported? (same unknown-is-no rule as hover)
    pub fn goto_definition(&self) -> bool {
        use async_lsp::lsp_types::OneOf;
        let Ok(guard) = self.0.lock() else {
            return false;
        };
        matches!(
            guard.as_ref().and_then(|c| c.definition_provider.as_ref()),
            Some(OneOf::Left(true)) | Some(OneOf::Right(_))
        )
    }

    /// The negotiated column encoding (spec default UTF-16 until the
    /// initialize result says otherwise).
    pub fn encoding(&self) -> PositionEncoding {
        let Ok(guard) = self.0.lock() else {
            return PositionEncoding::Utf16;
        };
        match guard.as_ref().and_then(|c| c.position_encoding.as_ref()) {
            Some(k) if *k == async_lsp::lsp_types::PositionEncodingKind::UTF8 => {
                PositionEncoding::Utf8
            }
            _ => PositionEncoding::Utf16,
        }
    }
}
/// A live server connection: send from any thread, the runtime thread
/// owns the socket drain.
pub struct Client {
    socket: ServerSocket,
    handle: tokio::runtime::Handle,
    tx: Sender<LspEvent>,
    root: PathBuf,
    caps: ServerCaps,
    /// Set once `shutdown()` runs: a server exit afterwards is the clean
    /// protocol exit, not a crash — no Failed event (the demo tape's
    /// `:q!` used to end every LSP session in a fake failure).
    quitting: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The runtime thread running the server mainloop. Joined on
    /// shutdown — dropping the socket while it lives panics inside
    /// async-lsp ("Sender is alive", seen in the demo tape).
    thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Documents opened before initialize completes — flushed on
    /// Initialized (strict servers like pyright drop pre-init opens).
    pending_opens: std::sync::Arc<parking_lot::Mutex<Vec<(PathBuf, String, String)>>>,
    /// goto/hover/switch fired before initialize answered: caps are
    /// unknown (not "no") — queue, flush on Initialized like
    /// pending_opens.
    pending_requests: std::sync::Arc<parking_lot::Mutex<Vec<PendingRequest>>>,
    initialized: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Per-document didChange versions — the spec requires strictly
    /// increasing, and pyright-family servers enforce it (0014).
    versions: std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<PathBuf, i32>>>,
}
/// clangd's proprietary `textDocument/switchSourceHeader` — not part of
/// the LSP spec, so lsp-types doesn't model it.
enum SwitchSourceHeader {}

impl async_lsp::lsp_types::request::Request for SwitchSourceHeader {
    type Params = async_lsp::lsp_types::TextDocumentIdentifier;
    type Result = Option<async_lsp::lsp_types::Url>;
    const METHOD: &'static str = "textDocument/switchSourceHeader";
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
                let _ = st.tx.send(LspEvent::Diagnostics { path, diags });
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
                    .stderr(match std::env::var_os("STROP_LSP_LOG") {
                        // append: strop's own trace lines share the file —
                        // create() truncated them on every server spawn
                        Some(p) => std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(p)
                            .map(Stdio::from)
                            .unwrap_or(Stdio::null()),
                        None => Stdio::null(),
                    })
                    .spawn();
                match child {
                    Ok(mut c) => {
                        let (so, si) = (c.stdout.take().unwrap(), c.stdin.take().unwrap());
                        // mainloop returns when the server dies — the
                        // editor must hear about it (silent death is a bug),
                        // unless we asked it to exit ourselves
                        let _ = mainloop.run_buffered(so.compat(), si.compat_write()).await;
                        if !quitting_mainloop.load(std::sync::atomic::Ordering::Relaxed) {
                            let _ = tx_fail.send(LspEvent::Failed { server: name, hint });
                        }
                    }
                    Err(_) => {
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
            if std::env::var_os("STROP_LSP_LOG").is_some() {
                eprintln!("strop-lsp: init = {:?}", init.as_ref().map(|_| "ok"));
            }
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
            thread: std::sync::Mutex::new(Some(thread)),
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

    /// The LSP exit sequence: shutdown request, then the exit
    /// notification. Called on editor quit — exiting without it makes
    /// servers die with "client exited without proper shutdown" and
    /// paints a fake failure on the statusline.
    pub fn shutdown(&self) {
        self.quitting
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let sock = self.socket.clone();
        self.handle.spawn(async move {
            let _ = sock
                .request::<async_lsp::lsp_types::request::Shutdown>(())
                .await;
            let _ = sock.notify::<async_lsp::lsp_types::notification::Exit>(());
        });
    }

    /// Join the runtime thread after `shutdown()`, with a timeout. A
    /// clean exit drops the client normally; on timeout we leak it on
    /// purpose — a detached thread that outlives the process is cheap,
    /// a dropped-socket panic in the user's terminal is not.
    pub fn wait(self, timeout: std::time::Duration) {
        let handle = self.thread.lock().ok().and_then(|mut t| t.take());
        let Some(handle) = handle else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = handle.join();
            let _ = tx.send(());
        });
        if rx.recv_timeout(timeout).is_err() {
            std::mem::forget(self);
        }
    }

    fn uri(&self, path: &Path) -> Option<Url> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        Url::from_file_path(abs).ok()
    }

    /// didOpen — full text, full sync (simplest correct; incremental sync
    /// is the perf follow-up, noted in 0009 §3).
    pub fn did_open(&self, path: &Path, language_id: &str, text: &str) {
        // initialize must hit the wire first — queue until it has
        if !self.initialized.load(std::sync::atomic::Ordering::Relaxed) {
            self.pending_opens.lock().push((
                path.to_path_buf(),
                language_id.to_string(),
                text.to_string(),
            ));
            return;
        }
        let Some(uri) = self.uri(path) else { return };
        self.versions.lock().insert(path.to_path_buf(), 1);
        let socket = self.socket.clone();
        let item = TextDocumentItem {
            uri,
            language_id: language_id.to_string(),
            version: 1,
            text: text.to_string(),
        };
        let _ = socket.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: item,
        });
    }

    /// The negotiated column encoding — callers convert at the boundary.
    pub fn encoding(&self) -> PositionEncoding {
        self.caps.encoding()
    }

    /// didChange — full document replacement (TextDocumentSyncKind::Full).
    pub fn did_change(&self, path: &Path, text: &str) {
        let Some(uri) = self.uri(path) else { return };
        let socket = self.socket.clone();
        // strictly increasing per the spec — "full sync doesn't care"
        // was wrong: pyright rejects stale versions (0014)
        let version = {
            let mut m = self.versions.lock();
            let v = m.entry(path.to_path_buf()).or_insert(1);
            *v += 1;
            *v
        };
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version },
            content_changes: vec![async_lsp::lsp_types::TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.to_string(),
            }],
        };
        let _ = socket.notify::<DidChangeTextDocument>(params);
    }

    /// Hover at (line, col) — UTF-8 converted at the boundary. The
    /// response posts onto the channel as HoverText (or nothing).
    /// Quiet no-op when the server doesn't advertise hover (0009 §2.5).
    pub fn hover(&self, path: &Path, line: usize, col: usize) {
        if !self.initialized.load(std::sync::atomic::Ordering::Relaxed) {
            self.pending_requests.lock().push(PendingRequest {
                path: path.to_path_buf(),
                line,
                col,
                kind: QueuedRequest::Hover,
            });
            return;
        }
        if !self.caps.hover() {
            return;
        }
        let Some(uri) = self.uri(path) else { return };
        let sock = self.socket.clone();
        let tx = self.tx.clone();
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: line as u32,
                    character: col as u32,
                },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        self.handle.spawn(async move {
            // -32801 "content modified": servers reject during their
            // initial index — one retry after a beat (helix does the same
            // class of dance)
            let mut resp = sock.request::<HoverRequest>(params.clone()).await;
            if is_content_modified(&resp) {
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                resp = sock.request::<HoverRequest>(params).await;
            }
            if std::env::var_os("STROP_LSP_LOG").is_some() {
                eprintln!(
                    "strop-lsp: hover = {:?}",
                    resp.as_ref().map(|h| h.is_some())
                );
            }
            if let Ok(Some(hover)) = resp {
                let text = hover_text(&hover);
                if !text.is_empty() {
                    let _ = tx.send(LspEvent::HoverText { text });
                }
            }
        });
    }

    /// Goto-definition; response posts as GotoLocation. Quiet no-op when
    /// the server doesn't advertise definitions (0009 §2.5).
    pub fn goto_definition(&self, path: &Path, line: usize, col: usize) {
        // pre-init: caps unknown ≠ unsupported — queue, flush on
        // Initialized (gd right after opening a project used to die
        // silently here)
        if !self.initialized.load(std::sync::atomic::Ordering::Relaxed) {
            self.pending_requests
                .lock()
                .push(PendingRequest {
                    path: path.to_path_buf(),
                    line,
                    col,
                    kind: QueuedRequest::Goto,
                });
            return;
        }
        if !self.caps.goto_definition() {
            return;
        }
        let Some(uri) = self.uri(path) else { return };
        let sock = self.socket.clone();
        let tx = self.tx.clone();
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
        self.handle.spawn(async move {
            let mut resp = sock.request::<GotoDefinition>(params.clone()).await;
            if is_content_modified(&resp) {
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                resp = sock.request::<GotoDefinition>(params).await;
            }
            log_line(format!("gd = {:?}", resp.as_ref().map(|r| r.is_some())));
            if let Ok(Some(resp)) = resp {
                use async_lsp::lsp_types::GotoDefinitionResponse as R;
                let loc = match resp {
                    R::Scalar(l) => Some(l),
                    R::Array(v) => v.into_iter().next(),
                    R::Link(v) => v
                        .into_iter()
                        .next()
                        .map(|l| async_lsp::lsp_types::Location {
                            uri: l.target_uri,
                            range: l.target_selection_range,
                        }),
                };
                if let Some(l) = loc {
                    if let Ok(path) = l.uri.to_file_path() {
                        let _ = tx.send(LspEvent::GotoLocation {
                            path,
                            line: l.range.start.line as usize,
                            col: l.range.start.character as usize,
                        });
                    }
                }
            }
        });
    }

    /// clangd's `textDocument/switchSourceHeader` (a clangd extension,
    /// absent from lsp-types' request set): the .cpp ↔ .h jump. The
    /// counterpart posts as GotoLocation at its top; "no counterpart"
    /// and unsupported servers surface as a Note, never an error.
    pub fn switch_source_header(&self, path: &Path) {
        if !self.initialized.load(std::sync::atomic::Ordering::Relaxed) {
            self.pending_requests.lock().push(PendingRequest {
                path: path.to_path_buf(),
                line: 0,
                col: 0,
                kind: QueuedRequest::SwitchHeader,
            });
            return;
        }
        let Some(uri) = self.uri(path) else { return };
        let sock = self.socket.clone();
        let tx = self.tx.clone();
        self.handle.spawn(async move {
            match sock
                .request::<SwitchSourceHeader>(TextDocumentIdentifier { uri })
                .await
            {
                Ok(Some(target)) => {
                    if let Ok(path) = target.to_file_path() {
                        let _ = tx.send(LspEvent::GotoLocation {
                            path,
                            line: 0,
                            col: 0,
                        });
                    }
                }
                Ok(None) => {
                    let _ = tx.send(LspEvent::Note {
                        text: "no header/source counterpart".into(),
                    });
                }
                Err(e) => {
                    let _ = tx.send(LspEvent::Note {
                        text: format!("switch source/header unsupported: {e}"),
                    });
                }
            }
        });
    }
}

fn is_content_modified<T>(resp: &Result<T, async_lsp::Error>) -> bool {
    matches!(
        resp,
        Err(async_lsp::Error::Response(e))
            if e.code == async_lsp::ErrorCode::from(-32801)
    )
}

/// Hover content → plain text (markdown flattened).
fn hover_text(hover: &async_lsp::lsp_types::Hover) -> String {
    use async_lsp::lsp_types::HoverContents;
    match &hover.contents {
        HoverContents::Markup(m) => m.value.clone(),
        HoverContents::Array(a) => a
            .iter()
            .map(|s| match s {
                async_lsp::lsp_types::MarkedString::String(s) => s.clone(),
                async_lsp::lsp_types::MarkedString::LanguageString(l) => l.value.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// LSP diagnostic → UTF-8-friendly Diag (positions are UTF-16 line/col;
/// the editor maps them through the rope at render time).
fn diag_from_lsp(d: &Diagnostic) -> Diag {
    Diag {
        line: d.range.start.line as usize,
        col: d.range.start.character as usize,
        end_line: d.range.end.line as usize,
        end_col: d.range.end.character as usize,
        severity: d
            .severity
            .map(|s| match s {
                async_lsp::lsp_types::DiagnosticSeverity::ERROR => 1,
                async_lsp::lsp_types::DiagnosticSeverity::WARNING => 2,
                async_lsp::lsp_types::DiagnosticSeverity::INFORMATION => 3,
                _ => 4,
            })
            .unwrap_or(3),
        message: d.message.clone(),
    }
}

/// Protocol logging goes to the file named by STROP_LSP_LOG — never the
/// TTY (an eprintln mid-frame clobbers the alt screen; caught in the demo).
pub fn log_line(line: String) {
    if let Some(p) = std::env::var_os("STROP_LSP_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
        {
            let _ = writeln!(f, "strop-lsp: {line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{to_byte_col, to_server_col, PositionEncoding, ServerCaps};
    use async_lsp::lsp_types::{
        DefinitionOptions, HoverProviderCapability, OneOf, ServerCapabilities,
    };

    #[test]
    fn capabilities_gate_hover_and_goto() {
        let caps = ServerCaps::default();
        // pre-initialize: capabilities unknown → requests must not race
        // server startup
        assert!(!caps.hover());
        assert!(!caps.goto_definition());

        // initialized, nothing advertised → both gated
        caps.set(ServerCapabilities::default());
        assert!(!caps.hover());
        assert!(!caps.goto_definition());

        // definition only → hover stays dropped
        caps.set(ServerCapabilities {
            definition_provider: Some(OneOf::Left(true)),
            ..Default::default()
        });
        assert!(!caps.hover());
        assert!(caps.goto_definition());

        // hover on, definition explicitly off
        caps.set(ServerCapabilities {
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            definition_provider: Some(OneOf::Left(false)),
            ..Default::default()
        });
        assert!(caps.hover());
        assert!(!caps.goto_definition());
    }

    #[test]
    fn option_shaped_providers_count_as_enabled() {
        let caps = ServerCaps::default();
        caps.set(ServerCapabilities {
            hover_provider: Some(HoverProviderCapability::Options(Default::default())),
            definition_provider: Some(OneOf::Right(DefinitionOptions {
                work_done_progress_options: Default::default(),
            })),
            ..Default::default()
        });
        assert!(caps.hover());
        assert!(caps.goto_definition());
    }

    #[test]
    fn column_encoding_roundtrips_unicode() {
        // the LSP wire is UTF-16 unless negotiated; strop is byte-native
        let line = "aé🦀b"; // bytes: 1+2+4+1, utf16: 1+1+2+1
                            // byte col of 'b' = 7; utf-16 col = 4
        assert_eq!(to_server_col(line, 7, PositionEncoding::Utf16), 4);
        assert_eq!(to_byte_col(line, 4, PositionEncoding::Utf16), 7);
        assert_eq!(to_server_col(line, 7, PositionEncoding::Utf8), 7);
        assert_eq!(to_byte_col(line, 7, PositionEncoding::Utf8), 7);
        // inside the emoji (byte 3..7): utf16 col 2..4
        assert_eq!(to_server_col(line, 3, PositionEncoding::Utf16), 2);
        assert_eq!(to_server_col(line, 7, PositionEncoding::Utf16), 4);
        assert_eq!(to_byte_col(line, 2, PositionEncoding::Utf16), 3);
        // past-the-end clamps
        assert_eq!(to_byte_col(line, 99, PositionEncoding::Utf16), line.len());
        // combining marks: e + U+0301 is 3 bytes, 2 utf-16 units
        let comb = "e\u{0301}x";
        assert_eq!(to_server_col(comb, 3, PositionEncoding::Utf16), 2);
        assert_eq!(to_byte_col(comb, 2, PositionEncoding::Utf16), 3);
    }
}
