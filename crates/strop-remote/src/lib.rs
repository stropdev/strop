//! Native-byte remote identity, read-only SFTP and explicit cooperative SSH saves.
//!
//! Requests belong on workers. Authentication and encryption stay in system
//! OpenSSH; no editor, CLI, view or rendering state lives here. Identity types
//! (endpoint, file, location, address errors) live in `strop-workspace` (0042);
//! this crate is the transport and behavior over them.
pub mod bootstrap;
mod client;
pub mod deploy_provider;
mod hosts;
mod pool;
mod selection;
mod ssh;
#[cfg(test)]
mod test_support;
mod transport;
pub mod worker_transport;
pub use client::{
    ConnectionLease, PermissionBitsError, RemoteClient, RemoteDirectorySnapshot, RemoteEntry,
    RemoteEntryKind, RemotePermissions, RemoteResource, RemoteSnapshot,
};
pub use hosts::{enumerate_hosts, CandidateOrigin, HostCandidate, HostEnumeration, HostSources};
pub use selection::{
    ReadLimit, ReadLimitError, ReadSelection, RemoteOffset, RemoteSize, RemoteWindow,
};
pub use transport::{ReadFailureKind, ReadStage, RemoteReadError};
