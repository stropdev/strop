//! The SSH deployment provider (0058 WK07): strop-worker-deploy's thin
//! authenticated transfer surface over a dedicated SFTP connection plus
//! the [`crate::worker_transport`] exec handshake.
//!
//! Deployment is a rare, consent-gated, multi-step state machine, so it
//! does not borrow the read path's pooled per-endpoint actor (whose job
//! model is single bounded reads): it opens its own connection built by
//! the SAME crate ssh policy ([`crate::ssh::sftp_subsystem`]) and the
//! SAME SFTP v3 codec ([`crate::transport::wire`]) — one authentication
//! configuration, one wire interpretation, no second engine. The
//! connection is exclusively owned here and reaped on drop.
//!
//! Failure classification is load-bearing: SFTP status codes carry the
//! authority and are mapped onto the deploy crate's explicit outcomes
//! (not-found, permission); server message text only refines between
//! the explicit read-only/disk-full outcomes the state machine must
//! name. Parsed strings are never authority — verified bytes are.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use strop_core::process::OwnedProcess;
use strop_core::worker::CancelToken;
use strop_worker_deploy::provider::{
    DeployProvider, EndpointIdentity, HandshakeReport, ProviderError, RemoteKind, RemoteStat,
};
use strop_worker_protocol::EndpointInfo;
use strop_workspace::RemoteEndpoint;
use tokio::process::{ChildStdin, ChildStdout};
use tokio::runtime::Runtime;

use crate::bootstrap::EndpointFacts;
use crate::transport::error::{Fault, ReadFailureKind, ReadStage};
use crate::transport::wire::{DeploySftp, FileHandle};
use crate::worker_transport;

/// The upload stream's chunk size (one SFTP write request each).
const UPLOAD_CHUNK: usize = 128 * 1024;
/// Bounded ssh stderr retained for classified diagnostics.
const STDERR_LIMIT: usize = 64 * 1024;

/// The authenticated SFTP deployment surface for one SSH endpoint.
/// The session lives behind a mutex because the provider trait is
/// `&self`; one deploy run still drives it strictly sequentially.
pub struct SftpDeployProvider {
    identity: EndpointIdentity,
    endpoint: RemoteEndpoint,
    cache_base: String,
    /// Codec first: its pipes release before the runtime that drives
    /// them, and the child is reaped last.
    session: Mutex<DeploySftp<ChildStdin, ChildStdout>>,
    runtime: Runtime,
    child: Mutex<OwnedProcess>,
}

impl SftpDeployProvider {
    /// Open one dedicated SFTP connection to `endpoint` and bind the
    /// deployment identity to the discovered facts. The connection
    /// negotiates under the caller's cancellation; afterwards the
    /// provider owns it exclusively until drop.
    pub fn connect(
        endpoint: &RemoteEndpoint,
        facts: &EndpointFacts,
        token: &CancelToken,
    ) -> Result<Self, ProviderError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| ProviderError::Transport(format!("private runtime: {error}")))?;
        let mut command = crate::ssh::sftp_subsystem(endpoint);
        let mut child = OwnedProcess::spawn(&mut command, token).map_err(|failure| {
            if token.is_cancelled() {
                ProviderError::Transport(format!(
                    "deploy connection cancelled: {}",
                    failure.message
                ))
            } else {
                ProviderError::Transport(format!("ssh spawn: {}", failure.message))
            }
        })?;
        let stderr = spawn_stderr_drain(&mut child)?;
        let (stdin, stdout) = {
            let _context = runtime.enter();
            convert(&mut child)?
        };
        let session = match runtime.block_on(DeploySftp::connect(stdin, stdout)) {
            Ok(session) => session,
            Err(fault) => {
                let mut error = classify(fault);
                if let Some(detail) = stderr_text(&stderr) {
                    error = ProviderError::Transport(format!("{error}; ssh: {detail}"));
                }
                return Err(error);
            }
        };
        // The connection now stands on its own ownership: release the
        // caller's single cancel slot so later operations on the same
        // token can register (the pool actor's rule, mirrored). Drop
        // still terminates and reaps the child directly.
        token.clear_cancel_resource();
        Ok(Self {
            identity: EndpointIdentity {
                context: endpoint.to_string(),
                principal: facts.principal.clone(),
                target: facts.target.clone(),
            },
            endpoint: endpoint.clone(),
            cache_base: facts.cache_base.clone(),
            session: Mutex::new(session),
            runtime,
            child: Mutex::new(child),
        })
    }

    /// Lock the codec, drive one operation on the owning runtime and
    /// classify the outcome. The mutex is never held across a caller
    /// boundary.
    fn session(&self) -> std::sync::MutexGuard<'_, DeploySftp<ChildStdin, ChildStdout>> {
        self.session.lock().expect("deploy session poisoned")
    }
}

