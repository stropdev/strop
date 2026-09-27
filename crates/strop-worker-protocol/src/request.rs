//! The typed request/result/event families (0058 WK02). The wire reuses
//! strop-workspace's pure operation/observation contracts — there is no
//! second outcome taxonomy and no parallel command table.
//!
//! Families (docs/worker-protocol.md §4):
//! - observation: bounded observe/list/read/range over exact resources;
//! - mutation: prepare/apply/verify with frozen byte streams and typed
//!   receipts (protected save included);
//! - exec: admitted finite commands and services with stream stdin,
//!   half-close, output streams, exit and cancel — and the WK12 PTY
//!   family: spawn with geometry ([`ExecSpec::pty`]), stream input with
//!   delivered acknowledgments ([`Event::ExecInput`]), a bounded output
//!   stream, ordered resize ([`Request::ExecResize`]) and the typed
//!   exit;
//! - notify: first-class subscribe/unsubscribe/event/overflow/
//!   reconcile-boundary (the 2026-09-14/16 amendments);
//! - lifecycle: lease health, quiesce, shutdown.

use serde::{Deserialize, Serialize};
use strop_core::worker::cache_record::CacheGcReport;
use strop_workspace::operation::{
    FsFailure, LocatedObservation, OperationIntent, OperationRefusal, PreparedOperation,
    StepReceipt, VerifiedOutcome,
};
use strop_workspace::{DirectorySnapshot, ResourceLocation};

use crate::id::{
    DocumentStamp, ExecId, NamespaceIdentity, RequestId, Session, StreamId, Subscription,
};
use crate::message::{
    Capabilities, EndpointInfo, Limits, NotifyCoverage, ProtocolError, Refusal, RequestClass,
    ShutdownReason,
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

/// Initial finite-file/process-output chunk window. Fewer than the
/// client's 64 retained slots; consumption returns `stream_credit`.
pub const STREAM_WINDOW_CHUNKS: usize = 32;

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
    /// Read-only reconciliation after the issuing worker died. The
    /// frozen prepared attempt belongs to an older session and cannot
    /// be replayed as a write; exact boot/mount/principal identity must
    /// match before the fresh worker may verify observed state.
    VerifyRecovered {
        attempt: Box<StepReceipt>,
        namespace: NamespaceIdentity,
        binding: Option<DocumentStamp>,
    },
    /// Spawn an admitted finite command or leased service.
    Exec { spec: ExecSpec },
    /// Half-close the child's stdin: distinct from revoking its lease.
    ExecHalfClose { exec: ExecId },
    /// Cancel: revoke the lease with the established supervision order.
    ExecCancel { exec: ExecId },
    /// Resize one admitted PTY exec (0058 WK12). The worker applies
    /// `TIOCSWINSZ` in the exec's input order — every input chunk
    /// admitted before this request reaches the terminal first — and
    /// the `done` reply is the ordered resize boundary: output chunks
    /// read after it are under the new geometry. Refused with
    /// `unknown_handle` when the exec is unknown, `limit` when it has
    /// no PTY. PTY input has no half-close.
    ExecResize { exec: ExecId, geometry: PtyGeometry },
    /// Install a notify subscription over one logical scope. The result
    /// carries the fresh subscription identity and honest coverage.
    Subscribe {
        scope: ResourceLocation,
        recursive: bool,
    },
    /// Retire one subscription by its full identity.
    Unsubscribe { subscription: Subscription },
    /// Scoped maintenance of this worker principal's private cache.
    /// The client supplies its selected endpoint context; the worker
    /// holds the same OS lock used before every cached Welcome.
    CollectCache { context: String },
    /// Lease health: liveness without side effects.
    Health,
    /// Begin retirement: refuse new mutations, drain admitted work.
    Quiesce,
}

