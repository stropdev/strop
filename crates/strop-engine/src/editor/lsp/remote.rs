//! Remote LSP discovery: the project layer is read over SFTP, the root
//! comes from `git` on the host, and the server runs there as a channel
//! on the endpoint's connection. The editor's local filesystem and local
//! project layer never stand in for the host. All I/O is on the
//! discovery job.
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;

use strop_core::worker::CancelToken;
use strop_remote::exec::ExecHost;
use strop_remote::{RemoteClient, RemoteFailureKind};
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
    match client.read(&RemoteLocation::from(file), CONFIG_READ_LIMIT_BYTES, token) {
        Ok(snapshot) => Fetch::Found(snapshot.buffer.text().to_string().into_bytes()),
        Err(error) => match error.kind() {
            RemoteFailureKind::NotFound => Fetch::Missing,
            RemoteFailureKind::Cancelled => Fetch::Cancelled,
            // The connection, not the candidate, failed: stopping the
            // search must be a typed refusal — never a "no config"
            // guess.
            RemoteFailureKind::Auth
            | RemoteFailureKind::Trust
            | RemoteFailureKind::Network
            | RemoteFailureKind::Connect
            | RemoteFailureKind::Deadline
            | RemoteFailureKind::Spawn
            | RemoteFailureKind::Subsystem => Fetch::Transport(error.to_string()),
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

/// A project layer owns its root. Without one, `git` on the host finds
/// it; git's own no-repository result falls back to the trigger
/// directory, while transport loss stays a typed discovery failure.
fn workspace_root(
    endpoint: &RemoteEndpoint,
    parent: &Path,
    host: &ExecHost,
    token: &CancelToken,
) -> Result<PathBuf, String> {
    strop_git::remote::discover(endpoint, parent, host, token)
        .map(|found| found.unwrap_or_else(|| parent.to_path_buf()))
        .map_err(|error| error.to_string())
}

/// Remote attach discovery: fetch the remote project layer → resolve
/// the spec through local-XDG + remote-project layers → remote root →
/// spawn on the endpoint. A connected host is trusted: the SSH login is
/// the consent, so a remote project layer's command runs there.
/// `None` means the attempt was cancelled; everything else is a record
/// the editor reports identically live and replayed.
pub(super) fn discover(
    input: &DiscoverInput,
    file: &RemoteFile,
    client: &RemoteClient,
    token: &CancelToken,
) -> Option<AttachRecord> {
    let DiscoverInput {
        ticket,
        ext,
        language,
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
        None => match client
            .host(&endpoint, token)
            .map_err(|error| error.to_string())
            .and_then(|host| workspace_root(&endpoint, &parent, &host, token))
        {
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
    let host = match client.host(&endpoint, token) {
        Ok(host) => host,
        Err(_) if token.is_cancelled() => return None,
        Err(error) => {
            return Some(record(
                None,
                name,
                root,
                AttachDecision::RemoteIo {
                    reason: error.to_string(),
                },
                layers,
            ))
        }
    };
    if token.is_cancelled() {
        return None;
    }
    // 5. Spawn on the host; a missing program is the host's own answer.
    let (tx, rx) = channel();
    match Client::spawn(
        &spec,
        Workspace::Remote {
            endpoint: endpoint.clone(),
            root: root.clone(),
        },
        tx,
        host.command(&spec.argv(), Some(&root)),
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
