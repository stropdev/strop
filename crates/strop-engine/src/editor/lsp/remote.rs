//! Remote LSP discovery over read-only SFTP metadata plus an admitted
//! endpoint worker for Git root and service execution. The editor's
//! local filesystem, local project layer and Python supervisor never
//! stand in for remote authority. All I/O is on the discovery job.
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;

use strop_core::worker::CancelToken;
use strop_remote::worker_transport::RemoteWorker;
use strop_remote::{ReadFailureKind, ReadLimit, ReadSelection, RemoteClient, RemoteOffset};
use strop_workspace::{RemoteEndpoint, RemoteFile, RemoteLocation};

use super::attach::{AttachDecision, AttachRecord, DiscoverInput, LiveTransport};
use strop_lsp::languages::{Languages, LayerDiagnostic, RemoteLayer};
use strop_lsp::registry;
use strop_lsp::{Client, ServerId, Workspace};

/// Bounded read for one candidate config file: languages.toml is
/// configuration, never a buffer — a slice is enough and a giant
/// remote file cannot balloon the editor.
const CONFIG_READ_LIMIT_BYTES: u64 = 256 * 1024;

/// What a candidate lookup on the remote filesystem told us.
enum Fetch {
    Found(Vec<u8>),
    Missing,
    Cancelled,
    /// The candidate exists but cannot serve as a layer.
    Failed(String),
    /// The owned connection itself failed — never misread as "no
    /// config".
    Transport(String),
}

/// One bounded read of a small remote file over the owned connection.
/// The seed file carries the endpoint identity; `with_path` keeps the
/// same endpoint with a sibling native path — no local probe, ever.
fn fetch_small(
    client: &RemoteClient,
    seed: &RemoteFile,
    path: &Path,
    token: &CancelToken,
) -> Fetch {
    let file = match seed.with_path(path.to_path_buf()) {
        Ok(file) => file,
        Err(error) => return Fetch::Failed(format!("not a canonical remote path: {error}")),
    };
    let selection = match ReadLimit::new(CONFIG_READ_LIMIT_BYTES) {
        Ok(length) => ReadSelection::Range {
            start: RemoteOffset::new(0),
            length,
        },
        Err(error) => return Fetch::Failed(error.to_string()),
    };
    match client.read(&RemoteLocation::from(file), selection, token) {
        Ok(snapshot) if snapshot.window.is_complete() => {
            Fetch::Found(snapshot.buffer.text().to_string().into_bytes())
        }
        Ok(_) => {
            Fetch::Failed("remote language configuration exceeds its bounded read limit".into())
        }
        Err(error) => match error.kind() {
            ReadFailureKind::NotFound => Fetch::Missing,
            ReadFailureKind::Cancelled => Fetch::Cancelled,
            // The connection, not the candidate, failed: stopping the
            // search must be a typed refusal — never a "no config"
            // guess.
            ReadFailureKind::Auth
            | ReadFailureKind::Trust
            | ReadFailureKind::Network
            | ReadFailureKind::Connect
            | ReadFailureKind::Deadline
            | ReadFailureKind::Spawn
            | ReadFailureKind::Subsystem => Fetch::Transport(error.to_string()),
            // The candidate exists but is unusable (unreadable, not
            // UTF-8, too large, wrong type): a malformed layer.
            _ => Fetch::Failed(error.to_string()),
        },
    }
}

/// The remote project layer search result.
enum LayerSearch {
    /// A readable candidate (path on the remote filesystem, bytes).
    Found(PathBuf, Vec<u8>),
    /// A candidate exists but is unusable — the diagnostic names it.
    Malformed(LayerDiagnostic),
    None,
    Cancelled,
    /// The owned connection failed before the search could finish.
    Io(String),
}

/// Walk up from the trigger file's directory looking for the remote
/// `.strop/languages.toml` — the remote project layer. Every hop is a
/// bounded read on the already-owned connection; the local project
/// layer of the editor's own cwd is never consulted (0036). The walk
/// is bounded by the path's own depth.
fn find_project_layer(
    client: &RemoteClient,
    seed: &RemoteFile,
    abs: &Path,
    token: &CancelToken,
) -> LayerSearch {
    let Some(mut dir) = abs.parent().map(Path::to_path_buf) else {
        return LayerSearch::None;
    };
    let endpoint = seed.endpoint();
    loop {
        let candidate = dir.join(".strop").join("languages.toml");
        match fetch_small(client, seed, &candidate, token) {
            Fetch::Found(bytes) => return LayerSearch::Found(candidate, bytes),
            Fetch::Missing => {}
            Fetch::Cancelled => return LayerSearch::Cancelled,
            Fetch::Transport(reason) => return LayerSearch::Io(reason),
            Fetch::Failed(reason) => {
                // A candidate that exists but cannot be read or parsed
                // is a malformed layer — diagnosed, never silent, and
                // the search stops here like the local walker's first
                // `is_file` hit.
                return LayerSearch::Malformed(LayerDiagnostic::remote_layer(
                    endpoint,
                    &candidate,
                    format!("{reason} — layer ignored"),
                ));
            }
        }
        if !dir.pop() {
            return LayerSearch::None;
        }
    }
}

/// A trusted project layer owns its root. Without one, only an
/// already-admitted endpoint worker may query Git; no Python probe
/// or guessed local repository path. Git's own no-repository result
/// falls back to the trigger directory, while transport loss stays
/// a typed discovery failure.
fn workspace_root(
    endpoint: &RemoteEndpoint,
    parent: &Path,
    worker: Option<&RemoteWorker>,
    token: &CancelToken,
) -> Result<PathBuf, String> {
    let Some(worker) = worker else {
        return Ok(parent.to_path_buf());
    };
    strop_git::remote::discover(endpoint, parent, Some(worker), token)
        .map(|found| found.unwrap_or_else(|| parent.to_path_buf()))
        .map_err(|error| error.to_string())
}

