//! Validated remote address forms (0034 "Implementation boundaries" 2):
//! `ssh://[user@]host[:port]/absolute/path` as a pure, byte-native URI
//! domain, split by responsibility.
//!
//! [`RemoteEndpoint`] is the connection identity (user, host, port) every
//! pooled session, file and diagnostic names. [`RemoteFile`] is a canonical
//! absolute native path on one endpoint. [`RemoteLocation`] is what a user
//! actually types: it may still be an unresolved `~` home query, which only
//! a negotiated SFTP session may expand. Parsing is the only place remote
//! identity is textual; every field is admitted before any OpenSSH
//! subprocess exists, so no [`RemoteFile`] can carry an option-shaped
//! endpoint, a password, a query, a fragment or a control byte toward the
//! `ssh` argv. Paths decode to native filename bytes — Unix keeps arbitrary
//! non-NUL bytes, other platforms require strict UTF-8 rather than a lossy
//! stand-in. Identity is the decoded value: spellings that decode to the
//! same endpoint and path bytes are one value, while a decoded path is never
//! trimmed or normalized — `.`, `..` and `//` belong to the remote host.

mod endpoint;
mod error;
mod file;
mod location;
pub mod uri;

pub use endpoint::RemoteEndpoint;
pub use error::AddressError;
pub use file::RemoteFile;
pub use location::RemoteLocation;
