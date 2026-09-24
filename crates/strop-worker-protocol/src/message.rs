//! Handshake, capability vocabulary, bounds, typed refusals and the
//! protocol-error surface (0058 WK02; the normative text is
//! `docs/worker-protocol.md`).
//!
//! Every type here is pure data. The handshake binds protocol version,
//! release/build/target, the fresh session authority, the namespace
//! identity, the admitted capabilities and the hard limits; advertised
//! capability means the relevant native primitive/profile was actually
//! admitted, never that the binary merely compiled for the platform.

use serde::{Deserialize, Serialize};

/// The only wire version this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// Who a peer is, for the handshake record: name, version, the exact
/// build identity and the target triple. Deployment binds the worker to
/// the client's exact release/build/target (WK05); a mismatch is a
/// truthful refusal, not a negotiated downgrade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointInfo {
    pub name: String,
    pub version: String,
    pub build: Option<String>,
    pub target: String,
}

/// The worker's hard bounds, restated on the wire so the client never
/// guesses them (0056 AR06 conventions; 0058 WK11 registers the budgets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub max_frame_bytes: usize,
    pub max_chunk_bytes: usize,
    pub max_pending_requests: usize,
    /// strop-fs's batch step ceiling, restated.
    pub max_batch_steps: usize,
    /// One listing's accumulated entry ceiling, restated.
    pub max_listing_entries: usize,
    pub max_subscriptions: usize,
    pub max_streams: usize,
    pub max_exec_processes: usize,
}

/// How a namespace can be watched, reported honestly (2026-09-16
/// amendment: a periodic full-tree crawl is not an invisible default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyCoverage {
    /// Native watching (inotify-class) in the selected namespace.
    Native,
    /// Bounded polling of the subscribed scopes.
    Polling,
    /// No push coverage; freshness comes from explicit observation.
    OnDemand,
    /// The capability is absent; subscribing is refused, typed.
    Unsupported,
}

/// The capability vocabulary. Advertisement means the native primitive
/// was admitted in this namespace; an unavailable capability answers
/// with a typed [`Refusal::Capability`], never a no-op or a guessed
/// local execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Bounded observation (stat-class) of exact resources.
    pub observe: bool,
    /// Bounded directory listing.
    pub list: bool,
    /// Complete/ranged reads and follow observation.
    pub read: bool,
    /// Prepare/apply/verify mutations: create/copy/rename/move/Trash/
    /// restore/remove where 0054/0040 admit them, plus protected save.
    pub write: bool,
    /// Platform-native Trash with recovery location.
    pub trash: bool,
    /// Filesystem notification coverage for this namespace.
    pub notify: NotifyCoverage,
    /// Admitted finite native commands.
    pub exec_finite: bool,
    /// Admitted long-lived services (LSP-class leases).
    pub exec_service: bool,
    /// Local terminal/PTY ownership (0055); remote/container interactive
    /// terminals stay refused ahead of their authorized milestones.
    pub pty: bool,
}

/// One named capability, for typed refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Observe,
    List,
    Read,
    Write,
    Trash,
    Notify,
    ExecFinite,
    ExecService,
    Pty,
}

/// A typed refusal (WK02: explicit outcomes, never silent divergence).
/// Admission/freshness failures live here; filesystem effect failures
/// keep the strop-workspace `FsFailure` taxonomy inside operation
/// outcomes — the two layers never blur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Refusal {
    /// The envelope names a different worker incarnation: a restarted
    /// worker rejects every frame of the old process.
    #[error("wrong worker incarnation; the current incarnation is {current}")]
    WrongIncarnation { current: u64 },
    /// The lease is stale or foreign to this session.
    #[error("stale or foreign lease")]
    WrongLease,
    /// The named handle (stream, exec, subscription) is not live in
    /// this session; prepared authority never survives a restart.
    #[error("unknown handle: {message}")]
    UnknownHandle { message: String },
    /// The subscription's generation predates the current one; a
    /// reinstalled subscription invalidates the old identity.
    #[error("stale subscription; the current generation is {current}")]
    StaleSubscription { current: u64 },
    /// The capability was not advertised for this namespace.
    #[error("capability not admitted here: {capability:?}")]
    Capability { capability: Capability },
    /// A declared bound was exceeded (frame bytes, batch steps, queue).
    #[error("bound exceeded: {message}")]
    Limit { message: String },
    /// Control/cancellation capacity is reserved; this class is full.
    #[error("admission queue full: {message}")]
    Busy { message: String },
    /// The host/mount/container namespace changed under a retained
    /// authority; the operation fails closed.
    #[error("namespace changed; the current namespace is {current}")]
    NamespaceChanged { current: String },
    /// The worker is quiescing: no new mutations, outcomes still drain.
    #[error("worker is retiring")]
    Retiring,
    /// The session is closed; no further requests are admitted.
    #[error("worker session is closed")]
    Closed,
}

/// A protocol-level failure, independent of any admitted request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProtocolError {
    /// The offered protocol version is not spoken here.
    #[error("protocol version {offered} is not supported (supported: {supported})")]
    Version { supported: u32, offered: u32 },
    /// A well-framed body that is not a valid message of its direction.
    #[error("undecodable message: {message}")]
    Decode { message: String },
    /// Frame-level corruption; the stream is poisoned and closes.
    #[error("frame violation: {message}")]
    Frame { message: String },
    /// A stream chunk that names no live stream of this session, or
    /// violates chunk bounds/ordering. Child output can never arrive as
    /// a control envelope, so this is always protocol, never data.
    #[error("stream violation: {message}")]
    Stream { message: String },
    /// A message that does not belong at this point (requests before
    /// the handshake, or a second hello).
    #[error("unexpected message: {message}")]
    Unexpected { message: String },
}

/// Why the session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownReason {
    /// The client sent `shutdown`; the worker quiesced and exited.
    Requested,
    /// The lease's transport closed or died.
    Disconnect,
    /// Frame-level corruption poisoned the stream.
    ProtocolViolation,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_wire_shape_is_pinned() {
        let refusal = Refusal::WrongIncarnation { current: 9 };
        assert_eq!(
            serde_json::to_string(&refusal).unwrap(),
            r#"{"reason":"wrong_incarnation","current":9}"#
        );
        let refusal = Refusal::Capability {
            capability: Capability::Pty,
        };
        assert_eq!(
            serde_json::to_string(&refusal).unwrap(),
            r#"{"reason":"capability","capability":"pty"}"#
        );
    }

    #[test]
    fn coverage_round_trip() {
        for coverage in [
            NotifyCoverage::Native,
            NotifyCoverage::Polling,
            NotifyCoverage::OnDemand,
            NotifyCoverage::Unsupported,
        ] {
            let json = serde_json::to_string(&coverage).unwrap();
            let back: NotifyCoverage = serde_json::from_str(&json).unwrap();
            assert_eq!(back, coverage);
        }
        assert_eq!(
            serde_json::to_string(&NotifyCoverage::Native).unwrap(),
            r#""native""#
        );
    }

    #[test]
    fn limits_round_trip() {
        let limits = Limits {
            max_frame_bytes: crate::frame::MAX_BODY_BYTES,
            max_chunk_bytes: crate::codec::MAX_CHUNK_BYTES,
            max_pending_requests: 256,
            max_batch_steps: 512,
            max_listing_entries: 100_000,
            max_subscriptions: 64,
            max_streams: 128,
            max_exec_processes: 32,
        };
        let json = serde_json::to_string(&limits).unwrap();
        let back: Limits = serde_json::from_str(&json).unwrap();
        assert_eq!(back, limits);
    }
}
