//! The typed request/result/event families (0058 WK02). The wire reuses
//! strop-workspace's pure operation/observation contracts — there is no
//! second outcome taxonomy and no parallel command table.
//!
//! Families (docs/worker-protocol.md §4):
//! - observation: bounded observe/list/read/range over exact resources;
//! - mutation: prepare/apply/verify with frozen byte streams and typed
//!   receipts (protected save included);
//! - exec: admitted finite commands and services with stream stdin,
//!   half-close, output streams, exit and cancel;
//! - notify: first-class subscribe/unsubscribe/event/overflow/
//!   reconcile-boundary (the 2026-09-14/16 amendments);
//! - lifecycle: lease health, quiesce, shutdown.

use serde::{Deserialize, Serialize};
use strop_workspace::operation::{
    FsFailure, LocatedObservation, OperationIntent, OperationRefusal, PreparedOperation,
    StepReceipt, VerifiedOutcome,
};
use strop_workspace::{DirectorySnapshot, ResourceLocation};

use crate::id::{
    DocumentStamp, ExecId, NamespaceIdentity, RequestId, Session, StreamId, Subscription,
};
use crate::message::{
    Capabilities, EndpointInfo, Limits, NotifyCoverage, ProtocolError, Refusal, ShutdownReason,
};

/// Native argv/environment entry: byte-exact, never lossy UTF-8. Control
/// envelopes carry these as bounded JSON byte strings; bulk payloads
/// never appear here (they flow as [`crate::codec::StreamChunk`]s).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

/// Terminal geometry for an admitted PTY (0055's contract; cells).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyGeometry {
    pub columns: u16,
    pub rows: u16,
}

/// One admitted native execution. Program/argv/cwd/env are native bytes
/// in the selected namespace; no shell ever interprets them. `service`
/// distinguishes a long-lived leased service (LSP-class) from a finite
/// command; both carry the same stream/lease semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecSpec {
    pub program: Vec<u8>,
    pub argv: Vec<Vec<u8>>,
    pub cwd: Vec<u8>,
    pub env: Vec<EnvVar>,
    pub service: bool,
    /// Request a PTY with this geometry; absent for piped stdio. Only
    /// admitted where `capabilities.pty` holds.
    pub pty: Option<PtyGeometry>,
}

/// A frozen content stream the client pushes for an apply/save: the
/// digest identifies the intended bytes, and the worker counts exactly
/// `bytes` before publication. The stream is session-scoped like every
/// other handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamRef {
    pub stream: StreamId,
    pub bytes: u64,
    pub digest: [u8; 32],
}

/// A client session's trash-root environment override for `prepare`:
/// native byte-exact paths, never lossy (WK04 amendment). Absent fields
/// fall back to the worker's captured environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EnvironmentOverride {
    #[serde(with = "strop_core::path_serde::option")]
    pub home: Option<std::path::PathBuf>,
    #[serde(with = "strop_core::path_serde::option")]
    pub data_home: Option<std::path::PathBuf>,
}

/// Client → worker request bodies. Mutation requests may carry a
/// [`DocumentStamp`] binding, echoed unchanged by receipts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Bounded stat-class observation of exact resources.
    Observe { locations: Vec<ResourceLocation> },
    /// One bounded listing page; `cursor` continues an earlier page.
    List {
        location: ResourceLocation,
        cursor: Option<String>,
    },
    /// A complete or ranged read. The payload streams back on the
    /// stream named by the result; length absent reads to EOF within
    /// the negotiated bounds.
    Read {
        location: ResourceLocation,
        offset: u64,
        length: Option<u64>,
    },
    /// Prepare a batch: exact source/destination/parent observations
    /// and the admitted capability, no effects yet. `environment`
    /// overrides the worker's captured trash-root context when the
    /// client session admitted a different one (WK04 amendment — the
    /// engine's per-session override must reach prepare, since trash
    /// roots are prepare-time policy baked into prepared capability).
    Prepare {
        intents: Vec<OperationIntent>,
        binding: Option<DocumentStamp>,
        #[serde(default)]
        environment: Option<EnvironmentOverride>,
    },
    /// Apply a prepared batch. Frozen byte content, when a step needs
    /// it (save), arrives on the named stream and is verified against
    /// its declared digest/length before publication.
    Apply {
        steps: Vec<PreparedOperation>,
        content: Option<StreamRef>,
        binding: Option<DocumentStamp>,
    },
    /// Verify an uncertain completed attempt: retained
    /// before/intended/observed evidence plus newly admitted read
    /// authority. Never revives an old session's write capability.
    Verify {
        attempt: Box<StepReceipt>,
        binding: Option<DocumentStamp>,
    },
    /// Spawn an admitted finite command or leased service.
    Exec { spec: ExecSpec },
    /// Half-close the child's stdin: distinct from revoking its lease.
    ExecHalfClose { exec: ExecId },
    /// Cancel: revoke the lease with the established supervision order.
    ExecCancel { exec: ExecId },
    /// Install a notify subscription over one logical scope. The result
    /// carries the fresh subscription identity and honest coverage.
    Subscribe {
        scope: ResourceLocation,
        recursive: bool,
    },
    /// Retire one subscription by its full identity.
    Unsubscribe { subscription: Subscription },
    /// Lease health: liveness without side effects.
    Health,
    /// Begin retirement: refuse new mutations, drain admitted work.
    Quiesce,
}

