//! Typed failures for every container conversation. A caller must always be
//! able to tell "engine gone" from "container gone" from "this build of
//! strop refuses that operation" — those have different UI and retry rules.

/// Why a container operation did not produce a result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContainerError {
    /// The local engine cannot be reached: the CLI is missing, the daemon
    /// is down, or the probe timed out. Retrying later is meaningful.
    #[error("container engine unavailable: {detail}")]
    EngineUnavailable { detail: String },

    /// The name or id resolved to nothing on the engine.
    #[error("no such container: {name}")]
    NoSuchContainer { name: String },

    /// The container exists but is not running; attach semantics require a
    /// running container even though the engine could read stopped ones.
    #[error("container is not running: {id}")]
    NotRunning { id: String },

    /// A previously resolved identity no longer matches the engine: the
    /// name now maps to a different container, or the same container id
    /// restarted (a new `StartedAt` incarnation). Old reads, caches and
    /// completions must not attach to the new incarnation.
    #[error(
        "stale container identity for {name}: held incarnation {expected}, engine reports {found}"
    )]
    StaleIdentity {
        name: String,
        expected: String,
        found: String,
    },

    /// A path inside the container does not exist.
    #[error("no such path in container {id}: {path}")]
    NoSuchPath { id: String, path: String },

    /// The operation is outside this crate's read-only capability
    /// boundary (writes, arbitrary exec, reading a directory as a file).
    /// This is a policy answer, not a failure to be retried.
    #[error("capability refused: {what}")]
    CapabilityRefused { what: String },

    /// A user-supplied name/id could inject CLI options or otherwise
    /// cannot be a Docker name or id prefix. Refused before the engine
    /// ever sees it.
    #[error("unsafe container name refused: {name:?}")]
    PoisonedName { name: String },

    /// Bounded capture overflowed: the engine had more to say than the
    /// retention limit. A partial listing is never presented as complete.
    #[error("bounded output exceeded: {what}")]
    OutputTooLarge { what: String },

    /// The engine answered but not in the shape its CLI contract
    /// promises (non-canonical id, malformed JSON, malformed tar).
    #[error("malformed engine response: {detail}")]
    Protocol { detail: String },

    /// The caller's cancellation token fired.
    #[error("operation cancelled")]
    Cancelled,

    /// Anything else the supervised subprocess reported: non-zero exit,
    /// deadline, pipe failure. The detail carries a bounded stderr tail.
    #[error("engine I/O: {detail}")]
    Io { detail: String },
}
