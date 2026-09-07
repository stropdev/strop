//! Asynchronous attach discovery (R6): config layers, workspace root,
//! trust and the PATH probe run on a worker thread — never on the
//! dispatch path. Completion is a pure serializable `AttachRecord`
//! (R11); the live transport is handed over through a side table keyed
//! by server identity, so replay injects records without spawning or
//! faking a client.
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use strop_core::worker::WorkerId;
use strop_lsp::registry::{self, ServerSpec};
use strop_lsp::{Client, LspEvent, ServerId};

/// A live connection produced by discovery: the client handle plus the
/// event stream every server owns.
pub(crate) struct LiveTransport {
    pub client: Client,
    pub rx: Receiver<LspEvent>,
}

/// What discovery was asked to do — the tape records this before any
/// native config/trust/probe work runs.
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
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum AttachDecision {
    Attached,
    NoServer,
    TrustRequired { command: String },
    TrustError { error: String },
    NotExecutable { hint: String },
    SpawnFailed,
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
    pub transport: Arc<Mutex<HashMap<ServerId, LiveTransport>>>,
}

/// Native discovery: config layers → spec → workspace root → trust →
/// probe → spawn. Runs entirely on the discovery worker thread; the
/// only editor contact is the completion record (and, on success, the
/// side-table transport).
pub(crate) fn discover(input: DiscoverInput) -> AttachRecord {
    let DiscoverInput {
        ticket,
        abs,
        ext,
        language,
        cwd,
        git_workdir,
        state_dir,
        transport,
    } = input;
    let languages = strop_lsp::languages::Languages::load(
        strop_lsp::languages::xdg_path().as_deref(),
        strop_lsp::languages::project_path(&abs).as_deref(),
    );
    let refused = |outcome: AttachDecision, name: String, root: PathBuf| AttachRecord {
        ticket,
        server: None,
        language: language.to_string(),
        name,
        root,
        outcome,
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
    if let Some(record) = trust_refusal(&spec, state_dir.as_deref(), &root, &name, ticket, language)
    {
        return record;
    }
    if !spec.absolute_command() && probe(&spec).is_err() {
        let hint = install_hint(&spec);
        return refused(AttachDecision::NotExecutable { hint }, name, root);
    }
    let (tx, rx) = channel();
    match Client::spawn(&spec, &root, tx) {
        Some(client) => {
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
            }
        }
        None => refused(AttachDecision::SpawnFailed, name, root),
    }
}

fn trust_refusal(
    spec: &ServerSpec<'_>,
    state_dir: Option<&std::path::Path>,
    root: &std::path::Path,
    name: &str,
    ticket: WorkerId,
    language: &'static str,
) -> Option<AttachRecord> {
    if !spec.project_executable {
        return None;
    }
    let record = |outcome: AttachDecision| AttachRecord {
        ticket,
        server: None,
        language: language.to_string(),
        name: name.to_string(),
        root: root.to_path_buf(),
        outcome,
    };
    match crate::session::is_trusted(state_dir, root) {
        Ok(true) => None,
        Ok(false) => Some(record(AttachDecision::TrustRequired {
            command: spec.command.to_string(),
        })),
        Err(error) => Some(record(AttachDecision::TrustError {
            error: error.to_string(),
        })),
    }
}

/// The PATH probe: same semantics as `Client::spawn`'s internal one.
fn probe(spec: &ServerSpec<'_>) -> std::io::Result<()> {
    std::process::Command::new(spec.command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|child| child.id())
        .map(|_| ())
}

fn install_hint(spec: &ServerSpec<'_>) -> String {
    match spec.install_hint {
        Some(hint) => hint.to_string(),
        None => "install it or fix the command in languages.toml".to_string(),
    }
}
