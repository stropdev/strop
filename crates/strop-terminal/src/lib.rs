//! Owned terminal emulation, transport and immutable text/cell projections.
//! PTY execution belongs to the namespace's admitted worker (0058 WK12);
//! frontend state never owns child handles — spawn/feed/resize/terminate
//! ride the worker client and emulation stays here.
#[cfg(unix)]
pub mod client;
pub mod launch;
pub mod model;
mod projection;
#[cfg(unix)]
pub mod service;
mod vt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("terminal {operation} failed: {detail}")]
    Io {
        operation: &'static str,
        detail: String,
    },
    #[error("terminal native {operation} failed ({code})")]
    Native { operation: &'static str, code: i32 },
    #[error("terminal state failed: {0}")]
    State(&'static str),
    #[error("terminal capability unavailable: {0}")]
    Unavailable(String),
    #[error("terminal capacity exceeded: {0}")]
    Capacity(&'static str),
    #[error("terminal protocol refused: {0}")]
    Protocol(String),
    #[error("terminal input queue is full; input was not admitted")]
    InputFull,
    #[error("terminal paste contains control or multiline input; confirm before sending")]
    PasteNeedsConfirmation,
    #[error("terminal session is no longer running")]
    Closed,
}
