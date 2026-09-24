//! The typed failure surface of the worker client (0058 WK04). Every
//! failure is one of these; there is no silent fallback to a second
//! filesystem implementation, ever.

use strop_worker_protocol::{ProtocolError, Refusal};
use strop_workspace::operation::FsFailure;

/// Why a worker operation failed. Transport/process failures and typed
/// refusals stay distinct from the domain [`FsFailure`] taxonomy.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The worker process could not be spawned at all.
    #[error("worker spawn failed: {0}")]
    Spawn(String),
    /// The handshake did not complete: EOF, garbage, or the timeout
    /// elapsed. A successful spawn alone never means ready.
    #[error("worker handshake failed: {0}")]
    Handshake(String),
    /// The answering worker is not this client (version/build/target
    /// mismatch is a truthful refusal, never a downgrade).
    #[error("worker identity mismatch in {field}: expected {expected}, got {actual}")]
    Mismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
    /// The worker refused admission, typed (capability, staleness,
    /// bounds, retirement).
    #[error("worker refused: {0}")]
    Refused(#[from] Refusal),
    /// The operation ran and failed in the domain taxonomy.
    #[error("{0}")]
    Domain(#[from] FsFailure),
    /// A protocol-level failure reported by the worker or decoded locally.
    #[error("protocol failure: {0}")]
    Protocol(#[from] ProtocolError),
    /// The worker died or the transport broke mid-session. In-flight
    /// requests fail with this; prepared authority from the dead
    /// incarnation is rejected by any fresh worker. Includes the worker's
    /// bounded private diagnostics when available.
    #[error("worker lost: {0}")]
    WorkerLost(String),
    /// The caller's cancellation propagated through the lease.
    #[error("cancelled")]
    Cancelled,
    /// The lease was shut down; no further requests are admitted.
    #[error("worker session is closed")]
    Closed,
}

impl ClientError {
    /// True when the caller's cancellation caused this failure.
    pub fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled)
            || matches!(
                self,
                Self::Domain(failure) if failure.kind == strop_workspace::operation::FsFailureKind::Cancelled
            )
    }
}
