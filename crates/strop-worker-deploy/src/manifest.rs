//! The release-catalog worker manifest (0058 WK05) as the deployment
//! client consumes it: strict parsing of the generator's wire shape and
//! the honest deploy-vs-fallback decision.
//!
//! The generator (`.github/scripts/release-catalog.py`) pins the same
//! shape from the producing end (`tests/release-catalog.sh`); the tests
//! here pin it from the consuming end. Unknown extra fields are ignored
//! (additive evolution), but every required field must be present with
//! the right type — a catalog that predates or mangles the worker
//! section is a truthful fallback, never a guessed deploy.

use std::fmt;

/// One artifact entry of the catalog, including the WK05 byte size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactManifest {
    pub target: String,
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
    pub url: String,
}

/// The catalog's worker compatibility manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerManifest {
    /// The only worker wire protocol these artifacts speak.
    pub protocol: u32,
    /// Oldest editor version that can drive a worker at all.
    pub min_editor: String,
    /// Target triples whose artifacts embed worker mode (the same static
    /// binary, so this is the artifact target list).
    pub targets: Vec<String>,
}

/// The catalog facts deployment needs: the release identity, the per-
/// target artifacts and the worker manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCatalog {
    pub version: String,
    pub tag: String,
    pub artifacts: Vec<ArtifactManifest>,
    pub worker: WorkerManifest,
}

/// Catalog parse/validation failure. Every variant is a precise refusal
/// reason the editor can surface.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("release catalog JSON: {0}")]
    Json(String),
    #[error("release catalog missing or mistyped field: {0}")]
    Field(&'static str),
}

/// Why a worker cannot serve this endpoint — the editor falls back to the
/// namespace's non-worker capabilities (read-only SFTP browsing stays).
/// Never an error, never a guessed downgrade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    /// The editor predates worker support entirely.
    EditorPredatesWorkers { editor: String, min_editor: String },
    /// Wire protocols differ; there is no negotiated downgrade.
    ProtocolMismatch { editor: u32, worker: u32 },
    /// No artifact embeds a worker for the endpoint's target.
    NoArtifactForTarget { target: String },
    /// Deployment binds the exact client release; this catalog is for a
    /// different one (the editor must resolve its own tag's catalog).
    CatalogVersionMismatch { editor: String, catalog: String },
}

impl fmt::Display for Fallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fallback::EditorPredatesWorkers { editor, min_editor } => write!(
                f,
                "editor {editor} predates worker support (needs >= {min_editor})"
            ),
            Fallback::ProtocolMismatch { editor, worker } => {
                write!(f, "worker protocol {worker} != editor protocol {editor}")
            }
            Fallback::NoArtifactForTarget { target } => {
                write!(f, "no worker artifact for target {target}")
            }
            Fallback::CatalogVersionMismatch { editor, catalog } => write!(
                f,
                "catalog is for release {catalog}, this editor is {editor}"
            ),
        }
    }
}

/// The deploy-vs-fallback verdict. `Compatible` carries the exact
/// artifact for the endpoint's target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility<'a> {
    Compatible(&'a ArtifactManifest),
    Fallback(Fallback),
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let v = version.strip_prefix('v').unwrap_or(version);
    let mut it = v.split('.');
    Some((
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    ))
}

