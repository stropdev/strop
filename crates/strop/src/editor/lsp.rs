//! LSP glue (0009): the editor drains typed events; tokio never touches
//! the input path. Diagnostics merge into the git gutter (severity wins),
//! Space d is the diagnostics picker, Space k hover, gd goto-definition.

use std::path::PathBuf;
use std::sync::mpsc::channel;

use strop_lsp::registry::{self, ServerSpec};
use strop_lsp::{Client, LspEvent};

use super::Editor;

impl Editor {
    /// Spawn a server for the current buffer if the registry has one and
    /// it's on PATH. One client per workspace root; buffers did_open on it.
    pub(crate) fn lsp_maybe_attach(&mut self) {
        if cfg!(test) {
            return; // hermetic test builds never spawn servers
        }
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let ext = std::path::Path::new(&path)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()));
        let Some(ext) = ext else { return };
        let Some(spec) = registry::for_extension(&ext) else {
            return;
        };
        if self.lsp.is_some() {
            // already attached to a server for this root — just did_open
            self.lsp_did_open_current();
            return;
        }
        if std::process::Command::new(spec.command)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_err()
        {
            self.lsp_hint_once(spec);
            return;
        }
        let root = self
            .git
            .as_ref()
            .map(|g| g.workdir().to_path_buf())
            .unwrap_or_else(|| self.cwd.clone());
        let root = registry::workspace_root(PathBuf::from(&path).as_path(), &root);
        let (tx, rx) = channel();
        match Client::spawn(&spec, &root, tx) {
            Some(client) => {
                self.lsp = Some(client);
                self.lsp_rx = Some(rx);
                self.message = format!("lsp: {} starting", spec.name);
                self.lsp_did_open_current();
            }
            None => self.lsp_hint_once(spec),
        }
    }

    fn lsp_hint_once(&mut self, spec: ServerSpec) {
        if self.lsp_hints_shown.insert(spec.name) {
            self.message = format!("lsp: {} not on PATH — {}", spec.name, spec.install_hint);
        }
    }

    fn lsp_did_open_current(&mut self) {
        let Some(client) = &self.lsp else { return };
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
        let lang = lang_id(&abs);
        client.did_open(&abs, lang, &self.buf().rope.to_string());
    }

    /// didChange when the buffer epoch moved (debounced by epoch — one
    /// full sync per edit burst; incremental sync is the perf follow-up).
    pub fn lsp_sync_changed(&mut self) {
        let Some(client) = &self.lsp else { return };
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let abs = if std::path::Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            self.cwd.join(&path)
        };
        let epoch = self.buf().epoch;
        if self.lsp_sent_epochs.get(&abs) == Some(&epoch) {
            return;
        }
        self.lsp_sent_epochs.insert(abs.clone(), epoch);
        client.did_change(&abs, &self.buf().rope.to_string());
    }

    /// Drain server events (event loop tick + headless settle).
    pub fn drain_lsp(&mut self) {
        let mut rx = None;
        std::mem::swap(&mut self.lsp_rx, &mut rx);
        if let Some(rx) = &rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    LspEvent::Diagnostics { path, diags } => {
                        self.diags.insert(path, diags);
                    }
                    LspEvent::Ready { server } => {
                        self.message = format!("lsp: {server} ready");
                    }
                    LspEvent::Failed { server, hint } => {
                        self.message = format!("lsp: {server} failed — {hint}");
                    }
                    LspEvent::HoverText { text } => self.hover_card = Some(text),
                    LspEvent::GotoLocation { path, line, col } => {
                        if std::env::var_os("STROP_LSP_LOG").is_some() {
                            eprintln!("strop: goto {}:{}:{}", path.display(), line, col);
                        }
                        let path_s = path.display().to_string();
                        if let Err(e) = self.open_buffer(&path_s) {
                            self.message = format!("open {path_s}: {e}");
                        } else {
                            let start = self.buf().line_start(line.min(self.buf().len_lines() - 1));
                            self.cursor = self.buf().clamp_boundary(start + col);
                            self.clamp_cursor();
                        }
                    }
                }
            }
        }
        std::mem::swap(&mut self.lsp_rx, &mut rx);
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
        let Some(client) = &self.lsp else {
            self.message = "no language server".into();
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
        client.hover(
            &abs,
            self.buf().line_of(self.cursor),
            self.buf().col_of(self.cursor),
        );
    }

    /// `gd`: goto definition at the cursor.
    pub(crate) fn lsp_goto_definition(&mut self) {
        let Some(client) = &self.lsp else {
            self.message = "no language server".into();
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
        client.goto_definition(
            &abs,
            self.buf().line_of(self.cursor),
            self.buf().col_of(self.cursor),
        );
    }
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
