//! Pure filesystem intent, observation and outcome contracts. No native effects.
use crate::{Observation, ResourceLocation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OperationKind {
    CreateFile,
    CreateDirectory,
    Rename,
    Copy,
    Trash,
    Remove,
    Restore,
}
impl OperationKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CreateFile => "create file",
            Self::CreateDirectory => "create directory",
            Self::Rename => "rename/move",
            Self::Copy => "copy",
            Self::Trash => "Trash",
            Self::Remove => "delete permanently",
            Self::Restore => "restore",
        }
    }
    pub const fn destructive(self) -> bool {
        matches!(self, Self::Trash | Self::Remove)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CopyVersion {
    Stored,
    Buffer,
}

/// Names are already resolved to one namespace. No executor parses display/Ex text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperationIntent {
    pub kind: OperationKind,
    pub source: Option<ResourceLocation>,
    pub destination: Option<ResourceLocation>,
    pub copy_version: CopyVersion,
    /// A receipt-derived condition, never an observation invented by the executor.
    pub expected_content: Option<[u8; 32]>,
}
impl OperationIntent {
    pub fn location(&self) -> Option<&ResourceLocation> {
        self.source.as_ref().or(self.destination.as_ref())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FsFailureKind {
    Unsupported,
    Permission,
    Conflict,
    Busy,
    InvalidPath,
    Incomplete,
    Io,
    Protocol,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
#[error("{kind:?}: {detail}")]
pub struct FsFailure {
    pub kind: FsFailureKind,
    pub detail: String,
}
impl FsFailure {
    pub fn new(kind: FsFailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocatedObservation {
    pub location: ResourceLocation,
    pub value: Option<Observation>,
}

/// Exact source/destination and parent observations, including absent names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PreparedOperation {
    pub intent: OperationIntent,
    pub source: Option<LocatedObservation>,
    pub destination: Option<LocatedObservation>,
    pub parents: Vec<LocatedObservation>,
    /// A parent created by an earlier approved step is checked against its receipt.
    pub dependencies: Vec<usize>,
    pub capability: OperationCapability,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperationCapability {
    pub principal: Option<u32>,
    /// Native boot/process namespace observation; never a display hostname alone.
    pub incarnation: String,
    pub no_replace: bool,
    pub trash: bool,
    pub trash_root: Option<ResourceLocation>,
    pub metadata_policy: String,
    pub concurrency_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperationRefusal {
    pub intent: OperationIntent,
    pub failure: FsFailure,
}

/// Object identity obtained from an owned descriptor before publication; the
/// optional digest identifies intended bytes, not whatever later occupies a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PublicationWitness {
    pub identity: crate::ObjectId,
    pub changed: Option<crate::FileTime>,
    pub content: Option<[u8; 32]>,
}
impl PublicationWitness {
    /// Missing after-version data is not publication ownership.
    pub fn matches_metadata(&self, observed: &Observation) -> bool {
        self.changed.is_some()
            && observed.identity == Some(self.identity)
            && observed.changed == self.changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StepOutcome {
    Committed {
        source_after: Option<Observation>,
        destination_after: Option<Observation>,
        /// A platform-native Trash location, when the backend provides one.
        recovery: Option<ResourceLocation>,
        warnings: Vec<String>,
        publication: Option<PublicationWitness>,
    },
    Refused(FsFailure),
    Cancelled {
        detail: String,
    },
    Unconfirmed {
        detail: String,
        /// Evidence observed after publication, if the acknowledgment path failed.
        observed_destination: Option<Observation>,
        recovery: Option<ResourceLocation>,
        publication: Option<PublicationWitness>,
    },
}
impl StepOutcome {
    pub fn is_committed(&self) -> bool {
        matches!(self, Self::Committed { .. })
    }
    pub fn is_unconfirmed(&self) -> bool {
        matches!(self, Self::Unconfirmed { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StepReceipt {
    pub step: usize,
    pub operation: PreparedOperation,
    pub outcome: StepOutcome,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VerifiedChange {
    pub source_after: Option<Observation>,
    pub destination_after: Option<Observation>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VerifiedOutcome {
    Unchanged,
    Committed(Box<VerifiedChange>),
    Unknown { detail: String },
}
