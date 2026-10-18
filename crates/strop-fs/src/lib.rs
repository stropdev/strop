//! Owned filesystem behavior over strop-workspace's pure resource contracts.
//! All functions performing native work are worker-only. No frontend types or
//! background task is created merely by constructing a plan or inspecting data.
pub mod batch;
mod context;
mod environment;
mod listing;
mod observation;
pub use context::{ExecutionContext, NamespaceView};
pub use environment::Environment;
#[cfg(all(test, target_os = "linux"))]
mod adversarial;
#[cfg(target_os = "linux")]
mod attributes;
#[cfg(target_os = "linux")]
mod guard;
#[cfg(target_os = "linux")]
pub mod local;
#[cfg(not(target_os = "linux"))]
#[path = "unsupported.rs"]
pub mod local;
#[cfg(target_os = "linux")]
mod stage;
#[cfg(test)]
mod tests;
#[cfg(target_os = "linux")]
mod trash;

pub use listing::{assemble, list, parent, ListedDirectory, ObservedEntry};
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
