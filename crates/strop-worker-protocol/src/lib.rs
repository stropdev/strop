//! The strop worker protocol (0058 WK02): one canonical, bounded, versioned
//! wire between the editor and the same release's native worker, over local
//! child stdio, OpenSSH stdio or selected container exec stdio.
//!
//! The normative contract is `docs/worker-protocol.md` (WK01); this crate is
//! its sign-off by construction — the codec implements exactly the documented
//! families, no more.
//!
//! Shape:
//! - Bytes: bounded Content-Length framing ([`frame`]), the convention
//!   strop-ui-protocol and strop-lsp already pin. stdout carries frames
//!   only; diagnostics stay on private bounded stderr.
//! - Bodies: [`codec`] — a one-byte class tag, then either a JSON control
//!   envelope ([`ClientMessage`]/[`WorkerMessage`]) or a binary stream
//!   chunk. Bulk file/VT/process bytes never become JSON arrays, base64
//!   copies or single giant buffers; they flow as bounded [`StreamChunk`]s.
//! - Envelopes: [`message`] — handshake with incarnation-keyed session
//!   authority, typed request/result/event families, typed refusals and a
//!   protocol-error/bye surface.
//! - Freshness: [`guard`] — the worker-side authority check: a restarted
//!   worker rejects old sessions, leases, subscriptions and handles; no
//!   mutation replays after connection loss.
//!
//! Identity types ([`id`]) keep the incarnation triple distinct: the fresh
//! worker session, the stable host/mount namespace incarnation, and the
//! editor's DocumentId/revision binding carried opaquely through mutation
//! receipts. Filesystem outcome taxonomy is not duplicated here: the wire
//! reuses `strop-workspace`'s pure operation/observation contracts.

pub mod codec;
pub mod frame;
pub mod guard;
pub mod id;
pub mod message;
pub mod request;

pub use codec::{CodecError, Incoming, StreamChunk, MAX_CHUNK_BYTES};
pub use frame::{FrameDecoder, FrameError, MAX_BODY_BYTES, MAX_HEADER_BYTES};
pub use guard::Authority;
pub use id::{
    DocumentStamp, ExecId, LeaseId, NamespaceIdentity, RequestId, Session, StreamId, Subscription,
};
pub use message::{
    Capabilities, Capability, EndpointInfo, Limits, NotifyCoverage, ProtocolError, Refusal,
    ShutdownReason, PROTOCOL_VERSION,
};
pub use request::{
    ClientMessage, Event, ExecSpec, ExitStatus, NotifyHint, NotifyKind, PtyGeometry, Request,
    ResultOutcome, StreamRef, WorkerMessage,
};
