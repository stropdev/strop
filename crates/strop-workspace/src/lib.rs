//! Shared workspace and resource identity (0042): the pure contracts every
//! consumer — editor, remote transport, LSP, Git, picker — routes by.
//!
//! This crate owns *identity*, never behavior: no I/O, no subprocess, no
//! filesystem probe. The transport (`strop-remote`), language services and
//! Git implement behavior against these types; nothing here depends on them.
//!
//! - [`addr`]: validated remote address forms — endpoint, canonical file,
//!   unresolved user location, and the strict percent-codec.
//! - [`Filesystem`]: the namespace a path names (local disk or one endpoint).
//! - [`ResourceLocation`]: one resolved path on one filesystem.

pub mod addr;
mod container;
pub mod directory;
mod filesystem;
pub mod operation;
mod resource;

pub use addr::{AddressError, RemoteEndpoint, RemoteFile, RemoteLocation};
pub use container::{ContainerId, ContainerIdError};
pub use directory::{
    DirectoryEntry, DirectorySnapshot, EntryKind, EntryName, EntryNameError, FileTime,
    ListingState, ObjectId, Observation, PermissionBitsError, Permissions,
};
pub use filesystem::Filesystem;
pub use resource::ResourceLocation;
