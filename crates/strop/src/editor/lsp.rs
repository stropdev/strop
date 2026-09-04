//! LSP glue (0009): the editor drains typed events; tokio never touches
//! the input path. Diagnostics merge into the git gutter (severity wins),
//! Space d is the diagnostics picker, Space k hover, gd goto-definition.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::OnceLock;

use strop_lsp::languages::Languages;
use strop_lsp::registry::{self, ServerSpec};
use strop_lsp::{Client, LspEvent};

use super::Editor;

/// One pooled server connection (0014 wave 2): the pool keys on
/// (workspace root, server name) so a polyglot session runs rust-analyzer
/// and clangd and pyright side by side.
pub(crate) struct LspServer {
    pub key: (PathBuf, String),
    pub client: strop_lsp::Client,
    pub rx: Receiver<strop_lsp::LspEvent>,
}

impl Editor {
    /// Spawn a server for the current buffer if the merged languages
    /// config (0012: project > XDG > embedded) resolves one. One client
    /// per workspace root; buffers did_open on it.
    pub(crate) fn lsp_maybe_attach(&mut self) {
        if cfg!(test) {
            return; // hermetic test builds never spawn servers
        }
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let ext = Path::new(&path)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()));
        let Some(ext) = ext else { return };
        let abs = if Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            self.cwd.join(&path)
        };
        let languages: &'static Languages = merged_languages(&abs);
        let warn = languages.warnings();
        let Some(spec) = registry::for_extension(&ext, languages) else {
            if !warn.is_empty() {
                self.message = format!("languages.toml: {}", warn.join("; "));
            }
            return;
        };
        // already pooled for this (root, server)? just did_open — the
        // root resolve repeats the languages/git walk, cheap and rare
        let root_known = self.lsp_server_root(&abs, languages);
        let key = (root_known, spec.name.to_string());
        if self.lsp_servers.iter().any(|s| s.key == key) {
            self.lsp_did_open_current();
            return;
        }
        // PATH probe only for bare command names — a config-provided
        // absolute path is its own existence check (0012 §5)
        if !spec.absolute_command()
            && std::process::Command::new(spec.command)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_err()
        {
            self.lsp_hint_once(spec);
            return;
        }
        let (tx, rx) = channel();
        match Client::spawn(&spec, &key.0.clone(), tx) {
            Some(client) => {
                self.lsp_servers.push(LspServer { key, client, rx });
                self.message = format!("lsp: {} starting", spec.name);
                if !warn.is_empty() {
                    self.message
                        .push_str(&format!(" — languages.toml: {}", warn.join("; ")));
                }
                self.lsp_did_open_current();
            }
            None => self.lsp_hint_once(spec),
        }
    }

    fn lsp_hint_once(&mut self, spec: ServerSpec<'static>) {
        if self.lsp_hints_shown.insert(spec.name) {
            let hint = spec
                .install_hint
                .unwrap_or("install it or fix the command in languages.toml");
            self.message = format!("lsp: {} not available — {}", spec.name, hint);
        }
    }

    /// The buffer's workspace root (project layer, git walk, cwd).
    fn lsp_server_root(&self, abs: &Path, languages: &Languages) -> PathBuf {
        languages
            .project_root
            .as_deref()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| {
                let fallback = self
                    .git
                    .as_ref()
                    .map(|g| g.workdir().to_path_buf())
                    .unwrap_or_else(|| self.cwd.clone());
                registry::workspace_root(abs, &fallback)
            })
    }

    /// The pooled server for the current buffer, resolved through its
    /// extension + root (None when nothing serves this file).
    fn lsp_current(&self) -> Option<&LspServer> {
        let path = self.buf().path.as_deref()?;
        let abs = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.cwd.join(path)
        };
        let ext = format!(".{}", abs.extension()?.to_string_lossy());
        let languages: &'static Languages = merged_languages(&abs);
        let spec = registry::for_extension(&ext, languages)?;
        let root = self.lsp_server_root(&abs, languages);
        let key = (root, spec.name.to_string());
        self.lsp_servers.iter().find(|s| s.key == key)
    }

    fn lsp_did_open_current(&mut self) {
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let abs = if std::path::Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            self.cwd.join(&path)
        };
        if !self.lsp_opened.insert(abs.clone()) {
            return;
        }
        let Some(client) = self.lsp_current().map(|s| &s.client) else {
            return;
        };
        let lang = lang_id(&abs);
        client.did_open(&abs, lang, &self.buf().rope.to_string());
    }

    /// didChange when the buffer epoch moved (debounced by epoch — one
    /// full sync per edit burst; incremental sync is the perf follow-up).
    pub fn lsp_sync_changed(&mut self) {
        // buffers can be empty post-quit (the TUI breaks first, but
        // scripted/headless callers may drain past the last :q)
        let Some(buf) = self.docs.get(self.current).map(|d| &d.buf) else {
            return;
        };
        let Some(path) = buf.path.clone() else {
            return;
        };
        let abs = if std::path::Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            self.cwd.join(&path)
        };
        let epoch = buf.epoch;
        if self.lsp_sent_epochs.get(&abs) == Some(&epoch) {
            return;
        }
        self.lsp_sent_epochs.insert(abs.clone(), epoch);
        let text = buf.rope.to_string();
        let Some(client) = self.lsp_current().map(|s| &s.client) else {
            return;
        };
        client.did_change(&abs, &text);
    }

    /// Drain server events (event loop tick + headless settle).
    pub fn drain_lsp(&mut self) {
        // drain every pooled server, remembering which one each event
        // came from (its negotiated encoding converts the columns)
        let mut events: Vec<(strop_lsp::PositionEncoding, LspEvent)> = Vec::new();
        for srv in &self.lsp_servers {
            while let Ok(ev) = srv.rx.try_recv() {
                events.push((srv.client.encoding(), ev));
            }
        }
        for (enc, event) in events {
            match event {
                LspEvent::Diagnostics { path, mut diags } => {
                    // server columns → byte columns against the open
                    // buffer's text (unopened files keep wire values)
                    {
                        if let Some(buf) = self.buffer_for_path(&path) {
                            for d in &mut diags {
                                let line = buf.line_text(d.line);
                                d.col = strop_lsp::to_byte_col(&line, d.col, enc);
                                let end_line = buf.line_text(d.end_line);
                                d.end_col = strop_lsp::to_byte_col(&end_line, d.end_col, enc);
                            }
                        }
                    }
                    self.diags.insert(path, diags);
                }
                LspEvent::Ready { server } => {
                    self.message = format!("lsp: {server} ready");
                }
                LspEvent::Failed { server, hint } => {
                    self.message = format!("lsp: {server} failed — {hint}");
                }
                LspEvent::Note { text } => self.message = text,
                LspEvent::HoverText { text } => self.hover_card = Some(text),
                LspEvent::GotoLocation { path, line, col } => {
                    if std::env::var_os("STROP_LSP_LOG").is_some() {
                        eprintln!("strop: goto {}:{}:{}", path.display(), line, col);
                    }
                    let path_s = path.display().to_string();
                    if let Err(e) = self.open_buffer(&path_s) {
                        self.message = format!("open {path_s}: {e}");
                    } else {
                        // server col → byte col against the target line
                        let col = {
                            let line_idx = line.min(self.buf().len_lines().saturating_sub(1));
                            let text = self.buf().line_text(line_idx);
                            strop_lsp::to_byte_col(&text, col, enc)
                        };
                        let start = self.buf().line_start(line.min(self.buf().len_lines() - 1));
                        self.cursor = self.buf().clamp_boundary(start + col);
                        self.clamp_cursor();
                    }
                }
            }
        }
    }

    /// `Space d`: diagnostics picker over the current buffer's diags.
    pub(crate) fn open_diagnostics_picker(&mut self) {
        use strop_picker::{Item, Kind, Payload};
        let items: Vec<Item> = self
            .diags
            .iter()
            .flat_map(|(path, diags)| {
                diags.iter().map(move |d| Item {
                    text: format!(
                        "{}:{} {} {}",
                        path.display(),
                        d.line + 1,
                        d.severity_char(),
                        d.message
                    ),
                    payload: Payload::Grep {
                        path: path.clone(),
                        line: d.line + 1,
                        col: d.col + 1,
                        match_len: 1,
                        line_text: d.message.clone(),
                    },
                })
            })
            .collect();
        if items.is_empty() {
            self.message = "no diagnostics".into();
            return;
        }
        let picker = strop_picker::Picker::new(Kind::Diagnostics, items, false);
        self.picker = Some(crate::editor::PickerGlue::diagnostics(picker));
    }

    /// `Space k`: hover at the cursor.
    pub(crate) fn lsp_hover(&mut self) {
        let Some(client) = self.lsp_current().map(|s| &s.client) else {
            self.message = "no language server — install it or fix languages.toml".into();
            return;
        };
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let abs = if std::path::Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            self.cwd.join(&path)
        };
        let line = self.buf().line_of(self.cursor);
        let col = self.buf().col_of(self.cursor);
        let col = self.server_col(client, col);
        client.hover(&abs, line, col);
    }

    /// The open buffer backing an absolute path, if any.
    fn buffer_for_path(&self, abs: &std::path::Path) -> Option<&strop_core::Buffer> {
        self.docs.iter().map(|(_, d)| &d.buf).find(|b| {
            b.path.as_deref().is_some_and(|p| {
                let p = std::path::Path::new(p);
                let buf_abs = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    self.cwd.join(p)
                };
                buf_abs == abs || buf_abs.canonicalize().ok().as_deref() == Some(abs)
            })
        })
    }

    /// byte col → the server's negotiated column for the current line.
    fn server_col(&self, client: &strop_lsp::Client, byte_col: usize) -> usize {
        let line = self.buf().line_of(self.cursor);
        let text = self.buf().line_text(line);
        strop_lsp::to_server_col(&text, byte_col, client.encoding())
    }

    /// `gd`: goto definition at the cursor.
    pub(crate) fn lsp_goto_definition(&mut self) {
        let Some(client) = self.lsp_current().map(|s| &s.client) else {
            self.message = "no language server — install it or fix languages.toml".into();
            return;
        };
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let abs = if std::path::Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            self.cwd.join(&path)
        };
        let line = self.buf().line_of(self.cursor);
        let col = self.buf().col_of(self.cursor);
        let col = self.server_col(client, col);
        client.goto_definition(&abs, line, col);
    }

    /// `gs`: switch between source and header (clangd's extension).
    pub(crate) fn lsp_switch_source_header(&mut self) {
        let Some(client) = self.lsp_current().map(|s| &s.client) else {
            self.message = "no language server — install it or fix languages.toml".into();
            return;
        };
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let abs = if std::path::Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            self.cwd.join(&path)
        };
        client.switch_source_header(&abs);
    }
}

/// The merged languages.toml layers (0012: project > XDG > embedded),
/// loaded once per process at first attach — never on the input path.
/// One project layer per session matches the one-server-per-workspace
/// model; the buffer anchors the project-file walk. OnceLock (not
/// LazyLock) because the initializer needs that runtime buffer path.
fn merged_languages(buffer: &Path) -> &'static Languages {
    static MERGED: OnceLock<Languages> = OnceLock::new();
    MERGED.get_or_init(|| {
        let xdg = strop_lsp::languages::xdg_path();
        let project = strop_lsp::languages::project_path(buffer);
        Languages::load(xdg.as_deref(), project.as_deref())
    })
}

/// LSP language id for a path (the registry's languages, lsp-named).
fn lang_id(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("py") | Some("pyi") => "python",
        Some("go") => "go",
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript",
        Some("ts") => "typescript",
        Some("tsx") => "typescriptreact",
        Some("json") => "json",
        Some("sh") | Some("bash") => "shellscript",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") | Some("hh") => "cpp",
        _ => "plaintext",
    }
}
