//! Attach completion handling: outcome publication, refusal and sticky
//! bookkeeping, per-project warm status rows, and the status-line
//! message for every decision — silence is not a report.
use super::super::super::picker::ProjectStatus;
use super::super::attach::AttachRecord;
use super::super::*;
use super::layer_suffix;
use std::sync::mpsc::channel;

impl Editor {
    pub(crate) fn handle_lsp_attach(&mut self, record: AttachRecord) {
        trace_attach(&record);
        self.worker_handles.remove(&record.ticket);
        let key = self
            .lsp_state
            .attach
            .pending
            .iter()
            .find_map(|(key, &owner)| (owner == record.ticket).then(|| key.clone()));
        let Some(key) = key else {
            trace::services::rejected("lsp", "attach completion superseded");
            self.retire_superseded_transport(record.server);
            return;
        };
        self.lsp_state.attach.pending.remove(&key);
        // Warm-up completions feed per-project status rows (0063 §2);
        // document attach completions already have the status line.
        let warm = self.lsp_state.attach.warm_attempts.remove(&key);
        if key.target != record.target || key.language != record.language {
            self.retire_superseded_transport(record.server);
            trace::services::rejected("lsp", "attach result differs from its requested workspace");
            return;
        }
        let attach::AttachRecord {
            ticket: _,
            server,
            language,
            name,
            root,
            target,
            outcome,
            layers,
        } = record;
        // Malformed layers are diagnosed whatever the outcome (0033
        // §2): a healthy fallback server must not erase them.
        self.record_layer_diagnostics(&layers);
        let warning = layer_suffix(&layers);
        match outcome {
            attach::AttachDecision::Cancelled => {}
            attach::AttachDecision::Attached => {
                if warm {
                    self.record_project_status(key.path.clone(), ProjectStatus::healthy());
                }
                let Some(server) = server else { return };
                if self.lsp_state.attach.attached.iter().any(|attachment| {
                    attachment.language == language
                        && attachment.root == root
                        && attachment.target == target
                }) {
                    self.retire_superseded_transport(Some(server));
                    self.lsp_did_open_current();
                    return;
                }
                self.lsp_state
                    .attach
                    .attached
                    .retain(|a| !(a.language == language && a.root == root && a.target == target));
                // The placement exists before any didOpen resolves
                // against it — live and replayed alike.
                self.lsp_state.attach.attached.push(attach::Attachment {
                    language: language.clone(),
                    root: root.clone(),
                    server,
                    target: target.clone(),
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
                                    if tx
                                        .send(crate::editor::events::AppEvent::Lsp(event))
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            });
                            let (_, empty) = channel();
                            self.lsp_servers.push(LspServer {
                                id: server,
                                client: Some(client),
                                rx: empty,
                                ready: false,
                            });
                        } else {
                            self.lsp_servers.push(LspServer {
                                id: server,
                                client: Some(client),
                                rx,
                                ready: false,
                            });
                        }
                    }
                    None => {
                        // Replayed server: identity only, replies arrive
                        // through the injected event stream.
                        self.lsp_servers.push(LspServer {
                            id: server,
                            client: None,
                            rx: channel().1,
                            ready: false,
                        });
                    }
                }
                self.lsp_did_open_current();
            }
            decision => {
                if warm {
                    self.record_project_status(
                        key.path.clone(),
                        ProjectStatus::from_decision(&decision, &name),
                    );
                }
                if matches!(
                    decision,
                    attach::AttachDecision::TrustRequired { .. }
                        | attach::AttachDecision::TrustError { .. }
                ) {
                    self.lsp_state
                        .attach
                        .trust_roots
                        .insert(key.clone(), root.clone());
                }
                let sticky = !matches!(
                    decision,
                    attach::AttachDecision::TrustRequired { .. }
                        | attach::AttachDecision::TrustError { .. }
                        | attach::AttachDecision::RemoteIo { .. }
                );
                let first = self
                    .lsp_state
                    .attach
                    .refused
                    .insert(key, decision.clone())
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
                    attach::AttachDecision::RemoteIo { reason } => {
                        format!("lsp: remote discovery failed — {reason}")
                    }
                    attach::AttachDecision::Attached | attach::AttachDecision::Cancelled => {
                        unreachable!("matched above")
                    }
                };
                if let Some(warning) = warning {
                    self.message = format!("{} — {}", self.message, warning);
                }
            }
        }
        // A completion freed a warm-up slot: keep the bounded queue
        // moving (0063 §2).
        self.lsp_warm_progress();
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
            "target": record.target.label(),
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
            attach::AttachDecision::SpawnFailed { reason }
            | attach::AttachDecision::RemoteIo { reason } => {
                value["reason"] = serde_json::json!(reason);
            }
            _ => {}
        }
        value
    });
}
