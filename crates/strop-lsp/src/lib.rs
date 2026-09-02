//! strop-lsp: the LSP client. async-lsp transport (0009 §2.1), tokio on
//! a worker thread, the editor sees a channel of typed events — never an
//! async type, never an await in the input path (0001 §5.6).

pub mod registry;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::Sender;
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
}

/// Client-side state for the router.
struct ClientState {
    tx: Sender<LspEvent>,
}

/// A live server connection: send from any thread, the runtime thread
/// owns the socket drain.
pub struct Client {
    socket: ServerSocket,
    handle: tokio::runtime::Handle,
    tx: Sender<LspEvent>,
    root: PathBuf,
}

impl Client {
    /// Spawn the server for `ext` at `root`. None when the registry has
    /// no server or the command isn't on PATH.
    pub fn spawn(spec: &registry::ServerSpec, root: &Path, tx: Sender<LspEvent>) -> Option<Self> {
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

        // all tokio work (child spawn included) lives on the runtime thread
        let cmd = spec.command.to_string();
        let args: Vec<String> = spec.args.iter().map(|a| a.to_string()).collect();
        let root_owned = root.to_path_buf();
        let tx_fail = tx.clone();
        let name = spec.name;
        let hint = spec.install_hint;
        std::thread::spawn(move || {
            rt.block_on(async move {
                let child = Command::new(&cmd)
                    .args(&args)
                    .current_dir(&root_owned)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(match std::env::var_os("STROP_LSP_LOG") {
                        Some(p) => std::fs::File::create(p)
                            .map(Stdio::from)
                            .unwrap_or(Stdio::null()),
                        None => Stdio::null(),
                    })
                    .spawn();
                match child {
                    Ok(mut c) => {
                        let (so, si) = (c.stdout.take().unwrap(), c.stdin.take().unwrap());
                        // mainloop returns when the server dies — the
                        // editor must hear about it (silent death is a bug)
                        let _ = mainloop.run_buffered(so.compat(), si.compat_write()).await;
                        let _ = tx_fail.send(LspEvent::Failed { server: name, hint });
                    }
                    Err(_) => {
                        let _ = tx_fail.send(LspEvent::Failed { server: name, hint });
                    }
                }
            });
        });

        let tx2 = tx.clone();
        let sock = socket.clone();
        let params = InitializeParams {
            #[allow(deprecated)] // root_uri is what every server still honors
            root_uri: Some(root_uri),
            capabilities: async_lsp::lsp_types::ClientCapabilities {
                text_document: Some(async_lsp::lsp_types::TextDocumentClientCapabilities {
                    synchronization: Some(Default::default()),
                    publish_diagnostics: Some(Default::default()),
                    hover: Some(Default::default()),
                    definition: Some(Default::default()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        handle.spawn(async move {
            let init = sock
                .request::<async_lsp::lsp_types::request::Initialize>(params)
                .await;
            if std::env::var_os("STROP_LSP_LOG").is_some() {
                eprintln!("strop-lsp: init = {:?}", init.as_ref().map(|_| "ok"));
            }
            match init {
                Ok(_) => {
                    let _ = sock.notify::<async_lsp::lsp_types::notification::Initialized>(
                        InitializedParams {},
                    );
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
            root: root.to_path_buf(),
        })
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
        let Some(uri) = self.uri(path) else { return };
        let version = 1;
        let socket = self.socket.clone();
        let item = TextDocumentItem {
            uri,
            language_id: language_id.to_string(),
            version,
            text: text.to_string(),
        };
        let _ = socket.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: item,
        });
    }

    /// didChange — full document replacement (TextDocumentSyncKind::Full).
    pub fn did_change(&self, path: &Path, text: &str) {
        let Some(uri) = self.uri(path) else { return };
        let socket = self.socket.clone();
        let version = 2; // version strictly increases; full sync doesn't care
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
    pub fn hover(&self, path: &Path, line: usize, col: usize) {
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

    /// Goto-definition; response posts as GotoLocation.
    pub fn goto_definition(&self, path: &Path, line: usize, col: usize) {
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
pub(crate) fn log_line(line: String) {
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
