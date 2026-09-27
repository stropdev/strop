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
    /// Protected document save (0058 WK09): frozen content bytes replace
    /// (or create) the destination atomically under a baseline-mtime
    /// conflict contract, preserving the destination's metadata.
    Store,
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
            Self::Store => "save",
        }
    }
    pub const fn destructive(self) -> bool {
        matches!(self, Self::Trash | Self::Remove)
    }
}

/// The conditional policy of a [`OperationKind::Store`] intent
/// (0058 WK09): the baseline is the editor's last observed destination
/// mtime (`None` = the editor knows the name was absent), never an
/// executor-invented observation. Conflict semantics match the local
/// `:w` behavior exactly: without `force` a moved baseline (or an
/// occupied new name) refuses; `force` (`:w!`) overwrites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StorePolicy {
    /// Baseline destination mtime; `None` when the editor knows the
    /// destination was absent at load/last save.
    pub baseline: Option<crate::FileTime>,
    /// Protected save's captured object identity. A replaced inode
    /// cannot borrow the old mtime/content and gain write authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_object: Option<crate::ObjectId>,
    /// Digest of the complete bounded xattr set at edit admission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_attributes: Option<[u8; 32]>,
    /// `:w!`: overwrite despite a moved baseline or an occupied new name.
    pub force: bool,
    /// Save-as (`:w {path}`): the destination is a new name, so an
    /// occupied destination is a conflict unless `force` is set.
    pub expect_absent: bool,
    /// Exact prior content digest. `:remote edit` checks the displayed
    /// snapshot at admission; protected saves check the retained
    /// baseline again before publication, including same-mtime edits.
    /// Ordinary local saves may omit this and use mtime policy only.
    pub displayed: Option<[u8; 32]>,
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
    /// A receipt-derived condition, never an observation invented by the
    /// executor. For `Store` it is REQUIRED at apply time: the digest of
    /// the frozen content bytes, so a lost receipt can still be verified.
    pub expected_content: Option<[u8; 32]>,
    /// The conditional policy of a `Store` intent; absent for every other
    /// kind. Serde-skipped when absent so the wire shape of non-Store
    /// intents is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<StorePolicy>,
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
