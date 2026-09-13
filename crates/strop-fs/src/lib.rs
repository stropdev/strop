//! Owned filesystem behavior over strop-workspace's pure resource contracts.
//! All functions performing native work are worker-only. No frontend types or
//! background task is created merely by constructing a plan or inspecting data.
pub mod batch;
mod environment;
mod listing;
mod observation;
pub use environment::Environment;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod attributes;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod guard;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod local;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[path = "unsupported.rs"]
pub mod local;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod stage;
#[cfg(test)]
mod tests;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod trash;

pub use listing::{from_container, from_remote, list, parent, ListedDirectory};
pub use observation::observe;
use strop_workspace::operation::{FsFailure, FsFailureKind};

pub(crate) fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}
pub(crate) fn io_failure(error: std::io::Error) -> FsFailure {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied => FsFailureKind::Permission,
        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotFound => FsFailureKind::Conflict,
        std::io::ErrorKind::InvalidInput => FsFailureKind::InvalidPath,
        std::io::ErrorKind::DirectoryNotEmpty => FsFailureKind::Unsupported,
        _ => FsFailureKind::Io,
    };
    failure(kind, error.to_string())
}
