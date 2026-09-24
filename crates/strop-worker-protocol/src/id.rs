//! Identity types: the incarnation triple and the handle vocabulary.
//!
//! The triple (0058 §4, 2026-09-16 amendment):
//! 1. the fresh worker **session** — [`Session`] pairs a per-process
//!    incarnation with the lease the handshake granted; a restarted worker
//!    never accepts an old session's handles or permits;
//! 2. the stable host/mount **namespace** — [`NamespaceIdentity`], a native
//!    observation (boot/mount/container incarnation), not a display
//!    hostname; a changed namespace fails closed;
//! 3. the editor's **document binding** — [`DocumentStamp`], carried
//!    opaquely through mutation requests and echoed in receipts so a late
//!    committed outcome reconciles its source exactly once. The worker
//!    never interprets it and never manufactures editor permits.

use serde::{Deserialize, Serialize};
use strop_core::id::{BufferRevision, DocumentId};

/// One admitted request. Client-allocated, monotone per session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub u64);

/// One bulk byte stream in either direction: file read payload, frozen save
/// content, process stdin/stdout/stderr, PTY VT bytes. Chunks on a stream
/// are ordered by sequence; the stream belongs to its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamId(pub u64);

/// The lease the handshake granted. Worker lifetime is leased to its client
/// session: no public listener, no reconnect daemon, no cross-session
/// sharing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LeaseId(pub u64);

/// One admitted exec: finite command or service. Owns its process/pipe/wait
/// state; child output flows on its streams, never as control messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecId(pub u64);

/// One notify subscription. Reinstallation after loss/overflow re-enters
/// with a bumped generation, so reused descriptors or inodes cannot
/// resurrect old authority; events name the generation they belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Subscription {
    pub id: u64,
    pub generation: u64,
}

/// The session authority stamped on every post-handshake client envelope.
/// Both halves are fresh per worker process; a restarted worker holds a
/// different pair and refuses old frames with a typed refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Session {
    pub incarnation: u64,
    pub lease: LeaseId,
}

/// Stable host/namespace identity, distinct from the session incarnation:
/// a native boot/mount/container observation plus the execution principal.
/// A changed value between reconnects is a different namespace and fails
/// closed — old write authority never transfers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceIdentity {
    /// Native incarnation evidence (boot ID, mount/container incarnation),
    /// never a display hostname alone.
    pub identity: String,
    /// The execution principal (uid), when the platform reports one.
    pub principal: Option<u32>,
}

/// The editor-side document binding, opaque to the worker. Mutation
/// requests may carry it; receipts echo it unchanged. The editor checks
/// its captured owner/binding/revision before granting authority or
/// updating a source — the wire never decides that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentStamp {
    pub document: DocumentId,
    pub revision: BufferRevision,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_transparent_u64s() {
        assert_eq!(serde_json::to_string(&RequestId(7)).unwrap(), "7");
        assert_eq!(serde_json::to_string(&StreamId(9)).unwrap(), "9");
        assert_eq!(serde_json::to_string(&LeaseId(3)).unwrap(), "3");
        assert_eq!(serde_json::to_string(&ExecId(1)).unwrap(), "1");
    }

    #[test]
    fn subscription_pin() {
        let subscription = Subscription {
            id: 4,
            generation: 2,
        };
        assert_eq!(
            serde_json::to_string(&subscription).unwrap(),
            r#"{"id":4,"generation":2}"#
        );
        let back: Subscription = serde_json::from_str(r#"{"id":4,"generation":2}"#).unwrap();
        assert_eq!(back, subscription);
    }

    #[test]
    fn session_pin() {
        let session = Session {
            incarnation: 42,
            lease: LeaseId(7),
        };
        assert_eq!(
            serde_json::to_string(&session).unwrap(),
            r#"{"incarnation":42,"lease":7}"#
        );
    }

    #[test]
    fn namespace_identity_keeps_principal() {
        let namespace = NamespaceIdentity {
            identity: "boot:9e1b…".to_owned(),
            principal: Some(1000),
        };
        let json = serde_json::to_string(&namespace).unwrap();
        let back: NamespaceIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(back, namespace);
    }
}
