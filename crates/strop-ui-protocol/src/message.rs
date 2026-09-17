//! Protocol envelopes (0056 AR09 §8): versioned handshake, admitted
//! actions, acknowledgements with explicit outcomes, semantic
//! snapshots/deltas, host effects, resynchronization and shutdown.
//!
//! Envelopes are distinct from LSP's — only the Content-Length byte
//! convention is shared ([`crate::frame`]). Every type here is pure
//! data; the engine mapping lives in the backend, the application
//! rules in [`crate::client`].

use serde::{Deserialize, Serialize};
use strop_core::frontend_input::Input;
use strop_core::id::{BufferRevision, DocumentId};

/// The only wire version this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// The state an action claims to be based on: the backend incarnation
/// and the newest view generation the client has applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseStamp {
    pub incarnation: u64,
    pub generation: u64,
}

/// Who the client is, for the handshake record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

/// What the client can absorb. Everything defaults to withheld: a
/// client that cannot accept clipboard writes never receives one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientCapabilities {
    pub clipboard_write: bool,
}

/// Who/what the backend is. `incarnation` is unique per backend
/// process: publications and actions key on it so state from a previous
/// backend can never resolve against a restarted one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendInfo {
    pub name: String,
    pub version: String,
    pub build: Option<String>,
    pub incarnation: u64,
}

/// The backend's hard bounds (AR06 conventions), restated on the wire
/// so the client never has to guess them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionLimits {
    pub max_frame_bytes: usize,
    pub max_pending_requests: usize,
    pub max_viewport_cells: u32,
}

/// The currently-implemented TUI families this backend serves (AR09 §8:
/// only real families are advertised).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCapabilities {
    pub terminals: bool,
    pub workspace_search: bool,
    pub filesystem: bool,
    pub clipboard_write: bool,
}

/// One admitted action — a one-to-one mirror of the engine's admitted
/// input surface (`AppEvent`'s client-initiated subset). The protocol
/// adds no commands of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", content = "data", rename_all = "snake_case")]
pub enum AdmittedAction {
    /// Physical/logical input; the engine selects the owner.
    Input(Input),
    /// Already-normalized semantic input (scripted/editor commands).
    EditorKey(strop_engine::editor::Key),
    /// Bracketed paste: one text payload, never a key stream.
    Paste(String),
    /// Terminal resized.
    Resize { columns: u16, rows: u16 },
    /// ctrl-c: the quit intent; the editor's policy decides.
    QuitIntent,
    /// Focus change.
    Focus(bool),
}

/// Terminal-cell viewport geometry (wire mirror of the engine's
/// `ViewGeometry`; frontends convert at their edge).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub columns: u16,
    pub rows: u16,
}

/// One rectangle in terminal cells (wire mirror of `CellRect`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// Declared bounds of a pane window (wire mirror of the engine's AR03
/// `WindowBounds`): every state is explicit; there is no empty success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewBounds {
    Complete,
    Partial,
    Loading,
    Stale,
    Error,
}

/// One pane's semantic window, keyed by document identity + the
/// revision the window was prepared against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneSnapshot {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub bounds: ViewBounds,
    pub cursor: usize,
    pub view_top: usize,
    pub hscroll: usize,
    pub terminal_input: bool,
    pub overlays: bool,
    pub rect: Rect,
    pub budget: Rect,
    /// The text window: lines starting at `window_top`, bounded by the
    /// pane's row budget and the document's length.
    pub window_top: usize,
    pub lines: Vec<String>,
}

/// A full semantic view: the prepared panes plus the same logical state
/// observation the headless driver pins (`state_json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewSnapshot {
    pub generation: u64,
    pub geometry: Geometry,
    pub active_pane: usize,
    pub panes: Vec<PaneSnapshot>,
    pub state: serde_json::Value,
}

/// A sparse view update valid only against `base`. `panes` is aligned
/// with the base snapshot's pane order; a changed pane set always ships
/// as a full snapshot instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewDelta {
    pub base: u64,
    pub generation: u64,
    pub geometry: Option<Geometry>,
    pub active_pane: Option<usize>,
    pub panes: Vec<PaneDelta>,
    pub state: Option<serde_json::Value>,
}

/// One pane slot's delta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneDelta {
    Unchanged,
    Changed(PaneSnapshot),
}

/// The outcome of one acknowledged request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AckOutcome {
    /// Applied; `applied` is the backend's monotone action counter and
    /// `generation` the view generation after application.
    Applied { applied: u64, generation: u64 },
    /// Refused with a typed reason; nothing was applied.
    Refused { refusal: Refusal },
}

/// A typed refusal (AR09: explicit outcomes, never silent divergence).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Refusal {
    /// The action's base predates the newest publication: the client
    /// missed a delta and must resync before acting.
    #[error("stale base generation; the current generation is {current}")]
    StaleGeneration { current: u64 },
    /// The action claims a generation the backend has not reached.
    #[error("future base generation; the current generation is {current}")]
    FutureGeneration { current: u64 },
    /// The action names a different backend incarnation.
    #[error("wrong backend incarnation; the current incarnation is {current}")]
    WrongIncarnation { current: u64 },
    /// A declared bound was exceeded (viewport cells, frame bytes).
    #[error("bound exceeded: {message}")]
    Limit { message: String },
    /// The admitted engine path itself reported an I/O failure.
    #[error("engine: {message}")]
    Engine { message: String },
    /// The backend is shutting down; no further actions are admitted.
    #[error("backend is closing")]
    Closed,
}

/// A host effect the backend requests of the client (AR08): the client
/// answers with [`EffectOutcome`]. The engine stages OSC52 clipboard
/// payloads for its frontend; over the protocol the client is it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectRequest {
    ClipboardWrite { text: String },
}

