//! Remote files and commands over plain SSH (plan 0090 §5): one
//! multiplexed OpenSSH connection per endpoint, SFTP for listing, reading
//! and saving, and exec for `rg`, `git`, language servers and terminals.
//!
//! Requests belong on jobs. Authentication and encryption stay in system
//! OpenSSH; no editor, CLI, view or rendering state lives here. Identity types
//! (endpoint, file, location, address errors) live in `strop-workspace` (0042);
//! this crate is the transport and behavior over them.
mod client;
#[cfg(unix)]
pub mod exec;
mod hosts;
mod pool;
mod ssh;
#[cfg(test)]
mod test_support;
mod transport;
pub use client::{
    ConnectionLease, PermissionBitsError, RemoteClient, RemoteDirectorySnapshot, RemoteEntry,
    RemoteEntryKind, RemotePermissions, RemoteResource, RemoteSnapshot,
};
pub use hosts::{enumerate_hosts, CandidateOrigin, HostCandidate, HostEnumeration, HostSources};
pub use pool::MAX_TEXT;
pub use transport::{Expect, RemoteError, RemoteFailureKind, RemoteStage, Stamp};
