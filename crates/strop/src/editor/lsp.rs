//! Editor-side LSP event handling and asynchronous navigation.
use crate::editor::lsp::attach::AttachRecord;

use super::{trace, Editor};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use strop_lsp::protocol::ResolvedDiag;
use strop_lsp::registry;
use strop_lsp::{LspEvent, ServerId};

pub(crate) mod attach;
pub(crate) mod state;
#[cfg(test)]
mod tests;

pub(crate) struct LspServer {
    pub id: ServerId,
    /// None for replayed servers: identity and replies come from the
    /// injected record/event stream — never a fake client.
    pub client: Option<strop_lsp::Client>,
    pub rx: Receiver<LspEvent>,
}

impl Editor {
    /// Try to attach a language server for the current buffer. The
    /// synchronous part only touches in-memory state; discovery
    /// (config, root, trust, executability) is owned worker work behind the
    /// replay gate.
    /// The LSP half of the startup "start services" action: enable
    /// attach, then attach for the current buffer. A pure state
    /// transition performed identically live and replayed — the tape
    /// gates the native discovery inside `lsp_maybe_attach`.
    pub fn lsp_start_services(&mut self) {
        self.lsp_state.attach.enabled = true;
        self.lsp_maybe_attach();
    }

    pub(crate) fn lsp_maybe_attach(&mut self) {
        if !self.lsp_state.attach.enabled {
            return;
        }
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let Some(ext) = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
        else {
            return;
        };
        let Some(language) = registry::language_for_extension(&ext) else {
            return;
        };
        let abs = self.cwd.join(&path);
        if self.lsp_server_for(&abs, language).is_some() {
            self.lsp_did_open_current();
            return;
        }
        match self.lsp_state.attach.refused.get(language) {
            // Trust decisions change (`:trust`); re-discover those.
            Some(attach::AttachDecision::TrustRequired { .. })
            | Some(attach::AttachDecision::TrustError { .. }) => {}
            // Everything else was reported once and stays refused.
            Some(_) => return,
            None => {}
        }
        if self.lsp_state.attach.pending.contains_key(language) {
            return;
        }
        let ticket = match self.worker_ids.allocate() {
            Ok(ticket) => ticket,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        self.lsp_state
            .attach
            .pending
            .insert(language.to_string(), ticket);
        let args = attach::AttachArgs {
            ticket,
            path: abs.clone(),
            language: language.to_string(),
        };
        // Replay gate: no native config/trust/executability before this
        // registration (R11).
        match self.tape.request("lsp.attach", &args) {
            Ok(true) => self.lsp_spawn_discovery(ticket, abs, ext, language),
            Ok(false) => {}
            Err(error) => {
                self.lsp_state.attach.pending.remove(language);
                self.message = format!("lsp attach diverged from trace: {error}");
            }
        }
    }

    fn lsp_spawn_discovery(
        &mut self,
        ticket: strop_core::worker::WorkerId,
        abs: PathBuf,
        ext: String,
        language: &'static str,
    ) {
        let input = attach::DiscoverInput {
            ticket,
            abs,
            ext,
            language,
            cwd: self.cwd.clone(),
            git_workdir: self.git.as_ref().map(|g| g.workdir().to_path_buf()),
            state_dir: self.state_dir.clone(),
            xdg: strop_lsp::languages::xdg_path(),
            transport: self.lsp_state.attach.transport.clone(),
        };
        let done = self.lsp_state.attach.attach_channel();
        let spawned = std::thread::Builder::new()
            .name("strop-lsp-attach".into())
            .spawn(move || {
                let _ = done.send(attach::discover(input));
            });
        if spawned.is_err() {
            self.lsp_state.attach.pending.remove(language);
            self.message = "lsp: cannot start attach discovery".into();
        }
    }

    /// The server placement for a buffer, if one is attached: exact
    /// language match, longest covering root wins.
    pub(crate) fn lsp_server_for(
        &self,
        abs: &Path,
        language: &'static str,
    ) -> Option<(ServerId, PathBuf)> {
        let attach = &self.lsp_state.attach;
        let best = attach
            .attached
            .iter()
            .filter(|a| a.language == language && abs.starts_with(&a.root))
            .max_by_key(|a| a.root.as_os_str().len())?;
        Some((best.server, best.root.clone()))
    }

    pub(crate) fn handle_lsp_attach(&mut self, record: AttachRecord) {
        trace_attach(&record);
        let language_key = record.language.clone();
        // Stale completion: a newer attempt owns this language now.
        if self.lsp_state.attach.pending.get(&language_key) != Some(&record.ticket) {
            trace::services::rejected("lsp", "attach completion superseded");
            self.retire_superseded_transport(record.server);
            return;
        }
        self.lsp_state.attach.pending.remove(&language_key);
        let attach::AttachRecord {
            ticket: _,
            server,
            language,
            name,
            root,
            outcome,
            layers,
        } = record;
        // Malformed layers are diagnosed whatever the outcome (0033
        // §2): a healthy fallback server must not erase them.
        self.record_layer_diagnostics(&layers);
        let warning = layer_suffix(&layers);
        match outcome {
            attach::AttachDecision::Attached => {
                let Some(server) = server else { return };
                self.lsp_state
                    .attach
                    .attached
                    .retain(|a| !(a.language == language && a.root == root));
                // The placement exists before any didOpen resolves
                // against it — live and replayed alike.
                self.lsp_state.attach.attached.push(attach::Attachment {
                    language: language.clone(),
                    root: root.clone(),
                    server,
                });
                match warning {
                    Some(warning) => self.message = format!("lsp: {warning}"),
                    None => self.message = format!("lsp: {name} starting"),
                }
                let transport = self
                    .lsp_state
                    .attach
                    .transport
                    .lock()
                    .ok()
                    .and_then(|mut table| table.remove(&server));
                match transport {
                    Some(attach::LiveTransport { client, rx }) => {
                        // TUI: forward like every late-attaching server.
                        if let Some(app_tx) = &self.app_tx {
                            let tx = app_tx.clone();
                            std::thread::spawn(move || {
                                while let Ok(event) = rx.recv() {
                                    if tx.send(super::events::AppEvent::Lsp(event)).is_err() {
                                        break;
                                    }
                                }
                            });
                            let (_, empty) = channel();
                            self.lsp_servers.push(LspServer {
                                id: server,
                                client: Some(client),
                                rx: empty,
                            });
                        } else {
                            self.lsp_servers.push(LspServer {
                                id: server,
                                client: Some(client),
                                rx,
                            });
                        }
                        self.lsp_did_open_current();
                    }
                    None => {
                        // Replayed server: identity only, replies arrive
                        // through the injected event stream.
                        self.lsp_servers.push(LspServer {
                            id: server,
                            client: None,
                            rx: channel().1,
                        });
                    }
                }
            }
            decision => {
                let sticky = !matches!(
                    decision,
                    attach::AttachDecision::TrustRequired { .. }
                        | attach::AttachDecision::TrustError { .. }
                );
                let first = self
                    .lsp_state
                    .attach
                    .refused
                    .insert(language.clone(), decision.clone())
                    .is_none();
                if sticky && !first {
                    return;
                }
                self.message = match decision {
                    attach::AttachDecision::NoServer => format!("no language server for {name}"),
                    attach::AttachDecision::TrustRequired { command } => {
                        format!("project config wants to run `{command}` — :trust to allow (once)")
                    }
                    attach::AttachDecision::TrustError { error } => {
                        format!("project trust: {error}")
                    }
                    attach::AttachDecision::NotExecutable {
                        command,
                        reason,
                        hint,
                    } => {
                        format!("lsp: {command} {reason} — {hint}")
                    }
                    attach::AttachDecision::SpawnFailed { reason } => {
                        format!("lsp: {name} could not start — {reason}")
                    }
                    attach::AttachDecision::Attached => unreachable!("matched above"),
                };
                if let Some(warning) = warning {
                    self.message = format!("{} — {}", self.message, warning);
                }
            }
        }
    }

    /// Record newly reported malformed-layer diagnostics (0033 §2),
    /// deduped: every later attach for the same layers is already
    /// covered.
    fn record_layer_diagnostics(&mut self, layers: &[strop_lsp::languages::LayerDiagnostic]) {
        for diagnostic in layers {
            let state = &mut self.lsp_state.attach;
            if !state.layer_diagnostics.contains(diagnostic) {
                state.layer_diagnostics.push(diagnostic.clone());
            }
        }
    }

    /// The first recorded layer diagnostic, when any — readiness and
    /// later messages must not erase it (0033 §2).
    fn layer_warning(&self) -> Option<String> {
        layer_suffix(&self.lsp_state.attach.layer_diagnostics)
    }

    /// A superseded discovery may already have published a live
    /// transport for its server: retire it so the attempt leaves no
    /// orphan process and no undrained event stream.
    fn retire_superseded_transport(&mut self, server: Option<ServerId>) {
        let Some(server) = server else { return };
        let transport = self
            .lsp_state
            .attach
            .transport
            .lock()
            .ok()
            .and_then(|mut table| table.remove(&server));
        if let Some(attach::LiveTransport { client, .. }) = transport {
            // Joining never blocks the input thread (same policy as
            // lsp_failed).
            std::thread::spawn(move || {
                client.shutdown();
                client.wait(std::time::Duration::from_secs(2));
            });
        }
    }

    pub(crate) fn handle_lsp_event(&mut self, event: LspEvent) {
        trace::services::lsp(&event);
        match event {
            LspEvent::Ready { server, name } => {
                if self.lsp_servers.iter().any(|s| s.id == server) {
                    // Success must not erase a configuration warning
                    // (0033 §2): readiness is reported alongside it.
                    self.message = match self.layer_warning() {
                        Some(warning) => format!("lsp: {name} ready — {warning}"),
                        None => format!("lsp: {name} ready"),
                    };
                }
            }
            LspEvent::Failed { server, name, hint } => {
                if self.lsp_servers.iter().any(|s| s.id == server) {
                    self.lsp_failed(server);
                    self.message = format!("lsp: {name} failed — {hint}");
                } else {
                    trace::services::rejected("lsp", "failure for an unowned server");
                }
            }
            LspEvent::Diagnostics {
                context,
                path,
                diags,
            } => {
                let valid = self
                    .lsp_state
                    .bindings
                    .get(&context.document)
                    .is_some_and(|b| {
                        b.server == context.server
                            && b.path == path
                            && b.revision == context.revision
                    });
                let Some(doc) = self
                    .docs
                    .get(context.document)
                    .filter(|d| valid && d.buf.revision() == context.revision)
                else {
                    trace::services::rejected("lsp", "diagnostic owner/revision changed");
                    return;
                };
                let buffer = &doc.buf;
                let resolved: Vec<ResolvedDiag> = diags
                    .into_iter()
                    .map(|d| d.resolve(context.encoding, buffer))
                    .collect();
                self.diags.insert(path, resolved);
            }
            LspEvent::HoverText { context, text } => {
                if !self.lsp_reply_fresh(&context) {
                    trace::services::rejected(
                        "lsp",
                        "hover request/server/document/revision changed",
                    );
                    return;
                }
                self.lsp_state.hover = None;
                self.hover_card = Some(text);
            }
            LspEvent::Note { context, text } => {
                if !self.lsp_reply_fresh(&context) {
                    trace::services::rejected(
                        "lsp",
                        "navigation request/server/document/revision changed",
                    );
                    return;
                }
                self.lsp_state.navigation = None;
                self.message = text;
            }
            LspEvent::GotoLocation { context, location } => {
                if !self.lsp_reply_fresh(&context) {
                    trace::services::rejected(
                        "lsp",
                        "navigation request/server/document/revision changed",
                    );
                    return;
                }
                self.jump_to_location(location, context);
            }
            LspEvent::Locations {
                context,
                kind,
                items,
            } => {
                if !self.lsp_reply_fresh(&context) {
                    trace::services::rejected(
                        "lsp",
                        "locations request/server/document/revision changed",
                    );
                    return;
                }
                match items.len() {
                    0 => {
                        self.lsp_state.navigation = None;
                        self.message = format!("lsp: no {}", kind.label());
                    }
                    1 => {
                        if let Some(location) = items.into_iter().next() {
                            self.jump_to_location(location, context);
                        }
                    }
                    n => {
                        use strop_picker::{Item, Kind, Payload};
                        let items = items
                            .into_iter()
                            .map(|location| {
                                let line = location.position.line.get() + 1;
                                let col = location.position.column.get() + 1;
                                Item {
                                    text: format!("{}:{}:{}", location.path.display(), line, col),
                                    payload: Payload::Grep {
                                        path: location.path,
                                        line,
                                        col,
                                        match_len: 1,
                                        line_text: String::new(),
                                    },
                                }
                            })
                            .collect();
                        let mut glue = super::PickerGlue::diagnostics(strop_picker::Picker::new(
                            Kind::Locations,
                            items,
                            false,
                        ));
                        glue.lsp_context = Some(context);
                        self.set_picker(glue);
                        self.message = format!("{n} {}", kind.label());
                    }
                }
            }
        }
    }

    pub(crate) fn jump_to_location(
        &mut self,
        location: strop_lsp::ServerLocation,
        context: strop_lsp::ReplyContext,
    ) {
        if !self.lsp_reply_fresh(&context) {
            return;
        }
        self.request_open(
            location.path,
            super::io::OpenIntent::LspLocation {
                context,
                position: location.position,
            },
        );
    }

    pub(crate) fn finish_lsp_jump(
        &mut self,
        target: strop_core::id::DocumentId,
        position: strop_lsp::ServerPosition,
        context: strop_lsp::ReplyContext,
    ) {
        if !self.lsp_reply_fresh(&context) {
            trace::services::rejected("lsp", "navigation changed while target was loading");
            return;
        }
        let Some(target_doc) = self.docs.get(target) else {
            return;
        };
        let Some(binding) = self.lsp_state.bindings.get(&context.stamp.document) else {
            return;
        };
        let outside = target_doc
            .buf
            .path
            .as_ref()
            .is_some_and(|path| !self.cwd.join(path).starts_with(&binding.root));
        let line = position
            .line
            .get()
            .min(target_doc.buf.len_lines().saturating_sub(1));
        let text = target_doc.buf.line_text(line);
        let col = strop_lsp::to_byte_col(&text, position.column, context.encoding).get();
        let head = target_doc
            .buf
            .clamp_boundary(target_doc.buf.line_start(line).saturating_add(col));
        self.push_jump();
        self.lsp_state.navigation = None;
        self.switch_to(target);
        if outside && !self.buf().readonly {
            self.buf_mut().readonly = true;
            self.message = "readonly — outside workspace (:set noro to edit)".into();
        }
        self.set_head(head);
        self.clamp_cursor();
        self.scroll_to_cursor(self.view_rows());
        self.lsp_maybe_attach();
    }

    pub(crate) fn lsp_locations(&mut self, kind: strop_lsp::LocKind) {
        self.lsp_request(strop_lsp::RequestKind::Locations(kind));
    }
    pub(crate) fn lsp_hover(&mut self) {
        self.lsp_request(strop_lsp::RequestKind::Hover);
    }
    pub(crate) fn lsp_goto_definition(&mut self) {
        self.lsp_request(strop_lsp::RequestKind::Goto);
    }
    pub(crate) fn lsp_switch_source_header(&mut self) {
        self.lsp_request(strop_lsp::RequestKind::SwitchHeader);
    }

    pub(crate) fn jump_diagnostic(&mut self, forward: bool) {
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let abs = self.cwd.join(path);
        let Some(diags) = self.diags.get(&abs).filter(|d| !d.is_empty()) else {
            self.message = "no diagnostics".into();
            return;
        };
        let cur = self.buf().line_of(self.head());
        let col = self.buf().col_of(self.head());
        let target = if forward {
            diags
                .iter()
                .find(|d| d.line.get() > cur || (d.line.get() == cur && d.col.get() > col))
                .or(diags.first())
        } else {
            diags
                .iter()
                .rev()
                .find(|d| d.line.get() < cur || (d.line.get() == cur && d.col.get() < col))
                .or(diags.last())
        };
        let Some(d) = target else {
            return;
        };
        let (line, col, msg) = (d.line.get(), d.col.get(), d.message.clone());
        let start = self
            .buf()
            .line_start(line.min(self.buf().len_lines().saturating_sub(1)));
        self.set_head(self.buf().clamp_boundary(start + col));
        self.clamp_cursor();
        self.scroll_to_cursor(self.view_rows());
        self.message = msg;
    }

    pub(crate) fn open_diagnostics_picker(&mut self) {
        use strop_picker::{Item, Kind, Payload};
        // Deterministic row order across hash seeds (R11).
        let mut by_path: Vec<(&PathBuf, &Vec<ResolvedDiag>)> = self.diags.iter().collect();
        by_path.sort_by(|a, b| a.0.cmp(b.0));
        let items: Vec<Item> = by_path
            .into_iter()
            .flat_map(|(path, diags)| {
                diags.iter().map(move |d| Item {
                    text: format!(
                        "{}:{} {} {}",
                        path.display(),
                        d.line.get() + 1,
                        d.severity_char(),
                        d.message
                    ),
                    payload: Payload::Grep {
                        path: path.clone(),
                        line: d.line.get() + 1,
                        col: d.col.get() + 1,
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
        self.set_picker(super::PickerGlue::diagnostics(strop_picker::Picker::new(
            Kind::Diagnostics,
            items,
            false,
        )));
    }

    pub(crate) fn lsp_goto_definition_pub(&mut self) {
        self.lsp_goto_definition();
    }
    pub(crate) fn lsp_switch_source_header_pub(&mut self) {
        self.lsp_switch_source_header();
    }
    pub(crate) fn lsp_hover_pub(&mut self) {
        self.lsp_hover();
    }
    pub fn lsp_locations_pub(&mut self, kind: strop_lsp::LocKind) {
        self.lsp_locations(kind);
    }
    pub fn jump_diagnostic_pub(&mut self, forward: bool) {
        self.jump_diagnostic(forward);
    }
}

/// Modeline suffix for malformed layers: the first diagnostic's exact
/// path, plus a count when more follow (0033 §2).
fn layer_suffix(layers: &[strop_lsp::languages::LayerDiagnostic]) -> Option<String> {
    let first = layers.first()?;
    Some(if layers.len() == 1 {
        first.display()
    } else {
        format!("{} (+{} more)", first.display(), layers.len() - 1)
    })
}

/// Attach completions reach the structured trace with their outcome
/// and any malformed-layer diagnostics (0033 §2/§3) — silence is not a
/// report. Runs at handler entry, before ownership decisions.
fn trace_attach(record: &attach::AttachRecord) {
    use strop_trace::{record_with, EventKind};
    record_with(EventKind::JobFinished, || {
        let mut value = serde_json::json!({
            "service": "lsp",
            "result": "attach",
            "outcome": record.outcome.label(),
            "language": record.language,
            "name": record.name,
            "server": record.server,
            "root": trace::services::NativePath(record.root.clone()),
            "layers": &record.layers,
        });
        match &record.outcome {
            attach::AttachDecision::NotExecutable {
                command,
                reason,
                hint,
            } => {
                value["command"] = serde_json::json!(command);
                value["reason"] = serde_json::json!(reason);
                value["hint"] = serde_json::json!(hint);
            }
            attach::AttachDecision::SpawnFailed { reason } => {
                value["reason"] = serde_json::json!(reason);
            }
            _ => {}
        }
        value
    });
}

/// The LSP language for a path, from the embedded extension table —
/// pure, in-memory, safe on every keystroke.
pub(crate) fn lsp_language(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    registry::language_for_extension_name(ext)
}

/// The didOpen languageId sent to servers.
pub(crate) fn lang_id(path: &Path) -> &'static str {
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