impl Drop for SftpDeployProvider {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.terminate();
            let _ = child.wait();
        }
    }
}

impl DeployProvider for SftpDeployProvider {
    fn endpoint(&self) -> &EndpointIdentity {
        &self.identity
    }

    fn cache_base(&self) -> Result<String, ProviderError> {
        Ok(self.cache_base.clone())
    }

    fn lstat(&self, path: &str) -> Result<Option<RemoteStat>, ProviderError> {
        let path = PathBuf::from(path);
        let mut session = self.session();
        let outcome = self.runtime.block_on(async { session.lstat(&path).await });
        match outcome.map_err(classify) {
            Ok(attrs) => Ok(Some(remote_stat(attrs)?)),
            Err(ProviderError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn mkdir_private(&self, path: &str) -> Result<(), ProviderError> {
        let path = PathBuf::from(path);
        let mut session = self.session();
        self.runtime
            .block_on(async { mkdir_recursive(&mut session, &path).await })
            .map_err(classify)
    }

    fn upload(&self, source: &Path, dest: &str) -> Result<(), ProviderError> {
        let mut local = std::fs::File::open(source).map_err(|error| {
            ProviderError::Transport(format!("local artifact {}: {error}", source.display()))
        })?;
        let dest = PathBuf::from(dest);
        let mut session = self.session();
        self.runtime
            .block_on(async {
                let handle = session.open_write(&dest).await?;
                let outcome = upload_stream(&mut session, &handle, &mut local).await;
                close_preserving(&mut session, handle, outcome).await
            })
            .map_err(classify)
    }

    fn write(&self, dest: &str, bytes: &[u8]) -> Result<(), ProviderError> {
        let dest = PathBuf::from(dest);
        let mut cursor = std::io::Cursor::new(bytes);
        let mut session = self.session();
        self.runtime
            .block_on(async {
                let handle = session.open_write(&dest).await?;
                let outcome = upload_stream(&mut session, &handle, &mut cursor).await;
                close_preserving(&mut session, handle, outcome).await
            })
            .map_err(classify)
    }

    fn fetch(&self, path: &str, max: u64) -> Result<Vec<u8>, ProviderError> {
        let path = PathBuf::from(path);
        let mut session = self.session();
        self.runtime
            .block_on(async { session.read_file(&path, max).await })
            .map_err(classify)
    }

    fn set_mode(&self, path: &str, mode: u32) -> Result<(), ProviderError> {
        let path = PathBuf::from(path);
        let mut session = self.session();
        self.runtime
            .block_on(async { session.set_mode(&path, mode).await })
            .map_err(classify)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), ProviderError> {
        let from = PathBuf::from(from);
        let to = PathBuf::from(to);
        let mut session = self.session();
        self.runtime
            .block_on(async { session.rename(&from, &to).await })
            .map_err(classify)
    }

    fn remove(&self, path: &str) -> Result<(), ProviderError> {
        let path = PathBuf::from(path);
        let mut session = self.session();
        self.runtime
            .block_on(async { session.remove(&path).await })
            .map_err(classify)
    }

    fn list(&self, dir: &str) -> Result<Vec<String>, ProviderError> {
        let dir = PathBuf::from(dir);
        let mut session = self.session();
        self.runtime
            .block_on(async { session.list_names(&dir).await })
            .map_err(classify)
    }

    fn handshake(&self, object: &str) -> Result<HandshakeReport, ProviderError> {
        let client = EndpointInfo {
            name: "strop".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            build: None,
            target: crate::bootstrap::local_target()
                .unwrap_or("unknown")
                .to_string(),
        };
        let facts =
            worker_transport::handshake(&self.endpoint, object, &client).map_err(|error| {
                let text = error.to_string();
                // Launch refusal (noexec mount, restricted account) is
                // classified from the diagnostic; it is never evaded.
                let lowered = text.to_ascii_lowercase();
                if lowered.contains("permission denied")
                    || lowered.contains("operation not permitted")
                {
                    ProviderError::NoExec(text)
                } else {
                    ProviderError::Handshake(text)
                }
            })?;
        Ok(HandshakeReport {
            protocol: facts.protocol,
            worker: facts.worker,
            session: facts.session,
        })
    }
}

/// One std-pipe conversion onto the runtime's IO driver.
fn convert(child: &mut OwnedProcess) -> Result<(ChildStdin, ChildStdout), ProviderError> {
    let stdin = child
        .take_stdin()
        .ok_or_else(|| ProviderError::Transport("ssh stdin pipe missing".into()))?;
    let stdin = ChildStdin::from_std(stdin)
        .map_err(|error| ProviderError::Transport(format!("ssh stdin pipe: {error}")))?;
    let stdout = child
        .take_stdout()
        .ok_or_else(|| ProviderError::Transport("ssh stdout pipe missing".into()))?;
    let stdout = ChildStdout::from_std(stdout)
        .map_err(|error| ProviderError::Transport(format!("ssh stdout pipe: {error}")))?;
    Ok((stdin, stdout))
}

/// Bounded stderr capture on a plain thread: the pipe is always
/// drained, the prefix retained for classified diagnostics.
fn spawn_stderr_drain(child: &mut OwnedProcess) -> Result<Arc<Mutex<Vec<u8>>>, ProviderError> {
    use std::io::Read;
    let mut source = child
        .take_stderr()
        .ok_or_else(|| ProviderError::Transport("ssh stderr pipe missing".into()))?;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    std::thread::spawn(move || {
        let mut chunk = [0_u8; 4096];
        loop {
            match source.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(count) => {
                    if let Ok(mut captured) = sink.lock() {
                        if captured.len() < STDERR_LIMIT {
                            let room = STDERR_LIMIT - captured.len();
                            captured.extend_from_slice(&chunk[..count.min(room)]);
                        }
                    }
                }
            }
        }
    });
    Ok(captured)
}

fn stderr_text(captured: &Arc<Mutex<Vec<u8>>>) -> Option<String> {
    let captured = captured.lock().ok()?;
    let text = String::from_utf8_lossy(&captured).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Map one SFTP exchange failure onto the deploy vocabulary. Status
/// codes carry the authority; message text only refines between the
/// explicit read-only/disk-full outcomes the state machine must name.
fn classify(fault: Fault) -> ProviderError {
    let (kind, _poisoned) = fault.disposition();
    let detail = fault.detail().to_owned();
    let lowered = detail.to_ascii_lowercase();
    if lowered.contains("no space left") {
        return ProviderError::DiskFull(detail);
    }
    if lowered.contains("read-only file system") || lowered.contains("readonly") {
        return ProviderError::ReadOnly(detail);
    }
    match kind {
        ReadFailureKind::NotFound => ProviderError::NotFound(detail),
        ReadFailureKind::Permission => ProviderError::Permission(detail),
        _ => ProviderError::Transport(detail),
    }
}

/// Decode one lstat reply into the deploy metadata surface. The owner
/// is the numeric uid string — exactly the discovered principal's
/// domain, so ownership validation compares like with like.
fn remote_stat(attrs: crate::transport::wire::DeployAttrs) -> Result<RemoteStat, ProviderError> {
    let mode = attrs
        .permissions
        .ok_or_else(|| ProviderError::Transport("server reported no permission bits".into()))?;
    let kind = match mode & 0o170000 {
        0o100000 => RemoteKind::File,
        0o040000 => RemoteKind::Dir,
        0o120000 => RemoteKind::Symlink,
        _ => RemoteKind::Other,
    };
    Ok(RemoteStat {
        kind,
        mode: mode & 0o7777,
        owner: attrs
            .uid
            .map(|uid| uid.to_string())
            .ok_or_else(|| ProviderError::Transport("server reported no owner".into()))?,
        len: attrs.size.unwrap_or(0),
    })
}

/// `mkdir -p` with exact 0700 bits: parents are created on the
/// not-found path only, each positively identified by its own path.
fn mkdir_recursive<'a>(
    session: &'a mut DeploySftp<ChildStdin, ChildStdout>,
    path: &'a Path,
) -> std::pin::Pin<Box<dyn Future<Output = Result<(), Fault>> + 'a>> {
    Box::pin(async move {
        match session.mkdir(path, 0o700).await {
            Ok(()) => Ok(()),
            Err(fault) if fault.disposition().0 == ReadFailureKind::NotFound => {
                let Some(parent) = path.parent() else {
                    return Err(fault);
                };
                mkdir_recursive(session, parent).await?;
                session.mkdir(path, 0o700).await
            }
            Err(fault) => Err(fault),
        }
    })
}
/// Stream the bounded upload in positioned write chunks.
async fn upload_stream(
    session: &mut DeploySftp<ChildStdin, ChildStdout>,
    handle: &FileHandle,
    source: &mut impl std::io::Read,
) -> Result<(), Fault> {
    let mut offset = 0_u64;
    let mut chunk = vec![0_u8; UPLOAD_CHUNK];
    loop {
        let count = std::io::Read::read(source, &mut chunk).map_err(|error| {
            Fault::new(
                ReadStage::Transfer,
                ReadFailureKind::Io,
                format!("local artifact read: {error}"),
            )
        })?;
        if count == 0 {
            return Ok(());
        }
        session.write_at(handle, offset, &chunk[..count]).await?;
        offset += count as u64;
    }
}

/// Close always: a failed close composes with the primary fault and
/// poisons the connection, exactly the read path's rule.
async fn close_preserving(
    session: &mut DeploySftp<ChildStdin, ChildStdout>,
    handle: FileHandle,
    outcome: Result<(), Fault>,
) -> Result<(), Fault> {
    match outcome {
        Ok(()) => session.close(handle).await,
        Err(primary) => match session.close(handle).await {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(primary.with_cleanup(cleanup)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::wire::DeployAttrs;

    fn fault(kind: ReadFailureKind, detail: &str) -> Fault {
        Fault::new(ReadStage::Transfer, kind, detail)
    }

    #[test]
    fn provider_classification_keeps_status_codes_authoritative() {
        // Codes classify; server text only refines read-only/disk-full.
        assert!(matches!(
            classify(fault(
                ReadFailureKind::Protocol,
                "SFTP status 4: No space left on device"
            )),
            ProviderError::DiskFull(_)
        ));
        assert!(matches!(
            classify(fault(
                ReadFailureKind::Permission,
                "SFTP status 3: Read-only file system"
            )),
            ProviderError::ReadOnly(_)
        ));
        assert!(matches!(
            classify(fault(
                ReadFailureKind::NotFound,
                "SFTP status 2: no such file"
            )),
            ProviderError::NotFound(_)
        ));
        assert!(matches!(
            classify(fault(
                ReadFailureKind::Permission,
                "SFTP status 3: Permission denied"
            )),
            ProviderError::Permission(_)
        ));
        assert!(matches!(
            classify(fault(ReadFailureKind::Protocol, "SFTP status 4: failure")),
            ProviderError::Transport(_)
        ));
        // A disk-full-sounding message never upgrades a clean code's
        // authority: classification is for reporting, not decisions.
        assert!(matches!(
            classify(fault(ReadFailureKind::Io, "pipe closed")),
            ProviderError::Transport(_)
        ));
    }

    #[test]
    fn lstat_decoding_maps_kinds_mode_owner_and_length() {
        let stat = remote_stat(DeployAttrs {
            size: Some(7),
            permissions: Some(0o100500),
            uid: Some(1000),
        })
        .expect("file decodes");
        assert_eq!(stat.kind, RemoteKind::File);
        assert_eq!(stat.mode, 0o500);
        assert!(stat.owner_executable());
        assert_eq!(stat.owner, "1000");
        assert_eq!(stat.len, 7);

        for (mode, kind) in [
            (0o040700_u32, RemoteKind::Dir),
            (0o120777_u32, RemoteKind::Symlink),
            (0o060660_u32, RemoteKind::Other),
        ] {
            let stat = remote_stat(DeployAttrs {
                size: None,
                permissions: Some(mode),
                uid: Some(0),
            })
            .expect("kind decodes");
            assert_eq!(stat.kind, kind);
        }
        // Missing owner or permission evidence is a transport failure,
        // never a guessed stat.
        assert!(remote_stat(DeployAttrs {
            size: None,
            permissions: Some(0o100644),
            uid: None,
        })
        .is_err());
    }
}
