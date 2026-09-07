//! Asynchronous attach discovery (R6): config layers, workspace root,
//! trust and the executability check run on a worker thread — never on
//! the dispatch path. Completion is a pure serializable `AttachRecord`
//! (R11); the live transport is handed over through a side table keyed
//! by server identity, so replay injects records without spawning or
//! faking a client.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use strop_core::worker::WorkerId;
use strop_lsp::languages::LayerDiagnostic;
use strop_lsp::registry::{self, ServerSpec};
use strop_lsp::{Client, LspEvent, ServerId};

/// A live connection produced by discovery: the client handle plus the
/// event stream every server owns.
pub(crate) struct LiveTransport {
    pub client: Client,
    pub rx: Receiver<LspEvent>,
}

/// What discovery was asked to do — the tape records this before any
/// native config/trust/executability work runs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AttachArgs {
    pub ticket: WorkerId,
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
    pub language: String,
}

/// The serializable outcome of one attach attempt. Refusals carry
/// `server: None`; a spawned (or replayed) server keeps its identity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AttachRecord {
    pub ticket: WorkerId,
    pub server: Option<ServerId>,
    pub language: String,
    pub name: String,
    #[serde(with = "strop_core::path_serde")]
    pub root: PathBuf,
    pub outcome: AttachDecision,
    /// Malformed layer diagnostics met while loading the config layers
    /// for this attempt (0033 §2) — reported even when a valid
    /// fallback server attached.
    #[serde(default)]
    pub layers: Vec<LayerDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum AttachDecision {
    Attached,
    NoServer,
    TrustRequired {
        command: String,
    },
    TrustError {
        error: String,
    },
    NotExecutable {
        #[serde(default)]
        command: String,
        #[serde(default)]
        reason: String,
        hint: String,
    },
    SpawnFailed {
        #[serde(default)]
        reason: String,
    },
}

impl AttachDecision {
    /// Stable trace label for attach completions.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Attached => "attached",
            Self::NoServer => "no_server",
            Self::TrustRequired { .. } => "trust_required",
            Self::TrustError { .. } => "trust_error",
            Self::NotExecutable { .. } => "not_executable",
            Self::SpawnFailed { .. } => "spawn_failed",
        }
    }
}

/// One live/replayed server placement: a language inside a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub language: String,
    pub root: PathBuf,
    pub server: ServerId,
}

pub(crate) struct AttachState {
    /// Services start explicitly (the startup action), never as a
    /// constructor side effect — live and replayed runs perform the
    /// same transition. Default false: pure test editors never spawn.
    pub enabled: bool,
    /// Attach discovery in flight: language → owning ticket. Later
    /// attempts replace the ticket, so stale completions are refused.
    pub pending: HashMap<String, WorkerId>,
    /// Terminal refusals per language. Trust refusals re-check on every
    /// attach attempt (`:trust` must work); the rest are reported once.
    pub refused: HashMap<String, AttachDecision>,
    /// Malformed layer diagnostics ever reported by discovery, deduped
    /// — a healthy Ready must not erase them (0033 §2).
    pub layer_diagnostics: Vec<LayerDiagnostic>,
    /// Servers placed for (language, root) — live or replayed.
    pub attached: Vec<Attachment>,
    /// Live transports published by discovery workers, keyed by server.
    pub transport: Arc<Mutex<HashMap<ServerId, LiveTransport>>>,
    pub rx: Receiver<AttachRecord>,
    tx: Sender<AttachRecord>,
}

impl AttachState {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            enabled: false,
            pending: HashMap::new(),
            refused: HashMap::new(),
            layer_diagnostics: Vec::new(),
            attached: Vec::new(),
            transport: Arc::new(Mutex::new(HashMap::new())),
            rx,
            tx,
        }
    }

    /// Hand the completion channel to the event forwarder (TUI); the
    /// headless drains read `rx` directly.
    pub fn take_rx(&mut self) -> Receiver<AttachRecord> {
        let (_, empty) = channel();
        std::mem::replace(&mut self.rx, empty)
    }

    /// A sender for one more discovery worker.
    pub fn attach_channel(&self) -> Sender<AttachRecord> {
        self.tx.clone()
    }
}

/// Everything the discovery worker owns for one attempt.
pub(crate) struct DiscoverInput {
    pub ticket: WorkerId,
    pub abs: PathBuf,
    pub ext: String,
    pub language: &'static str,
    pub cwd: PathBuf,
    pub git_workdir: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    /// The resolved XDG layer path (`languages::xdg_path()`), injected
    /// so tests never read a real HOME.
    pub xdg: Option<PathBuf>,
    pub transport: Arc<Mutex<HashMap<ServerId, LiveTransport>>>,
}

