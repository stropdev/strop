//! Remote save refusal and uncertainty belong to the worker-backed
//! editor attempt, not the retired Python transport.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    Conflict,
    Busy,
    Unsupported,
    Permission,
    InvalidPath,
    Metadata,
    Protocol,
    Io,
    Cancelled,
}

impl std::fmt::Display for RefusalKind {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(match self {
            Self::Conflict => "remote file changed",
            Self::Busy => "remote file busy",
            Self::Unsupported => "unsupported remote save capability",
            Self::Permission => "remote permission denied",
            Self::InvalidPath => "remote path refused",
            Self::Metadata => "remote metadata cannot be preserved",
            Self::Protocol => "invalid remote save response",
            Self::Io => "remote I/O failed",
            Self::Cancelled => "remote operation cancelled before commit",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum RemoteSaveError {
    #[error("{kind}: {detail}")]
    Refused { kind: RefusalKind, detail: String },
    #[error("remote save outcome unconfirmed: {detail}; use :remote verify")]
    Unconfirmed { detail: String },
}

impl RemoteSaveError {
    pub fn is_unconfirmed(&self) -> bool {
        matches!(self, Self::Unconfirmed { .. })
    }
}
