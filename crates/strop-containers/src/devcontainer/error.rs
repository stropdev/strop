use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;

/// Failures produced by passive discovery, configuration parsing and the
/// explicitly invoked reference CLI. Diagnostics never include configuration
/// values; process diagnostics are redacted before this error is constructed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum DevcontainerError {
    #[error("cannot {operation} {}: {detail}", path.display())]
    Io {
        operation: Cow<'static, str>,
        path: PathBuf,
        detail: String,
    },
    #[error("devcontainer input exceeds the {limit}-byte bound: {}", path.display())]
    InputTooLarge { path: PathBuf, limit: usize },
    #[error("invalid JSONC in {} at line {line}, column {column}: {detail}", path.display())]
    Json {
        path: PathBuf,
        line: usize,
        column: usize,
        detail: String,
    },
    #[error("devcontainer configuration root must be an object")]
    InvalidRoot,
    #[error(
        "reference Dev Container CLI was not found; install it explicitly or configure its path"
    )]
    CliNotFound,
    #[error("unsupported reference Dev Container CLI version {found}; supported versions are {supported}")]
    UnsupportedVersion {
        found: String,
        supported: Cow<'static, str>,
    },
    #[error("malformed reference CLI version response")]
    VersionProtocol,
    #[error("reference CLI {operation} failed with exit code {code:?}: {detail}")]
    CliFailure {
        operation: Cow<'static, str>,
        code: Option<i32>,
        detail: String,
    },
    #[error("reference CLI returned a malformed {operation} response: {detail}")]
    CliProtocol {
        operation: Cow<'static, str>,
        detail: String,
    },
    #[error("reference CLI {operation} exceeded its {} second deadline", deadline.as_secs())]
    Timeout {
        operation: Cow<'static, str>,
        deadline: Duration,
    },
    #[error("reference CLI {operation} output exceeded its {limit}-byte bound")]
    OutputTooLarge {
        operation: Cow<'static, str>,
        limit: usize,
    },
    #[error("devcontainer operation cancelled")]
    Cancelled,
    #[error("configuration cannot be used for this operation: {detail}")]
    UnsupportedConfiguration { detail: String },
    #[error("ownership refused: {detail}")]
    OwnershipRefused { detail: String },
    #[error("reference CLI process failed: {detail}")]
    Process { detail: String },
}
