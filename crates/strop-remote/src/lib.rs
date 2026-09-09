//! Native-byte remote identity, read-only SFTP and explicit cooperative SSH saves.
//!
//! Requests belong on workers. Authentication and encryption stay in system
//! OpenSSH; no editor, CLI, view or rendering state lives here.
mod address;
mod client;
mod exec;
mod hosts;
mod pool;
pub mod save;
mod selection;
mod ssh;
#[cfg(test)]
mod test_support;
mod transport;

pub use address::{AddressError, RemoteEndpoint, RemoteFile, RemoteLocation};
pub use client::{
    ConnectionLease, PermissionBitsError, RemoteClient, RemoteDirectorySnapshot, RemoteEntry,
    RemoteEntryKind, RemotePermissions, RemoteResource, RemoteSnapshot,
};
pub use exec::{
    command, command_supervised, run, run_with_input, CommandOutput, RemoteCommand,
    RemoteCommandError, RemoteExitStatus, RemoteProgram, StdinMode, SupervisionKey,
    SupervisionOutcome,
};
pub use hosts::{enumerate_hosts, CandidateOrigin, HostCandidate, HostEnumeration, HostSources};
pub use selection::{
    ReadLimit, ReadLimitError, ReadSelection, RemoteOffset, RemoteSize, RemoteWindow,
};
pub use transport::{ReadFailureKind, ReadStage, RemoteReadError};
