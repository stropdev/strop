//! Worker-side SFTP transport for the pooled remote sessions. The actor in
//! [`crate::pool`] owns one physical connection at a time; this module tree
//! supplies the wire codec (`wire`), the per-job protocol scripts
//! (`session`), the process lifecycle (`process`), bounded stderr
//! retention (`stderr`) and the typed failures (`error`). The only values
//! that escape are snapshots, listings, leases or typed errors — no async
//! type, pipe or process is ever visible to the editor, so input and render
//! never touch I/O.

pub(crate) mod error;

pub(crate) use error::Fault;
pub use error::{ReadFailureKind, ReadStage, RemoteReadError};

#[cfg(unix)]
pub(crate) mod process;
#[cfg(unix)]
pub(crate) mod session;
#[cfg(unix)]
mod stderr;
#[cfg(unix)]
mod wire;