/// The outcome of one admitted request. `Refused` carries the typed
/// admission refusal; per-family payloads carry the workspace outcome
/// taxonomy unchanged (committed/refused/cancelled/unconfirmed stay
/// distinct inside [`StepReceipt`]).
/// (`DirectorySnapshot` is intentionally not `PartialEq` — snapshots are
/// observations, compared by identity fields at the consumer.)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ResultOutcome {
    Observations {
        observations: Vec<LocatedObservation>,
    },
    Listing {
        snapshot: DirectorySnapshot,
        cursor: Option<String>,
    },
    /// Read admitted: payload bytes follow as chunks on `stream`, ended
    /// by a `last` chunk; `size` is the observed total when known.
    ReadOpened {
        stream: StreamId,
        size: Option<u64>,
    },
    /// Prepared batch: the exact admitted steps plus every per-intent
    /// refusal (WK04 amendment — the review surface renders refusals, so
    /// dropping them would silently weaken the in-process parity).
    Prepared {
        steps: Vec<PreparedOperation>,
        refused: Vec<OperationRefusal>,
    },
    Applied {
        receipts: Vec<StepReceipt>,
        binding: Option<DocumentStamp>,
    },
    Verified {
        verified: VerifiedOutcome,
        binding: Option<DocumentStamp>,
    },
    /// Process admitted; stdin (when piped), stdout and stderr are
    /// streams. Exit arrives as an [`Event::ExecExit`].
    ExecStarted {
        exec: ExecId,
        stdin: Option<StreamId>,
        stdout: StreamId,
        stderr: StreamId,
    },
    Subscribed {
        subscription: Subscription,
        coverage: NotifyCoverage,
    },
    Healthy,
    Quiesced,
    Done,
    Refused {
        refusal: Refusal,
    },
    /// A domain failure of the observation/read/exec-admission family
    /// (WK04 amendment): the workspace `FsFailure` taxonomy rides here,
    /// never blurred into the protocol's admission [`Refusal`].
    Failed {
        failure: FsFailure,
    },
}

/// An exec's terminal status. `Lost` means the supervisor cannot attest
/// the exit — never silently mapped to a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitStatus {
    Exit(i32),
    Signal(i32),
    Lost,
}

/// One advisory freshness hint (2026-09-14 amendment: events are hints,
/// never authority). Nothing here is an authoritative edit, save receipt
/// or write log; the editor invalidates and reobserves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyHint {
    /// Native path bytes relative to the subscribed scope.
    pub path: Vec<u8>,
    pub kind: NotifyKind,
}

/// Hint kinds. Rename pairs are advisory: an ambiguous rename is
/// reconciled, never a silent relocation of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    Created,
    Removed,
    Modified,
    Renamed,
    /// Coverage could not decide; reconcile by observation.
    Ambiguous,
}

/// Worker → client, unsolicited. Notify events carry the subscription
/// identity (id + generation) and a per-subscription monotone sequence
/// so the reconcile boundary cannot erase a newer invalidation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Advisory freshness hints for one subscription.
    Notify {
        subscription: Subscription,
        sequence: u64,
        hints: Vec<NotifyHint>,
    },
    /// Native queue overflow / partial coverage loss: the subscription's
    /// baseline is invalidated; reobserve and reestablish coverage
    /// before claiming freshness. Never silently dropped.
    NotifyOverflow {
        subscription: Subscription,
        sequence: u64,
    },
    /// The initial scan is complete at this sequence: events at or
    /// below it are covered by the scan, later ones are not.
    ReconcileBoundary {
        subscription: Subscription,
        sequence: u64,
    },
    /// An admitted exec reached its terminal status.
    ExecExit { exec: ExecId, status: ExitStatus },
}