/// The client's answer to an [`EffectRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EffectOutcome {
    Applied,
    Refused { reason: String },
}

/// Why the session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownReason {
    /// The client sent `shutdown`.
    Requested,
    /// The editor quit through its own grammar (`:q`, quit intent).
    Quit,
    /// The client link closed or died.
    Disconnect,
    /// Frame-level corruption poisoned the stream.
    ProtocolViolation,
}

/// A protocol-level failure, independent of any admitted action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProtocolError {
    /// The offered protocol version is not spoken here.
    #[error("protocol version {offered} is not supported (supported: {supported})")]
    Version { supported: u32, offered: u32 },
    /// A well-framed body that is not a valid client message.
    #[error("undecodable message: {message}")]
    Decode { message: String },
    /// Frame-level corruption; the stream is poisoned and closes.
    #[error("frame violation: {message}")]
    Frame { message: String },
    /// A message that does not belong at this point (e.g. actions
    /// before the handshake, or a second hello).
    #[error("unexpected message: {message}")]
    Unexpected { message: String },
}

/// Client → backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// The first and only pre-handshake message.
    Hello {
        protocol: u32,
        client: ClientInfo,
        capabilities: ClientCapabilities,
    },
    /// Admitted actions, applied in order, on the stated base. One
    /// sequence number covers the batch; the acknowledgement carries
    /// the post-application generation.
    Act {
        seq: u64,
        base: BaseStamp,
        actions: Vec<AdmittedAction>,
    },
    /// Viewport interest: the geometry preparation runs against. The
    /// engine sees the same resize a TUI would deliver.
    Viewport { seq: u64, columns: u16, rows: u16 },
    /// Request a complete current snapshot — the only recovery from a
    /// dropped delta or a poisoned client.
    Resync { seq: u64 },
    /// Answer a host effect request.
    EffectResult { id: u64, outcome: EffectOutcome },
    /// Authorized orderly shutdown: the backend finishes background
    /// work, publishes `bye` and exits.
    Shutdown { seq: u64 },
}

/// Backend → client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// The handshake answer, carrying identity, bounds and the real
    /// capability set.
    Welcome {
        protocol: u32,
        backend: BackendInfo,
        limits: ActionLimits,
        capabilities: ServerCapabilities,
    },
    /// The acknowledgement of one sequenced request.
    Ack { seq: u64, outcome: AckOutcome },
    /// A complete semantic view; valid against any prior state.
    Snapshot {
        incarnation: u64,
        view: ViewSnapshot,
    },
    /// A sparse update valid only against its `base` generation.
    Delta { incarnation: u64, delta: ViewDelta },
    /// A host effect request (AR08).
    Effect { id: u64, effect: EffectRequest },
    /// A protocol-level failure. `seq` names the offending request when
    /// one could be identified.
    Error {
        seq: Option<u64>,
        error: ProtocolError,
    },
    /// The final message: the backend is exiting.
    Bye { reason: ShutdownReason },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The envelope round-trip pins the wire shape: envelopes stay
    /// distinct typed data, never skip-garbage JSON sniffing.
    #[test]
    fn envelopes_round_trip() {
        let messages = [
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                client: ClientInfo {
                    name: "driver".into(),
                    version: "0.1".into(),
                },
                capabilities: ClientCapabilities {
                    clipboard_write: true,
                },
            },
            ClientMessage::Act {
                seq: 7,
                base: BaseStamp {
                    incarnation: 42,
                    generation: 3,
                },
                actions: vec![
                    AdmittedAction::Input(Input::Text("héllo".into())),
                    AdmittedAction::EditorKey(strop_engine::editor::Key::Enter),
                    AdmittedAction::Resize {
                        columns: 120,
                        rows: 40,
                    },
                    AdmittedAction::QuitIntent,
                ],
            },
            ClientMessage::Resync { seq: 8 },
            ClientMessage::EffectResult {
                id: 1,
                outcome: EffectOutcome::Refused {
                    reason: "no clipboard".into(),
                },
            },
            ClientMessage::Shutdown { seq: 9 },
        ];
        for message in messages {
            let bytes = serde_json::to_vec(&message).unwrap();
            assert_eq!(
                serde_json::from_slice::<ClientMessage>(&bytes).unwrap(),
                message
            );
        }
    }

    #[test]
    fn server_envelopes_round_trip() {
        let messages = [
            ServerMessage::Ack {
                seq: 1,
                outcome: AckOutcome::Applied {
                    applied: 5,
                    generation: 9,
                },
            },
            ServerMessage::Ack {
                seq: 2,
                outcome: AckOutcome::Refused {
                    refusal: Refusal::StaleGeneration { current: 11 },
                },
            },
            ServerMessage::Effect {
                id: 0,
                effect: EffectRequest::ClipboardWrite {
                    text: "payload".into(),
                },
            },
            ServerMessage::Error {
                seq: None,
                error: ProtocolError::Version {
                    supported: 1,
                    offered: 99,
                },
            },
            ServerMessage::Bye {
                reason: ShutdownReason::ProtocolViolation,
            },
        ];
        for message in messages {
            let bytes = serde_json::to_vec(&message).unwrap();
            assert_eq!(
                serde_json::from_slice::<ServerMessage>(&bytes).unwrap(),
                message
            );
        }
    }

    /// An unknown envelope type is a decode failure, not a guess.
    #[test]
    fn unknown_envelopes_fail_decoding() {
        assert!(serde_json::from_slice::<ClientMessage>(br#"{"type":"teleport"}"#).is_err());
        assert!(serde_json::from_slice::<ServerMessage>(br#"{"type":"teleport"}"#).is_err());
    }
}
