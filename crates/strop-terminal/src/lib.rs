//! Owned local terminal emulation, transport and immutable text/cell projections.
//! Native execution belongs to workers; frontend state never owns child handles.
#[cfg(unix)]
pub mod client;
#[cfg(unix)]
pub mod helper;
pub mod launch;
pub mod model;
mod projection;
mod protocol;
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