/// Remote attach discovery: fetch the remote project layer → resolve
/// the spec through local-XDG + remote-project layers → remote root →
/// remote trust → remote executability → spawn on the endpoint.
/// `None` means the attempt was cancelled; everything else is a record
/// the editor reports identically live and replayed.
pub(super) fn discover(
    input: &DiscoverInput,
    file: &RemoteFile,
    client: &RemoteClient,
    worker: &Option<RemoteWorker>,
    token: &CancelToken,
) -> Option<AttachRecord> {
    let DiscoverInput {
        ticket,
        ext,
        language,
        state_dir,
        xdg,
        transport,
        ..
    } = input;
    let endpoint = file.endpoint().clone();
    let abs = file.path().to_path_buf();
    let target = strop_workspace::Filesystem::Remote(endpoint.clone());
    let parent = abs.parent().map(Path::to_path_buf).unwrap_or_default();
    let record = |server: Option<ServerId>,
                  name: String,
                  root: PathBuf,
                  outcome: AttachDecision,
                  layers: Vec<LayerDiagnostic>| AttachRecord {
        ticket: *ticket,
        server,
        language: language.to_string(),
        name,
        root,
        target: target.clone(),
        outcome,
        layers,
    };
    // 1. The remote project layer, fetched over the owned connection —
    //    never a local project layer, never a local probe.
    let mut layers: Vec<LayerDiagnostic> = Vec::new();
    let project = match find_project_layer(client, file, &abs, token) {
        LayerSearch::Found(path, bytes) => Some((path, bytes)),
        LayerSearch::Malformed(diagnostic) => {
            layers.push(diagnostic);
            None
        }
        LayerSearch::None => None,
        LayerSearch::Cancelled => return None,
        LayerSearch::Io(reason) => {
            // The connection failed mid-search: a typed, non-sticky
            // refusal — the next attach attempt re-discovers.
            return Some(record(
                None,
                language.to_string(),
                parent,
                AttachDecision::RemoteIo { reason },
                layers,
            ));
        }
    };
    // 2. Layers: the trusted local XDG layer may select the command;
    //    the remote project layer rides along as bytes with its
    //    endpoint named in every diagnostic.
    let languages = Languages::load_remote(
        xdg.as_deref(),
        project.as_ref().map(|(path, bytes)| RemoteLayer {
            endpoint: &endpoint,
            path,
            bytes,
        }),
    );
    layers.extend(languages.layer_diagnostics().iter().cloned());
    // 3. Root: the dir holding a remote project layer, else remote
    //    git, else the trigger file's own directory.
    let root = match project.as_ref().map(|(path, _)| path) {
        Some(config) => config
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| parent.clone()),
        None => match workspace_root(&endpoint, &parent, worker.as_ref(), token) {
            Ok(root) => root,
            Err(_) if token.is_cancelled() => return None,
            Err(reason) => {
                return Some(record(
                    None,
                    language.to_string(),
                    parent,
                    AttachDecision::RemoteIo { reason },
                    layers,
                ))
            }
        },
    };
    // 4. Resolve the server through the merged layers.
    let Some(spec) = registry::for_extension(ext, &languages) else {
        return Some(record(
            None,
            language.to_string(),
            root,
            AttachDecision::NoServer,
            layers,
        ));
    };
    let name = spec.name.to_string();
    // 5. A remote project layer naming a command is executable content
    //    from another host: the existing trust gate, keyed by endpoint
    //    + remote root — never by a path that could alias a local one.
    if spec.project_executable {
        match crate::session::is_trusted_remote(state_dir.as_deref(), &endpoint, &root) {
            Ok(true) => {}
            Ok(false) => {
                return Some(record(
                    None,
                    name,
                    root,
                    AttachDecision::TrustRequired {
                        command: spec.command.to_string(),
                    },
                    layers,
                ))
            }
            Err(error) => {
                return Some(record(
                    None,
                    name,
                    root,
                    AttachDecision::TrustError {
                        error: error.to_string(),
                    },
                    layers,
                ))
            }
        }
    }
    // 6. The worker owns actual server launch/readiness. An unadmitted
    // host refuses at Client::spawn; a missing program is classified by
    // the worker's real exec attempt, never by a shell or local probe.
    if worker
        .as_ref()
        .is_some_and(|lease| lease.endpoint() != &endpoint)
    {
        return Some(record(
            None,
            name,
            root,
            AttachDecision::RemoteIo {
                reason: "remote LSP lease belongs to a different endpoint".into(),
            },
            layers,
        ));
    }
    if token.is_cancelled() {
        return None;
    }
    // 7. Spawn in the endpoint's namespace with the bound lease.
    let (tx, rx) = channel();
    match Client::spawn(
        &spec,
        Workspace::Remote {
            endpoint: endpoint.clone(),
            root: root.clone(),
        },
        tx,
        worker.as_ref().map(|lease| lease.worker().clone()),
    ) {
        Ok(client) => {
            let server = client.id();
            if let Ok(mut table) = transport.lock() {
                table.insert(server, LiveTransport { client, rx });
            }
            Some(record(
                Some(server),
                name,
                root,
                AttachDecision::Attached,
                layers,
            ))
        }
        Err(error) => Some(record(
            None,
            name,
            root,
            AttachDecision::SpawnFailed {
                reason: error.to_string(),
            },
            layers,
        )),
    }
}