/// Native discovery: config layers → spec → workspace root → trust →
/// executability → spawn. Runs entirely on the discovery worker
/// thread; the only editor contact is the completion record (and, on
/// success, the side-table transport).
pub(crate) fn discover(input: DiscoverInput) -> AttachRecord {
    let DiscoverInput {
        ticket,
        abs,
        ext,
        language,
        cwd,
        git_workdir,
        state_dir,
        xdg,
        transport,
    } = input;
    let languages = strop_lsp::languages::Languages::load(
        xdg.as_deref(),
        strop_lsp::languages::project_path(&abs).as_deref(),
    );
    // Malformed layers ride along with every outcome (0033 §2) — even
    // a healthy fallback attach must keep diagnosing them.
    let layers: Vec<LayerDiagnostic> = languages.layer_diagnostics().to_vec();
    let refused = |outcome: AttachDecision, name: String, root: PathBuf| AttachRecord {
        ticket,
        server: None,
        language: language.to_string(),
        name,
        root,
        outcome,
        layers: layers.clone(),
    };
    let Some(spec) = registry::for_extension(&ext, &languages) else {
        return refused(AttachDecision::NoServer, language.to_string(), cwd.clone());
    };
    let name = spec.name.to_string();
    let root = match languages.project_root.as_deref() {
        Some(root) => root.to_path_buf(),
        None => match git_workdir.as_deref() {
            Some(workdir) => workdir.to_path_buf(),
            None => registry::workspace_root(&abs, &cwd),
        },
    };
    if let Some(outcome) = trust_refusal(&spec, state_dir.as_deref(), &root) {
        return refused(outcome, name, root);
    }
    // The executability check is pure metadata (0033 §3): no process is
    // spawned, so no orphan probe exists and an untrusted project
    // command is never executed merely to test it.
    match registry::command_status(&spec, &root, std::env::var_os("PATH").as_deref()) {
        registry::CommandStatus::Executable => {}
        registry::CommandStatus::Unrunnable(reason) => {
            let decision = AttachDecision::NotExecutable {
                command: spec.command.to_string(),
                reason: reason.to_string(),
                hint: install_hint(&spec),
            };
            return refused(decision, name, root);
        }
    }
    let (tx, rx) = channel();
    match Client::spawn(&spec, &root, tx) {
        Ok(client) => {
            let server = client.id();
            if let Ok(mut table) = transport.lock() {
                table.insert(server, LiveTransport { client, rx });
            }
            AttachRecord {
                ticket,
                server: Some(server),
                language: language.to_string(),
                name,
                root,
                outcome: AttachDecision::Attached,
                layers,
            }
        }
        Err(error) => refused(
            AttachDecision::SpawnFailed {
                reason: error.to_string(),
            },
            name,
            root,
        ),
    }
}

fn trust_refusal(
    spec: &ServerSpec<'_>,
    state_dir: Option<&std::path::Path>,
    root: &std::path::Path,
) -> Option<AttachDecision> {
    if !spec.project_executable {
        return None;
    }
    match crate::session::is_trusted(state_dir, root) {
        Ok(true) => None,
        Ok(false) => Some(AttachDecision::TrustRequired {
            command: spec.command.to_string(),
        }),
        Err(error) => Some(AttachDecision::TrustError {
            error: error.to_string(),
        }),
    }
}

fn install_hint(spec: &ServerSpec<'_>) -> String {
    match spec.install_hint {
        Some(hint) => hint.to_string(),
        None => format!(
            "install `{}` or fix the command in languages.toml",
            spec.command
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        dir: &std::path::Path,
        xdg: Option<PathBuf>,
        transport: Arc<Mutex<HashMap<ServerId, LiveTransport>>>,
    ) -> DiscoverInput {
        DiscoverInput {
            ticket: WorkerId::new(1),
            abs: dir.join("src/main.rs"),
            ext: ".rs".into(),
            language: "rust",
            cwd: dir.to_path_buf(),
            git_workdir: None,
            state_dir: None,
            xdg,
            transport,
        }
    }

    /// 0033 §3: a config-defined absolute command that does not exist
    /// refuses before any spawn, naming the command — no PATH or HOME
    /// dependence, no process.
    #[test]
    fn missing_absolute_command_refuses_without_spawning() {
        let dir = std::env::temp_dir().join("strop-attach-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let xdg = dir.join("languages.toml");
        std::fs::write(
            &xdg,
            "[language-server.ghost]\ncommand = \"/nonexistent/ghost-lsp\"\n\
             \n[language.rust]\nlanguage-servers = [\"ghost\"]\n",
        )
        .unwrap();
        let transport = Arc::new(Mutex::new(HashMap::new()));
        let record = discover(input(&dir, Some(xdg), transport.clone()));
        assert_eq!(record.server, None);
        assert!(record.layers.is_empty());
        assert_eq!(
            record.outcome,
            AttachDecision::NotExecutable {
                command: "/nonexistent/ghost-lsp".into(),
                reason: "no such file".into(),
                hint: "install `/nonexistent/ghost-lsp` or fix the command in languages.toml"
                    .into(),
            }
        );
        // a refusal publishes no transport
        assert!(transport.lock().is_ok_and(|table| table.is_empty()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 0033 §2: a malformed project layer's typed diagnostic rides
    /// along even with a refusal — here the trust gate, which never
    /// touches the disk without a state directory.
    #[test]
    fn malformed_layer_diagnostic_rides_a_refusal() {
        let dir = std::env::temp_dir().join("strop-attach-malformed");
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("proj");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::create_dir_all(project.join(".strop")).unwrap();
        let broken = project.join(".strop/languages.toml");
        std::fs::write(&broken, "language-server = 3").unwrap();
        let xdg = dir.join("languages.toml");
        std::fs::write(
            &xdg,
            "[language-server.ghost]\ncommand = \"/nonexistent/ghost-lsp\"\n\
             \n[language.rust]\nlanguage-servers = [\"ghost\"]\n",
        )
        .unwrap();
        let transport = Arc::new(Mutex::new(HashMap::new()));
        let record = discover(input(&project, Some(xdg), transport.clone()));
        // the broken project layer reports with its exact path
        assert_eq!(record.layers.len(), 1);
        assert_eq!(record.layers[0].path, broken);
        // the project layer makes the command project-executable: the
        // trust gate refuses before executability is even checked
        assert!(matches!(
            record.outcome,
            AttachDecision::TrustRequired { .. }
        ));
        assert!(transport.lock().is_ok_and(|table| table.is_empty()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
