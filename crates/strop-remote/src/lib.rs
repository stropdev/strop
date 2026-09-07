//! Native-byte remote file identity and owned, read-only SSH/SFTP transport.
//!
//! `read` belongs on a worker: it accepts that worker's cancellation token and
//! returns an in-memory buffer or typed failure. Authentication and encryption
//! stay in system OpenSSH; no editor, CLI, view or rendering state lives here.
mod address;
mod transport;

pub use address::{AddressError, RemoteFile};
pub use transport::{read, ReadFailureKind, ReadStage, RemoteReadError};
