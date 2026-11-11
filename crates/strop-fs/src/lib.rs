//! Owned filesystem behavior over strop-workspace's pure resource contracts.
//! All functions performing native work run on jobs, never on input or
//! render. No frontend types or
//! background task is created merely by constructing a plan or inspecting data.
#[cfg(not(target_os = "linux"))]
compile_error!("strop-fs supports Linux only; the release target is x86_64-unknown-linux-musl");
pub mod batch;
mod context;
mod environment;
mod listing;
mod observation;
pub use context::{ExecutionContext, NamespaceView};
pub use environment::Environment;
#[cfg(test)]
mod adversarial;
mod attributes;
mod guard;
pub mod local;
pub mod notify;
mod stage;
#[cfg(test)]
mod tests;
mod trash;

pub use guard::reserved as is_protocol_path;
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
