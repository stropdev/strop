//! One same-release worker artifact catalog for SSH and container
//! deployment. Called on jobs; no binary hashing crosses input→render.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use strop_worker_deploy::{DeployRefusal, ReleaseCatalog, MAX_WORKER_BYTES};
use strop_workspace::operation::{FsFailure, FsFailureKind};

fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}

pub(crate) fn map_refusal(refusal: DeployRefusal) -> FsFailure {
    let kind = match &refusal {
        DeployRefusal::ConsentRequired { .. } => FsFailureKind::Permission,
        DeployRefusal::OfflineNoArtifact { .. }
        | DeployRefusal::ArtifactNotStaged { .. }
        | DeployRefusal::PreinstalledInvalid { .. } => FsFailureKind::Unsupported,
        DeployRefusal::IdentityMismatch { .. } => FsFailureKind::Protocol,
        _ => FsFailureKind::Io,
    };
    failure(kind, refusal.to_string())
}

/// The installed binary can supply only its own compile target. A
/// foreign endpoint needs an explicit administrator-provisioned worker
/// artifact; the handshake still verifies its build and target after
/// deployment. No guessed cross-target upload.
pub(crate) fn select_binary(target: &str) -> Result<PathBuf, FsFailure> {
    if let Some(path) = std::env::var_os("STROP_WORKER_BINARY") {
        return Ok(PathBuf::from(path));
    }
    if target != strop_worker_protocol::TARGET_TRIPLE {
        return Err(failure(
            FsFailureKind::Unsupported,
            format!("this editor is built for {}, but {target} needs a matching worker artifact; set STROP_WORKER_BINARY to its verified release binary", strop_worker_protocol::TARGET_TRIPLE),
        ));
    }
    std::env::current_exe().map_err(|error| {
        failure(
            FsFailureKind::Io,
            format!("this install's own worker binary is unavailable: {error}"),
        )
    })
}

/// Bind exact local bytes to one release/target, before either provider
/// transfers them. The provider re-verifies the object at its final path;
/// a mutated source after this hash fails rather than being trusted.
pub(crate) fn catalog_for(target: &str, binary: &Path) -> Result<ReleaseCatalog, FsFailure> {
    let unreadable = |error: std::io::Error| {
        failure(
            FsFailureKind::Io,
            format!("cannot read this install's worker artifact: {error}"),
        )
    };
    let mut file = std::fs::File::open(binary).map_err(unreadable)?;
    let length = file.metadata().map_err(unreadable)?.len();
    if length > MAX_WORKER_BYTES {
        return Err(failure(
            FsFailureKind::Unsupported,
            format!("worker artifact exceeds the {MAX_WORKER_BYTES}-byte supply bound"),
        ));
    }
    let mut digest = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(unreadable)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > MAX_WORKER_BYTES {
            return Err(failure(
                FsFailureKind::Unsupported,
                "worker artifact grew beyond the supply bound while hashing",
            ));
        }
        digest.update(&buffer[..read]);
    }
    if count != length {
        return Err(failure(
            FsFailureKind::Io,
            "worker artifact changed length while hashing",
        ));
    }
    let version = env!("CARGO_PKG_VERSION");
    let body = serde_json::json!({
        "schema": 1,
        "product": "strop",
        "version": version,
        "tag": format!("v{version}"),
        "published_at": "1970-01-01T00:00:00Z",
        "artifacts": [{
            "target": target,
            "name": format!("strop-{version}-{target}.tar.gz"),
            "sha256": format!("{:x}", digest.finalize()),
            "bytes": count,
            "url": "",
        }],
        "worker": {
            "protocol": strop_worker_protocol::PROTOCOL_VERSION,
            "min_editor": strop_worker_deploy::MIN_EDITOR_VERSION,
            "targets": [target],
        },
    })
    .to_string();
    ReleaseCatalog::parse(body.as_bytes()).map_err(|error| {
        failure(
            FsFailureKind::Protocol,
            format!("this build's worker catalog does not parse: {error}"),
        )
    })
}