impl ReleaseCatalog {
    /// Strict parse of the generator's wire shape.
    pub fn parse(body: &[u8]) -> Result<ReleaseCatalog, CatalogError> {
        let json: serde_json::Value =
            serde_json::from_slice(body).map_err(|e| CatalogError::Json(e.to_string()))?;
        let field =
            |entry: &serde_json::Value, key: &'static str| -> Result<String, CatalogError> {
                entry
                    .get(key)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or(CatalogError::Field(key))
            };
        let mut artifacts = Vec::new();
        for entry in json["artifacts"]
            .as_array()
            .ok_or(CatalogError::Field("artifacts"))?
        {
            artifacts.push(ArtifactManifest {
                target: field(entry, "target")?,
                name: field(entry, "name")?,
                sha256: field(entry, "sha256")?,
                bytes: entry
                    .get("bytes")
                    .and_then(|value| value.as_u64())
                    .ok_or(CatalogError::Field("bytes"))?,
                url: field(entry, "url")?,
            });
        }
        if artifacts.is_empty() {
            return Err(CatalogError::Field("artifacts"));
        }
        let worker = &json["worker"];
        if worker.is_null() {
            return Err(CatalogError::Field("worker"));
        }
        let mut targets: Vec<String> = Vec::new();
        for entry in worker["targets"]
            .as_array()
            .ok_or(CatalogError::Field("worker.targets"))?
        {
            targets.push(
                entry
                    .as_str()
                    .map(str::to_string)
                    .ok_or(CatalogError::Field("worker.targets"))?,
            );
        }
        let protocol = worker["protocol"]
            .as_u64()
            .ok_or(CatalogError::Field("worker.protocol"))?;
        let protocol =
            u32::try_from(protocol).map_err(|_| CatalogError::Field("worker.protocol"))?;
        Ok(ReleaseCatalog {
            version: field(&json, "version")?,
            tag: field(&json, "tag")?,
            artifacts,
            worker: WorkerManifest {
                protocol,
                min_editor: field(worker, "min_editor")?,
                targets,
            },
        })
    }

    pub fn artifact(&self, target: &str) -> Option<&ArtifactManifest> {
        self.artifacts.iter().find(|entry| entry.target == target)
    }

    /// The WK05 deploy-vs-fallback decision: exact release binding, exact
    /// protocol, target coverage and the minimum editor version, each an
    /// honest named reason.
    pub fn compatibility(
        &self,
        editor_version: &str,
        editor_protocol: u32,
        target: &str,
    ) -> Compatibility<'_> {
        if parse_version(editor_version)
            .zip(parse_version(&self.worker.min_editor))
            .is_some_and(|(editor, min)| editor < min)
        {
            return Compatibility::Fallback(Fallback::EditorPredatesWorkers {
                editor: editor_version.to_string(),
                min_editor: self.worker.min_editor.clone(),
            });
        }
        let protocol_matches = editor_protocol == self.worker.protocol;
        let version_matches = editor_version == self.version;
        let artifact = if protocol_matches
            && version_matches
            && self.worker.targets.iter().any(|t| t == target)
        {
            self.artifact(target)
        } else {
            None
        };
        match strop_core::worker::deploy_policy::catalog_verdict(
            protocol_matches,
            version_matches,
            artifact.is_some(),
        ) {
            strop_core::worker::deploy_policy::CatalogVerdict::Compatible => {}
            strop_core::worker::deploy_policy::CatalogVerdict::ProtocolMismatch => {
                return Compatibility::Fallback(Fallback::ProtocolMismatch {
                    editor: editor_protocol,
                    worker: self.worker.protocol,
                });
            }
            strop_core::worker::deploy_policy::CatalogVerdict::VersionMismatch => {
                return Compatibility::Fallback(Fallback::CatalogVersionMismatch {
                    editor: editor_version.to_string(),
                    catalog: self.version.clone(),
                });
            }
            strop_core::worker::deploy_policy::CatalogVerdict::NoArtifactForTarget => {
                return Compatibility::Fallback(Fallback::NoArtifactForTarget {
                    target: target.to_string(),
                });
            }
        }
        // The verdict's inputs and the returned artifact are the same
        // lookup; no second search or different target can slip through.
        match artifact {
            Some(artifact) => Compatibility::Compatible(artifact),
            None => Compatibility::Fallback(Fallback::NoArtifactForTarget {
                target: target.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Historical v1 catalog: current v2 clients must refuse it,
    /// while parsing and typed compatibility remain explicit.
    const CATALOG_FIXTURE: &str = r#"{
  "schema": 1,
  "product": "strop",
  "version": "0.35.0",
  "tag": "v0.35.0",
  "published_at": "2026-09-24T00:00:00Z",
  "artifacts": [
    {
      "target": "aarch64-apple-darwin",
      "name": "strop-0.35.0-aarch64-apple-darwin.tar.gz",
      "sha256": "aaaa",
      "bytes": 101,
      "url": "https://example.invalid/v0.35.0/strop-0.35.0-aarch64-apple-darwin.tar.gz"
    },
    {
      "target": "x86_64-unknown-linux-musl",
      "name": "strop-0.35.0-x86_64-unknown-linux-musl.tar.gz",
      "sha256": "bbbb",
      "bytes": 202,
      "url": "https://example.invalid/v0.35.0/strop-0.35.0-x86_64-unknown-linux-musl.tar.gz"
    }
  ],
  "worker": {
    "protocol": 1,
    "min_editor": "0.35.0",
    "targets": [
      "aarch64-apple-darwin",
      "x86_64-unknown-linux-musl"
    ]
  }
}"#;

    fn catalog() -> ReleaseCatalog {
        ReleaseCatalog::parse(CATALOG_FIXTURE.as_bytes()).expect("fixture parses")
    }

    #[test]
    fn missing_worker_section_is_a_truthful_refusal() {
        let body = CATALOG_FIXTURE.replace(
            r#"  "worker": {
    "protocol": 1,
    "min_editor": "0.35.0",
    "targets": [
      "aarch64-apple-darwin",
      "x86_64-unknown-linux-musl"
    ]
  }"#,
            r#"  "worker": null"#,
        );
        assert_eq!(
            ReleaseCatalog::parse(body.as_bytes()),
            Err(CatalogError::Field("worker"))
        );
    }

    #[test]
    fn mistyped_worker_facts_are_rejected() {
        let body = CATALOG_FIXTURE.replace(r#""protocol": 1"#, r#""protocol": "1""#);
        assert_eq!(
            ReleaseCatalog::parse(body.as_bytes()),
            Err(CatalogError::Field("worker.protocol"))
        );
        let body = CATALOG_FIXTURE.replace(r#""bytes": 101"#, r#""bytes": "101""#);
        assert_eq!(
            ReleaseCatalog::parse(body.as_bytes()),
            Err(CatalogError::Field("bytes"))
        );
    }

    #[test]
    fn compatibility_binds_exact_release_protocol_and_target() {
        let catalog = catalog();
        assert!(matches!(
            catalog.compatibility("0.35.0", 1, "x86_64-unknown-linux-musl"),
            Compatibility::Compatible(_)
        ));
        assert_eq!(
            catalog.compatibility("0.34.0", 1, "x86_64-unknown-linux-musl"),
            Compatibility::Fallback(Fallback::EditorPredatesWorkers {
                editor: "0.34.0".into(),
                min_editor: "0.35.0".into(),
            })
        );
        assert_eq!(
            catalog.compatibility("0.35.0", 2, "x86_64-unknown-linux-musl"),
            Compatibility::Fallback(Fallback::ProtocolMismatch {
                editor: 2,
                worker: 1
            })
        );
        assert_eq!(
            catalog.compatibility("0.36.0", 1, "x86_64-unknown-linux-musl"),
            Compatibility::Fallback(Fallback::CatalogVersionMismatch {
                editor: "0.36.0".into(),
                catalog: "0.35.0".into(),
            })
        );
        assert_eq!(
            catalog.compatibility("0.35.0", 1, "riscv64-unknown-linux-musl"),
            Compatibility::Fallback(Fallback::NoArtifactForTarget {
                target: "riscv64-unknown-linux-musl".into(),
            })
        );
    }
}
