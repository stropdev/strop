//! Hermetic fake provider: an in-memory remote filesystem with metadata,
//! failure injection at every provider operation and a scripted
//! handshake. Stands in for both thin provider shapes (SFTP upload and
//! container tar) — the trait surface they implement is identical.
// Each integration binary uses its own slice of these fixtures; the
// module is shared, so unused-helper warnings would be noise here.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use strop_worker_deploy::manifest::ReleaseCatalog;
use strop_worker_deploy::provider::{
    DeployProvider, EndpointIdentity, HandshakeReport, ProviderError, RemoteKind, RemoteStat,
};
use strop_worker_protocol::{EndpointInfo, Session};

/// The catalog of the editor's own release, matching the WK05 wire shape.
pub fn catalog(version: &str) -> ReleaseCatalog {
    let body = format!(
        r#"{{
  "schema": 1,
  "product": "strop",
  "version": "{version}",
  "tag": "v{version}",
  "published_at": "2026-09-24T00:00:00Z",
  "artifacts": [
    {{
      "target": "x86_64-unknown-linux-musl",
      "name": "strop-{version}-x86_64-unknown-linux-musl.tar.gz",
      "sha256": "cccc",
      "bytes": 4096,
      "url": "https://example.invalid/v{version}/strop-{version}-x86_64-unknown-linux-musl.tar.gz"
    }}
  ],
  "worker": {{
    "protocol": 1,
    "min_editor": "0.35.0",
    "targets": ["x86_64-unknown-linux-musl"]
  }}
}}"#
    );
    ReleaseCatalog::parse(body.as_bytes()).expect("test catalog parses")
}

/// A local "verified artifact": unique bytes per version in a tempdir.
pub fn local_binary(dir: &tempfile::TempDir, version: &str) -> std::path::PathBuf {
    let path = dir.path().join("strop");
    std::fs::write(
        &path,
        format!("fake strop worker binary {version} — verified local artifact").as_bytes(),
    )
    .expect("write local binary");
    path
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: RemoteKind,
    pub mode: u32,
    pub owner: String,
    pub bytes: Vec<u8>,
}

impl Entry {
    pub fn dir(mode: u32, owner: &str) -> Entry {
        Entry {
            kind: RemoteKind::Dir,
            mode,
            owner: owner.to_string(),
            bytes: Vec::new(),
        }
    }

    pub fn file(mode: u32, owner: &str, bytes: &[u8]) -> Entry {
        Entry {
            kind: RemoteKind::File,
            mode,
            owner: owner.to_string(),
            bytes: bytes.to_vec(),
        }
    }

    pub fn symlink(owner: &str) -> Entry {
        Entry {
            kind: RemoteKind::Symlink,
            mode: 0o777,
            owner: owner.to_string(),
            bytes: Vec::new(),
        }
    }

    fn stat(&self) -> RemoteStat {
        RemoteStat {
            kind: self.kind,
            mode: self.mode,
            owner: self.owner.clone(),
            len: self.bytes.len() as u64,
        }
    }
}

/// Failure injection: one optional error per operation, plus a corrupting
/// upload that flips a byte in flight.
#[derive(Debug, Default)]
pub struct Failures {
    pub mkdir: Option<ProviderError>,
    pub upload: Option<ProviderError>,
    pub corrupt_upload: bool,
    pub write: Option<ProviderError>,
    pub fetch: Option<ProviderError>,
    pub set_mode: Option<ProviderError>,
    pub rename: Option<ProviderError>,
    pub remove: Option<ProviderError>,
    pub handshake: Option<ProviderError>,
}

#[derive(Debug, Default)]
pub struct Counts {
    pub upload: usize,
}

pub struct FakeProvider {
    pub endpoint: EndpointIdentity,
    pub cache_base: String,
    pub entries: RefCell<BTreeMap<String, Entry>>,
    pub failures: RefCell<Failures>,
    pub counts: RefCell<Counts>,
    pub handshake_report: HandshakeReport,
}

fn parent(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => "/",
    }
}

impl FakeProvider {
    /// An endpoint whose principal already has a standard cache base.
    pub fn new(handshake_report: HandshakeReport) -> FakeProvider {
        let mut entries = BTreeMap::new();
        entries.insert("/home".to_string(), Entry::dir(0o755, "root"));
        entries.insert("/home/alice".to_string(), Entry::dir(0o755, "alice"));
        entries.insert("/home/alice/.cache".to_string(), Entry::dir(0o755, "alice"));
        FakeProvider {
            endpoint: EndpointIdentity {
                context: "dev-box".to_string(),
                principal: "alice".to_string(),
                target: "x86_64-unknown-linux-musl".to_string(),
            },
            cache_base: "/home/alice/.cache".to_string(),
            entries: RefCell::new(entries),
            failures: RefCell::new(Failures::default()),
            counts: RefCell::new(Counts::default()),
            handshake_report,
        }
    }

