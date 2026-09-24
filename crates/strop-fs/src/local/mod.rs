//! Native checked preparation, publication, copy and read-only reconciliation.
mod copy;
mod execute;
mod outcome;
mod prepare;
mod verify;
use crate::guard::{self, NameLock, Parent};
use crate::{failure, io_failure, observation, stage};
pub use execute::execute;
use outcome::{committed, uncertain};
pub use prepare::prepare;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use strop_core::worker::CancelToken;
use strop_workspace::operation::*;
use strop_workspace::{EntryKind, Observation, ResourceLocation};
pub use verify::verify;
pub(super) fn rename_error(error: rustix::io::Errno) -> FsFailure {
    if error == rustix::io::Errno::XDEV {
        return failure(
            FsFailureKind::Unsupported,
            "cross-filesystem move refused; choose Copy explicitly",
        );
    }
    if matches!(
        error,
        rustix::io::Errno::NOSYS | rustix::io::Errno::INVAL | rustix::io::Errno::OPNOTSUPP
    ) {
        return failure(
            FsFailureKind::Unsupported,
            format!("native no-replace rename is unavailable: {error}"),
        );
    }
    io_failure(error.into())
}
pub(super) fn witness(
    file: &File,
    content: Option<[u8; 32]>,
) -> Result<PublicationWitness, FsFailure> {
    let metadata = file.metadata().map_err(io_failure)?;
    Ok(PublicationWitness {
        identity: strop_workspace::ObjectId {
            device: metadata.dev(),
            inode: metadata.ino(),
        },
        changed: observation::metadata(&metadata).changed,
        content,
    })
}