/// Client → worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// The first and only pre-handshake message.
    Hello { protocol: u32, client: EndpointInfo },
    /// One admitted request, stamped with the session authority.
    Request {
        session: Session,
        id: RequestId,
        body: Box<Request>,
    },
    /// Cancel one in-flight request; cancellation is fair and never
    /// coalesces accepted mutations or outcomes.
    Cancel { session: Session, id: RequestId },
    /// Authorized orderly shutdown: quiesce, drain, publish `bye`.
    Shutdown { session: Session },
}

/// Worker → client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    /// The handshake answer: identity, fresh session authority,
    /// namespace identity, hard limits and the admitted capability set.
    Welcome {
        protocol: u32,
        worker: EndpointInfo,
        session: Session,
        namespace: NamespaceIdentity,
        limits: Limits,
        capabilities: Capabilities,
    },
    /// The outcome of one admitted request.
    Result {
        id: RequestId,
        outcome: ResultOutcome,
    },
    /// An unsolicited event (notify, exec exit).
    Event { event: Event },
    /// A protocol-level failure; `id` names the offending request when
    /// one could be identified.
    Error {
        id: Option<RequestId>,
        error: ProtocolError,
    },
    /// The final message: the worker is exiting.
    Bye { reason: ShutdownReason },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::LeaseId;
    use crate::message::Capability;

    fn session() -> Session {
        Session {
            incarnation: 11,
            lease: LeaseId(2),
        }
    }

    fn endpoint() -> EndpointInfo {
        EndpointInfo {
            name: "strop".to_owned(),
            version: "0.35.0".to_owned(),
            build: Some("locked".to_owned()),
            target: "x86_64-unknown-linux-musl".to_owned(),
        }
    }

    #[test]
    fn hello_wire_shape_is_pinned() {
        let hello = ClientMessage::Hello {
            protocol: 1,
            client: endpoint(),
        };
        assert_eq!(
            serde_json::to_string(&hello).unwrap(),
            r#"{"type":"hello","protocol":1,"client":{"name":"strop","version":"0.35.0","build":"locked","target":"x86_64-unknown-linux-musl"}}"#
        );
    }

    #[test]
    fn request_envelope_carries_session() {
        let request = ClientMessage::Request {
            session: session(),
            id: RequestId(1),
            body: Box::new(Request::Health),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            json,
            r#"{"type":"request","session":{"incarnation":11,"lease":2},"id":1,"body":{"op":"health"}}"#
        );
        let back: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, request);
    }

    #[test]
    fn exec_spec_keeps_native_bytes() {
        let spec = ExecSpec {
            program: b"/bin/echo".to_vec(),
            argv: vec![b"echo".to_vec(), vec![0xff, b'a']],
            cwd: b"/tmp".to_vec(),
            env: vec![EnvVar {
                name: b"LC_ALL".to_vec(),
                value: b"C".to_vec(),
            }],
            service: false,
            pty: None,
        };
        let json = serde_json::to_string(&Request::Exec { spec: spec.clone() }).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Request::Exec { spec });
    }

    #[test]
    fn notify_events_carry_subscription_and_sequence() {
        let event = Event::ReconcileBoundary {
            subscription: Subscription {
                id: 3,
                generation: 1,
            },
            sequence: 41,
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"event":"reconcile_boundary","subscription":{"id":3,"generation":1},"sequence":41}"#
        );
    }

    #[test]
    fn refused_outcome_round_trip() {
        let outcome = ResultOutcome::Refused {
            refusal: Refusal::Capability {
                capability: Capability::Notify,
            },
        };
        let json = serde_json::to_string(&outcome).unwrap();
        assert_eq!(
            json,
            r#"{"outcome":"refused","refusal":{"reason":"capability","capability":"notify"}}"#
        );
        let back: ResultOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn exit_status_pin() {
        assert_eq!(
            serde_json::to_string(&ExitStatus::Signal(9)).unwrap(),
            r#"{"signal":9}"#
        );
    }
}
