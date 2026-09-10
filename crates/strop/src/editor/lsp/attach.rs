//! Asynchronous attach discovery (R6): config layers, workspace root,
//! trust and the executability check run on a worker thread — never on
//! the dispatch path. Completion is a pure serializable `AttachRecord`
//! (R11); the live transport is handed over through a side table keyed
//! by server identity, so replay injects records without spawning or
//! faking a client. Remote discovery (0036 RW8) lives in
//! [`super::remote`]; this module owns the shared record/key types and
//! the local path.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use strop_core::worker::{CancelToken, WorkerId};
use strop_lsp::languages::LayerDiagnostic;
use strop_lsp::registry::{self, ServerSpec};
use strop_lsp::{Client, LspEvent, ServerId};
use strop_workspace::Filesystem;

/// A live connection produced by discovery: the client handle plus the
/// event stream every server owns.
pub(crate) struct LiveTransport {
    pub client: Client,
    pub rx: Receiver<LspEvent>,
}

/// What discovery was asked to do — the tape records this before any
/// native config/trust/executability work runs. `target` distinguishes
/// the local workspace from a remote endpoint: the same language on
/// two filesystems is two different attempts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AttachArgs {
    pub ticket: WorkerId,
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
    pub language: String,
    #[serde(default)]
    pub target: Filesystem,
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
    #[serde(default)]
    pub target: Filesystem,
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
    Cancelled,
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
    /// Remote discovery infrastructure failed (owned connection lost,
    /// remote probe unreachable). Non-sticky: the next attach attempt
    /// re-discovers rather than caching a transient failure.
    RemoteIo {
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
            Self::Cancelled => "cancelled",
            Self::TrustRequired { .. } => "trust_required",
            Self::TrustError { .. } => "trust_error",
            Self::NotExecutable { .. } => "not_executable",
            Self::SpawnFailed { .. } => "spawn_failed",
            Self::RemoteIo { .. } => "remote_io",
        }
    }
}

/// One attach attempt's identity: filesystem target + language. A
/// remote workspace and a local one sharing a language never share a
/// pending attempt, refusal or placement (0036 RW8).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AttachKey {
    pub target: Filesystem,
    pub language: String,
    pub path: PathBuf,
}

/// One live/replayed server placement: a language inside a root on one
/// filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub language: String,
    pub root: PathBuf,
    pub server: ServerId,
    pub target: Filesystem,
}

