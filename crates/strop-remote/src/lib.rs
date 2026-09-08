//! Native-byte remote file identity and owned, read-only SSH/SFTP transport.
//!
//! Requests belong on workers. Authentication and encryption stay in system
//! OpenSSH; no editor, CLI, view or rendering state lives here.
mod address;
mod client;
mod exec;
mod hosts;
mod pool;
mod selection;
mod ssh;
#[cfg(test)]
mod test_support;
mod transport;

pub use address::{AddressError, RemoteEndpoint, RemoteFile, RemoteLocation};
pub use client::{
    ConnectionLease, RemoteClient, RemoteDirectorySnapshot, RemoteEntry, RemoteEntryKind,
    RemoteResource, RemoteSnapshot,
};
pub use exec::{
    command, command_supervised, run, CommandOutput, RemoteCommand, RemoteCommandError,
    RemoteExitStatus, StdinMode, SupervisionKey, SupervisionOutcome,
};
pub use hosts::{enumerate_hosts, CandidateOrigin, HostCandidate, HostEnumeration, HostSources};
pub use selection::{
    ReadLimit, ReadLimitError, ReadSelection, RemoteOffset, RemoteSize, RemoteWindow,
};
pub use transport::{ReadFailureKind, ReadStage, RemoteReadError};
