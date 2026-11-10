//! Owned terminal emulation, transport and immutable text/cell projections.
//! A local session's PTY runs in this process (0090 §4); a remote one
//! belongs to the namespace's admitted worker (0058 WK12). Frontend state
//! never owns child handles — it enqueues intents on a service and takes
//! immutable updates; emulation stays here.
#[cfg(unix)]
pub mod client;
pub mod launch;
#[cfg(unix)]
mod local;
pub mod model;
mod projection;
#[cfg(unix)]
pub mod service;
#[cfg(unix)]
mod transport;
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
    #[error("terminal session is no longer running")]
    Closed,
}