pub(crate) struct AttachState {
    /// Services start explicitly (the startup action), never as a
    /// constructor side effect — live and replayed runs perform the
    /// same transition. Default false: pure test editors never spawn.
    pub enabled: bool,
    /// Attach discovery in flight: key → owning ticket. Later attempts
    /// replace the ticket, so stale completions are refused and the
    /// superseded worker is cancelled.
    pub pending: HashMap<AttachKey, WorkerId>,
    /// Terminal refusals per key. Trust and remote-io refusals
    /// re-check on every attach attempt (`:trust` must work;
    /// connections recover); the rest are reported once.
    pub refused: HashMap<AttachKey, AttachDecision>,
    pub trust_roots: HashMap<AttachKey, PathBuf>,
    /// Malformed layer diagnostics ever reported by discovery, deduped
    /// — a healthy Ready must not erase them (0033 §2).
    pub layer_diagnostics: Vec<LayerDiagnostic>,
    /// Servers placed for (target, language, root) — live or replayed.
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
            trust_roots: HashMap::new(),
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

/// Where discovery runs: the local workspace, or one remote endpoint
/// whose canonical trigger file is `file` and whose owned connection is
/// reachable through `client`.
pub(crate) enum DiscoverPlace {
    Local {
        abs: PathBuf,
        cwd: PathBuf,
        git_workdir: Option<PathBuf>,
    },
    Remote {
        file: strop_workspace::RemoteFile,
        client: strop_remote::RemoteClient,
    },
    /// A running container (0037 DC1b): the engine is local, the id is
    /// the canonical inspect id, root names container paths.
    Container {
        id: strop_workspace::ContainerId,
        root: PathBuf,
    },
}

/// Everything the discovery worker owns for one attempt.
pub(crate) struct DiscoverInput {
    pub ticket: WorkerId,
    pub place: DiscoverPlace,
    pub ext: String,
    pub language: &'static str,
    pub state_dir: Option<PathBuf>,
    /// The resolved XDG layer path (`languages::xdg_path()`), injected
    /// so tests never read a real HOME. Trusted local user
    /// configuration — the only local layer a remote attach reads.
    pub xdg: Option<PathBuf>,
    pub transport: Arc<Mutex<HashMap<ServerId, LiveTransport>>>,
}

/// Native discovery, dispatched by target: local layers → spec →
/// workspace root → trust → executability → spawn, or the remote
/// equivalent in [`super::remote`]. Runs entirely on the discovery
/// worker; the only editor contact is the completion record (and, on
/// success, the side-table transport). `None` means the attempt was
/// cancelled — nothing is reported.
pub(crate) fn discover(input: DiscoverInput, token: &CancelToken) -> Option<AttachRecord> {
    match &input.place {
        DiscoverPlace::Local {
            abs,
            cwd,
            git_workdir,
        } => Some(discover_local(&input, abs, cwd, git_workdir.as_deref())),
        DiscoverPlace::Remote { file, client } => {
            super::remote::discover(&input, file, client, token)
        }
        DiscoverPlace::Container { id, root } => Some(discover_container(&input, id, root)),
    }
}

/// Local discovery: config layers → spec → workspace root → trust →
/// executability → spawn.
fn discover_local(
    input: &DiscoverInput,
    abs: &Path,
    cwd: &Path,
    git_workdir: Option<&Path>,
) -> AttachRecord {
    let DiscoverInput {
        ticket,
        ext,
        language,
        state_dir,
        xdg,
        transport,
        ..
    } = input;
    let languages = strop_lsp::languages::Languages::load(
        xdg.as_deref(),
        strop_lsp::languages::project_path(abs).as_deref(),
    );
    // Malformed layers ride along with every outcome (0033 §2) — even
    // a healthy fallback attach must keep diagnosing them.
    let layers: Vec<LayerDiagnostic> = languages.layer_diagnostics().to_vec();
    let refused = |outcome: AttachDecision, name: String, root: PathBuf| AttachRecord {
        ticket: *ticket,
        server: None,
        language: language.to_string(),
        name,
        root,
        target: Filesystem::Local,
        outcome,
        layers: layers.clone(),
    };
    let Some(spec) = registry::for_extension(ext, &languages) else {
        return refused(
            AttachDecision::NoServer,
            language.to_string(),
            cwd.to_owned(),
        );
    };
    let name = spec.name.to_string();
    let root = match languages.project_root.as_deref() {
        Some(root) => root.to_path_buf(),
        None => match git_workdir {
            Some(workdir) => workdir.to_path_buf(),
            None => registry::workspace_root(abs, cwd),
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
    match Client::spawn(
        &spec,
        strop_lsp::Workspace::Local { root: root.clone() },
        tx,
    ) {
        Ok(client) => {
            let server = client.id();
            if let Ok(mut table) = transport.lock() {
                table.insert(server, LiveTransport { client, rx });
            }
            AttachRecord {
                ticket: *ticket,
                server: Some(server),
                language: language.to_string(),
                name,
                root,
                target: Filesystem::Local,
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

/// Container discovery (0037 DC1b): XDG/embedded layers only — an
/// in-container project languages.toml is deliberately not read yet (it
/// joins the trust gate when it is). The server binary's absence is
/// classified by the spawn, honestly, from the engine's own error.
fn discover_container(
    input: &DiscoverInput,
    id: &strop_workspace::ContainerId,
    root: &Path,
) -> AttachRecord {
    let languages = strop_lsp::languages::Languages::load(input.xdg.as_deref(), None);
    let layers: Vec<LayerDiagnostic> = languages.layer_diagnostics().to_vec();
    let target = Filesystem::Container(id.clone());
    let refused = |outcome: AttachDecision, name: String| AttachRecord {
        ticket: input.ticket,
        server: None,
        language: input.language.to_string(),
        name,
        root: root.to_path_buf(),
        target: target.clone(),
        outcome,
        layers: layers.clone(),
    };
    let Some(spec) = registry::for_extension(&input.ext, &languages) else {
        return refused(AttachDecision::NoServer, input.language.to_string());
    };
    let name = spec.name.to_string();
    let (tx, rx) = channel();
    match Client::spawn(
        &spec,
        strop_lsp::Workspace::Container {
            container: id.clone(),
            root: root.to_path_buf(),
        },
        tx,
    ) {
        Ok(client) => {
            let server = client.id();
            if let Ok(mut table) = input.transport.lock() {
                table.insert(server, LiveTransport { client, rx });
            }
            AttachRecord {
                ticket: input.ticket,
                server: Some(server),
                language: input.language.to_string(),
                name,
                root: root.to_path_buf(),
                target,
                outcome: AttachDecision::Attached,
                layers,
            }
        }
        Err(error) => refused(
            AttachDecision::SpawnFailed {
                reason: error.to_string(),
            },
            name,
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

pub(super) fn install_hint(spec: &ServerSpec<'_>) -> String {
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

    fn input(place: DiscoverPlace, ext: &str) -> DiscoverInput {
        DiscoverInput {
            ticket: WorkerId::new(0),
            place,
            ext: ext.into(),
            language: "nosuchlanguage",
            state_dir: None,
            xdg: None,
            transport: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[test]
    fn attach_keys_separate_local_from_remote_targets() {
        let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://builder.example").unwrap();
        let local = AttachKey {
            target: Filesystem::Local,
            language: "rust".into(),
            path: "/workspace/a.rs".into(),
        };
        let remote = AttachKey {
            target: Filesystem::Remote(endpoint),
            language: "rust".into(),
            path: "/workspace/a.rs".into(),
        };
        assert_ne!(local, remote);
        // Pending and refusal maps keyed this way never conflate a
        // local attach with a remote one for the same language.
        let mut pending = HashMap::new();
        pending.insert(local, WorkerId::new(1));
        assert!(!pending.contains_key(&remote));
    }

    #[test]
    fn attach_args_replay_legacy_local_records_as_local() {
        // Records from before remote targets existed carry no target
        // field; they must deserialize as local attempts.
        let legacy = r#"{"ticket":0,"path":"/w/a.rs","language":"rust"}"#;
        let args: AttachArgs = serde_json::from_str(legacy).unwrap();
        assert_eq!(args.target, Filesystem::Local);
    }

    #[test]
    fn decision_labels_are_stable() {
        assert_eq!(
            AttachDecision::RemoteIo { reason: "x".into() }.label(),
            "remote_io"
        );
        assert_eq!(AttachDecision::Attached.label(), "attached");
    }

    #[test]
    fn local_discovery_refuses_without_a_server() {
        // An extension no layer or registry entry covers: an honest
        // NoServer refusal, target local, no layer diagnostics.
        let dir = std::path::Path::new("/w/definitely-not-here");
        let abs = dir.join("a.nosuchlang");
        let record = discover_local(
            &input(
                DiscoverPlace::Local {
                    abs: abs.clone(),
                    cwd: dir.to_path_buf(),
                    git_workdir: None,
                },
                ".nosuchlang",
            ),
            &abs,
            dir,
            None,
        );
        assert_eq!(record.outcome, AttachDecision::NoServer);
        assert_eq!(record.target, Filesystem::Local);
        assert_eq!(record.root, dir);
        assert!(record.layers.is_empty());
    }
}
