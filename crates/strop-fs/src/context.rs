//! Admitted execution context, passed as data (0058 WK03). The kernel holds
//! no transport adapters and no process-global state: namespace identity is
//! compared as data, the native principal is re-read at each use, and the
//! session incarnation lives here instead of a process-wide lazy static.
//!
//! Prepared authority is bound to the context that minted it: execution and
//! verification compare a receipt's principal/incarnation against the calling
//! context, so a restarted worker or editor session cannot apply or "verify"
//! an old session's prepared operation.

use crate::failure;
use strop_workspace::operation::{FsFailure, FsFailureKind, OperationCapability};
use strop_workspace::{ContainerId, Filesystem, RemoteEndpoint, ResourceLocation};

/// One admitted filesystem namespace, as data — never a client handle.
/// `Native` is the filesystem of the process running the kernel, the only
/// namespace this crate touches directly. Detached variants carry identity
/// only: the kernel answers their operations with a typed refusal and the
/// owning client dispatches through its admitted transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceView {
    /// The native filesystem of the process running the kernel.
    Native,
    /// One remote endpoint's namespace (identity only).
    Remote(RemoteEndpoint),
    /// One running container's namespace (identity only; read-only by policy).
    Container(ContainerId),
}

impl NamespaceView {
    /// The workspace identity this view admits.
    pub fn filesystem(&self) -> Filesystem {
        match self {
            Self::Native => Filesystem::Local,
            Self::Remote(endpoint) => Filesystem::Remote(endpoint.clone()),
            Self::Container(id) => Filesystem::Container(id.clone()),
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Native => "the native namespace".into(),
            other => format!("namespace {}", other.filesystem().label()),
        }
    }
}

/// Admitted context for kernel calls: the namespace this call runs for plus
/// the session incarnation that binds prepared authority to this session.
/// Cheap to clone; clones share the incarnation.
#[derive(Clone)]
pub struct ExecutionContext {
    namespace: NamespaceView,
    incarnation: std::sync::Arc<std::sync::LazyLock<Result<String, FsFailure>>>,
}

impl std::fmt::Debug for ExecutionContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExecutionContext")
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl ExecutionContext {
    /// Capture one session's context. Construction performs no native work;
    /// the incarnation nonce is minted on first use so a randomness failure
    /// surfaces as a typed operation failure, never a construction panic.
    pub fn capture(namespace: NamespaceView) -> Self {
        Self {
            namespace,
            incarnation: std::sync::Arc::new(std::sync::LazyLock::new(nonce)),
        }
    }

    /// The context for this process's own native filesystem.
    pub fn native() -> Self {
        Self::capture(NamespaceView::Native)
    }
    pub fn namespace(&self) -> &NamespaceView {
        &self.namespace
    }

    /// Admit one location: its identity must match the admitted namespace
    /// exactly. A remote path never aliases a local one with the same bytes.
    pub fn admit(&self, location: &ResourceLocation) -> Result<(), FsFailure> {
        if location.filesystem == self.namespace.filesystem() {
            return Ok(());
        }
        Err(failure(
            FsFailureKind::Unsupported,
            format!(
                "{} is outside this context's admitted namespace ({})",
                location.filesystem.label(),
                self.namespace.describe()
            ),
        ))
    }

    /// This session's incarnation, minted once and shared by every clone.
    fn incarnation(&self) -> Result<String, FsFailure> {
        (**self.incarnation).clone()
    }

    /// The capability profile this context grants prepared operations. The
    /// principal is re-read at each call so a privilege change between
    /// preparation and execution is caught by the receipt comparison, exactly
    /// as when it was derived per call.
    pub(crate) fn capability(&self) -> Result<OperationCapability, FsFailure> {
        Ok(OperationCapability {
            principal: principal(),
            incarnation: self.incarnation()?,
            no_replace: true,
            trash: true,
            trash_root: None,
            metadata_policy: "create: 0666/0777 subject to umask; copy: permissions and bounded xattrs, stored-file mtime; owner is current user".into(),
            concurrency_policy: "descriptor-pinned parents, final observations and cooperative name locks; no universal CAS against nonparticipants".into(),
        })
    }
}

/// The calling process's effective principal, when the platform has one.
fn principal() -> Option<u32> {
    #[cfg(unix)]
    {
        Some(rustix::process::geteuid().as_raw())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// 128 bits of fresh randomness as hex: session incarnations and private
/// stage names. Not a secret; it separates one session's records from
/// another's, never authenticates a hostile same-principal process.
pub(crate) fn nonce() -> Result<String, FsFailure> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| failure(FsFailureKind::Io, error.to_string()))?;
    use std::fmt::Write as _;
    let mut text = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}
