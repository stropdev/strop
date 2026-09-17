//! The strop UI protocol (0056 AR09/AR10): one bounded, versioned,
//! non-graphical contract between the `strop --ui-stdio` backend and any
//! frontend process (the future Windows GUI, test drivers, automation).
//!
//! Shape:
//! - Bytes: bounded Content-Length framing ([`frame`]), the convention
//!   strop-lsp already pins at its trace boundary. stdout carries frames
//!   only; diagnostics stay on stderr.
//! - Envelopes: [`message`] — handshake, admitted actions, acknowledgements
//!   with explicit outcomes, semantic snapshots/deltas keyed by backend
//!   incarnation + view generation, host effects, resync and shutdown.
//! - Actions: [`message::AdmittedAction`] mirrors the engine's admitted
//!   input surface one-to-one — there is no parallel command table.
//! - Client: [`client::Client`] is the pure readonly cache (snapshot/delta
//!   application, resync poisoning); [`driver::Driver`] is the stdio
//!   process transport with deterministic state/revision barriers.
//!
//! The client is never an editing model: it holds the last published
//! view, refuses to act on stale or poisoned state, and recovers through
//! an explicit resync — never by blindly applying a delta.

pub mod client;
pub mod driver;
pub mod frame;
pub mod message;

pub use client::{Client, ClientError, ClientEvent, ResyncReason};
pub use driver::{Driver, DriverError};
pub use frame::{FrameDecoder, FrameError, MAX_BODY_BYTES, MAX_HEADER_BYTES};
pub use message::{
    AckOutcome, ActionLimits, AdmittedAction, BackendInfo, BaseStamp, ClientCapabilities,
    ClientInfo, ClientMessage, EffectOutcome, EffectRequest, Geometry, PaneDelta, PaneSnapshot,
    ProtocolError, Rect, Refusal, ServerCapabilities, ServerMessage, ShutdownReason, ViewBounds,
    ViewDelta, ViewSnapshot, PROTOCOL_VERSION,
};

/// Bound on client requests queued between the transport reader and the
/// server loop (AR06 convention: the LSP queue admits 256 jobs).
pub const MAX_PENDING_REQUESTS: usize = 256;

/// Geometry cap, the same one the engine's frame action enforces.
pub const MAX_VIEWPORT_CELLS: u32 = 1_000_000;