impl Request {
    /// The scheduling class (WK11): control requests draw from the
    /// reserved admission slots and their outcomes are scheduled ahead of
    /// bulk stream data; bulk reads are the bounded data producers.
    /// Cancellation of a specific exec is control; launching one is work.
    pub fn class(&self) -> RequestClass {
        match self {
            Request::Health
            | Request::Quiesce
            | Request::ExecCancel { .. }
            | Request::ExecHalfClose { .. }
            | Request::ExecResize { .. }
            | Request::Unsubscribe { .. } => RequestClass::Control,
            Request::Read { .. } => RequestClass::Bulk,
            _ => RequestClass::Standard,
        }
    }
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
    /// A bounded pass reports every committed deletion, even when a
    /// later OS failure or cancellation leaves the pass incomplete.
    CacheCollected {
        report: CacheGcReport,
        failure: Option<FsFailure>,
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
    /// One PTY input chunk was delivered to the terminal (0058 WK12):
    /// the acknowledgment that bounds the client's retained-input
    /// budget. `sequence` is the chunk's sequence on the exec's stdin
    /// stream; chunks are written in order, so an acknowledgment covers
    /// every earlier sequence too. Advisory accounting only — never a
    /// mutation receipt.
    ExecInput { exec: ExecId, sequence: u64 },
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
    /// A finite file or process-output chunk was consumed by the
    /// client. Grants slots back to that session's named bounded
    /// stream; already-finished streams ignore harmless late credits.
    /// Control-class so data cannot starve the producer's credits.
    StreamCredit {
        session: Session,
        stream: StreamId,
        chunks: u16,
    },
    /// The client dropped an unfinished exec output receiver. The
    /// worker keeps draining that child pipe without publishing more
    /// bytes, so process settlement cannot hang behind absent credits.
    /// File reads instead cancel their owning request.
    StreamAbandon { session: Session, stream: StreamId },
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
    use strop_workspace::operation::FsFailureKind;

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
            protocol: crate::PROTOCOL_VERSION,
            client: endpoint(),
        };
        assert_eq!(
            serde_json::to_string(&hello).unwrap(),
            r#"{"type":"hello","protocol":2,"client":{"name":"strop","version":"0.35.0","build":"locked","target":"x86_64-unknown-linux-musl"}}"#
        );
    }

    #[test]
    fn stream_credit_is_a_stamped_control_envelope() {
        let credit = ClientMessage::StreamCredit {
            session: session(),
            stream: StreamId(4),
            chunks: 1,
        };
        let json = serde_json::to_string(&credit).unwrap();
        assert_eq!(
            json,
            r#"{"type":"stream_credit","session":{"incarnation":11,"lease":2},"stream":4,"chunks":1}"#
        );
        assert_eq!(
            serde_json::from_str::<ClientMessage>(&json).unwrap(),
            credit
        );
    }

    #[test]
    fn stream_abandon_remains_bound_to_the_admitting_session() {
        let abandon = ClientMessage::StreamAbandon {
            session: session(),
            stream: StreamId(6),
        };
        let json = serde_json::to_string(&abandon).unwrap();
        assert_eq!(
            json,
            r#"{"type":"stream_abandon","session":{"incarnation":11,"lease":2},"stream":6}"#
        );
        assert_eq!(
            serde_json::from_str::<ClientMessage>(&json).unwrap(),
            abandon
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
    fn cache_collect_wire_preserves_scope_and_partial_retirements() {
        let request = ClientMessage::Request {
            session: session(),
            id: RequestId(7),
            body: Box::new(Request::CollectCache {
                context: "ssh://selected-host".into(),
            }),
        };
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"type":"request","session":{"incarnation":11,"lease":2},"id":7,"body":{"op":"collect_cache","context":"ssh://selected-host"}}"#
        );
        let report = CacheGcReport {
            removed_objects: vec!["a".repeat(64)],
            removed_receipts: 0,
            kept_objects: 2,
        };
        let response = WorkerMessage::Result {
            id: RequestId(7),
            outcome: ResultOutcome::CacheCollected {
                report,
                failure: Some(FsFailure::new(
                    FsFailureKind::Incomplete,
                    "receipt unlink failed",
                )),
            },
        };
        let encoded = serde_json::to_value(response).unwrap();
        assert_eq!(encoded["type"], "result");
        assert_eq!(encoded["outcome"]["outcome"], "cache_collected");
        assert_eq!(
            encoded["outcome"]["report"]["removed_objects"][0],
            "a".repeat(64)
        );
        assert_eq!(encoded["outcome"]["report"]["kept_objects"], 2);
        assert_eq!(encoded["outcome"]["failure"]["kind"], "Incomplete");
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
    #[test]
    fn exec_resize_wire_shape_is_pinned() {
        let request = Request::ExecResize {
            exec: ExecId(7),
            geometry: PtyGeometry {
                columns: 120,
                rows: 40,
            },
        };
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            json,
            r#"{"op":"exec_resize","exec":7,"geometry":{"columns":120,"rows":40}}"#
        );
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(back, request);
        // The resize rides the control class: it never queues behind bulk.
        assert!(matches!(request.class(), RequestClass::Control));
    }

    #[test]
    fn exec_input_ack_wire_shape_is_pinned() {
        let event = Event::ExecInput {
            exec: ExecId(7),
            sequence: 41,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, r#"{"event":"exec_input","exec":7,"sequence":41}"#);
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }
}