    pub fn handshake_ok(version: &str) -> HandshakeReport {
        HandshakeReport {
            protocol: strop_worker_protocol::PROTOCOL_VERSION,
            worker: EndpointInfo {
                name: "strop-worker".to_string(),
                version: version.to_string(),
                build: None,
                target: "x86_64-unknown-linux-musl".to_string(),
            },
            session: Session {
                incarnation: 1,
                lease: strop_worker_protocol::LeaseId(7),
            },
        }
    }

    pub fn fail(&self, failure: &Option<ProviderError>) -> Result<(), ProviderError> {
        match failure {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    pub fn entry(&self, path: &str) -> Option<Entry> {
        self.entries.borrow().get(path).cloned()
    }

    pub fn names(&self, dir: &str) -> Vec<String> {
        self.list(dir).expect("fake list")
    }
}

impl DeployProvider for FakeProvider {
    fn endpoint(&self) -> &EndpointIdentity {
        &self.endpoint
    }

    fn cache_base(&self) -> Result<String, ProviderError> {
        Ok(self.cache_base.clone())
    }

    fn lstat(&self, path: &str) -> Result<Option<RemoteStat>, ProviderError> {
        Ok(self.entries.borrow().get(path).map(Entry::stat))
    }

    fn mkdir_private(&self, path: &str) -> Result<(), ProviderError> {
        self.fail(&self.failures.borrow().mkdir)?;
        let mut current = String::new();
        for part in path.split('/').filter(|part| !part.is_empty()) {
            current.push('/');
            current.push_str(part);
            self.entries
                .borrow_mut()
                .entry(current.clone())
                .or_insert_with(|| Entry::dir(0o700, &self.endpoint.principal));
        }
        Ok(())
    }

    fn upload(&self, source: &Path, dest: &str) -> Result<(), ProviderError> {
        self.fail(&self.failures.borrow().upload)?;
        self.counts.borrow_mut().upload += 1;
        let mut bytes =
            std::fs::read(source).map_err(|e| ProviderError::Transport(e.to_string()))?;
        if self.failures.borrow().corrupt_upload {
            let last = bytes.len() - 1;
            bytes[last] ^= 0xff;
        }
        self.entries.borrow_mut().insert(
            dest.to_string(),
            Entry::file(0o600, &self.endpoint.principal, &bytes),
        );
        Ok(())
    }

    fn write(&self, dest: &str, bytes: &[u8]) -> Result<(), ProviderError> {
        self.fail(&self.failures.borrow().write)?;
        self.entries.borrow_mut().insert(
            dest.to_string(),
            Entry::file(0o600, &self.endpoint.principal, bytes),
        );
        Ok(())
    }

    fn fetch(&self, path: &str, max: u64) -> Result<Vec<u8>, ProviderError> {
        self.fail(&self.failures.borrow().fetch)?;
        let entries = self.entries.borrow();
        let entry = entries
            .get(path)
            .ok_or_else(|| ProviderError::NotFound(path.to_string()))?;
        if entry.bytes.len() as u64 > max {
            return Err(ProviderError::Transport(format!(
                "{path} over the fetch bound"
            )));
        }
        Ok(entry.bytes.clone())
    }

    fn set_mode(&self, path: &str, mode: u32) -> Result<(), ProviderError> {
        self.fail(&self.failures.borrow().set_mode)?;
        let mut entries = self.entries.borrow_mut();
        let entry = entries
            .get_mut(path)
            .ok_or_else(|| ProviderError::NotFound(path.to_string()))?;
        entry.mode = mode;
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), ProviderError> {
        self.fail(&self.failures.borrow().rename)?;
        let mut entries = self.entries.borrow_mut();
        let entry = entries
            .remove(from)
            .ok_or_else(|| ProviderError::NotFound(from.to_string()))?;
        entries.insert(to.to_string(), entry);
        Ok(())
    }

    fn remove(&self, path: &str) -> Result<(), ProviderError> {
        self.fail(&self.failures.borrow().remove)?;
        self.entries
            .borrow_mut()
            .remove(path)
            .ok_or_else(|| ProviderError::NotFound(path.to_string()))?;
        Ok(())
    }

    fn list(&self, dir: &str) -> Result<Vec<String>, ProviderError> {
        Ok(self
            .entries
            .borrow()
            .keys()
            .filter(|path| parent(path) == dir && path.as_str() != dir)
            .map(|path| path.rsplit('/').next().expect("split").to_string())
            .collect())
    }

    fn handshake(&self, _object: &str) -> Result<HandshakeReport, ProviderError> {
        self.fail(&self.failures.borrow().handshake)?;
        Ok(self.handshake_report.clone())
    }
}
