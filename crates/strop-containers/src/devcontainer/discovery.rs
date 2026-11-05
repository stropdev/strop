use super::config::RawConfiguration;
use super::jsonc::DEFAULT_INPUT_LIMIT;
use super::DevcontainerError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_NAMED_DIRECTORIES: usize = 256;
const MAX_DIRECTORY_ENTRIES: usize = 4096;

/// SHA-256 of the exact JSONC bytes. Comments and formatting therefore also
/// invalidate suppression/trust decisions, which is conservative by design.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentDigest([u8; 32]);
impl ContentDigest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    pub fn to_hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
impl fmt::Debug for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ContentDigest")
            .field(&self.to_hex())
            .finish()
    }
}
impl fmt::Display for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// Stable workspace/config/content identity. A path rename or content edit is
/// a different identity; an unrelated nested project is never folded into it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConfigIdentity {
    workspace_root: PathBuf,
    config_path: PathBuf,
    digest: ContentDigest,
}
impl ConfigIdentity {
    pub(crate) fn new(workspace_root: PathBuf, config_path: PathBuf, bytes: &[u8]) -> Self {
        Self {
            workspace_root,
            config_path,
            digest: ContentDigest::of(bytes),
        }
    }
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
    pub fn digest(&self) -> ContentDigest {
        self.digest
    }
    /// Stable ownership scope across reviewed edits. The digest still changes,
    /// so trust/suppression state must not be carried over implicitly.
    pub fn same_location(&self, other: &Self) -> bool {
        self.workspace_root == other.workspace_root && self.config_path == other.config_path
    }
    pub fn labels(&self) -> [String; 2] {
        [
            format!(
                "devcontainer.local_folder={}",
                self.workspace_root.display()
            ),
            format!("devcontainer.config_file={}", self.config_path.display()),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiscoveryPrecedence {
    DevcontainerDirectory,
    WorkspaceRoot,
    NamedSubfolder,
}

#[derive(Debug, Clone)]
pub struct DiscoveredConfiguration {
    pub identity: ConfigIdentity,
    pub configuration: RawConfiguration,
    pub precedence: DiscoveryPrecedence,
}

#[derive(Debug, Clone)]
pub struct DiscoveryIssue {
    pub path: PathBuf,
    pub error: DevcontainerError,
}

/// All valid configurations in precedence order. The first is the default
/// offer, not an assertion that other legitimate named configurations vanish.
#[derive(Debug, Default)]
pub struct DiscoveryReport {
    pub configurations: Vec<DiscoveredConfiguration>,
    pub issues: Vec<DiscoveryIssue>,
}
impl DiscoveryReport {
    pub fn preferred(&self) -> Option<&DiscoveredConfiguration> {
        self.configurations.first()
    }
    pub fn is_empty(&self) -> bool {
        self.configurations.is_empty()
    }
}

/// Inspect exactly the three standard layouts. No reference CLI, Docker,
/// lifecycle command, environment probe or network access is reachable here.
pub fn discover(workspace_root: &Path) -> Result<DiscoveryReport, DevcontainerError> {
    discover_with_limit(workspace_root, DEFAULT_INPUT_LIMIT)
}

pub fn discover_with_limit(
    workspace_root: &Path,
    input_limit: usize,
) -> Result<DiscoveryReport, DevcontainerError> {
    let root = fs::canonicalize(workspace_root).map_err(|error| DevcontainerError::Io {
        operation: "resolve workspace root".into(),
        path: workspace_root.to_path_buf(),
        detail: error.to_string(),
    })?;
    let mut report = DiscoveryReport::default();
    let devcontainer = root.join(".devcontainer");
    load_candidate(
        &root,
        devcontainer.join("devcontainer.json"),
        DiscoveryPrecedence::DevcontainerDirectory,
        input_limit,
        &mut report,
    );
    load_candidate(
        &root,
        root.join(".devcontainer.json"),
        DiscoveryPrecedence::WorkspaceRoot,
        input_limit,
        &mut report,
    );

    let entries = match fs::read_dir(&devcontainer) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => {
            report.issues.push(DiscoveryIssue {
                path: devcontainer,
                error: DevcontainerError::Io {
                    operation: "list named configurations".into(),
                    path: root.join(".devcontainer"),
                    detail: error.to_string(),
                },
            });
            return Ok(report);
        }
    };
    let mut directories = Vec::new();
    let mut entry_count = 0;
    for entry in entries {
        entry_count += 1;
        if entry_count > MAX_DIRECTORY_ENTRIES {
            report.issues.push(DiscoveryIssue {
                path: root.join(".devcontainer"),
                error: DevcontainerError::UnsupportedConfiguration {
                    detail: format!("more than {MAX_DIRECTORY_ENTRIES} entries in .devcontainer"),
                },
            });
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.issues.push(DiscoveryIssue {
                    path: root.join(".devcontainer"),
                    error: DevcontainerError::Io {
                        operation: "read named configuration entry".into(),
                        path: root.join(".devcontainer"),
                        detail: error.to_string(),
                    },
                });
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                report.issues.push(DiscoveryIssue {
                    path: entry.path(),
                    error: DevcontainerError::Io {
                        operation: "inspect named configuration entry".into(),
                        path: entry.path(),
                        detail: error.to_string(),
                    },
                });
                continue;
            }
        };
        // Symlinked directories and every deeper nesting level are outside the
        // discovery contract. A direct config file itself may be a symlink.
        if file_type.is_dir() {
            if directories.len() == MAX_NAMED_DIRECTORIES {
                report.issues.push(DiscoveryIssue {
                    path: root.join(".devcontainer"),
                    error: DevcontainerError::UnsupportedConfiguration {
                        detail: format!(
                            "more than {MAX_NAMED_DIRECTORIES} named configuration directories"
                        ),
                    },
                });
                break;
            }
            directories.push(entry.path());
        }
    }
    directories.sort();
    for directory in directories {
        load_candidate(
            &root,
            directory.join("devcontainer.json"),
            DiscoveryPrecedence::NamedSubfolder,
            input_limit,
            &mut report,
        );
    }
    Ok(report)
}

fn load_candidate(
    root: &Path,
    path: PathBuf,
    precedence: DiscoveryPrecedence,
    input_limit: usize,
    report: &mut DiscoveryReport,
) {
    let bytes = match read_bounded(&path, input_limit) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return,
        Err(error) => {
            report.issues.push(DiscoveryIssue { path, error });
            return;
        }
    };
    let identity = ConfigIdentity::new(root.to_path_buf(), path.clone(), &bytes);
    match RawConfiguration::parse_jsonc(&bytes, &path, input_limit) {
        Ok(configuration) => report.configurations.push(DiscoveredConfiguration {
            identity,
            configuration,
            precedence,
        }),
        Err(error) => report.issues.push(DiscoveryIssue { path, error }),
    }
}

fn read_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, DevcontainerError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::IsADirectory => return Ok(None),
        Err(error) => {
            return Err(DevcontainerError::Io {
                operation: "open configuration".into(),
                path: path.to_path_buf(),
                detail: error.to_string(),
            })
        }
    };
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| DevcontainerError::Io {
            operation: "read configuration".into(),
            path: path.to_path_buf(),
            detail: error.to_string(),
        })?;
    if bytes.len() > limit {
        return Err(DevcontainerError::InputTooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    Ok(Some(bytes))
}
