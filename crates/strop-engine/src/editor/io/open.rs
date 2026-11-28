//! Resource inspection and opening on a job: the local filesystem in
//! process, SSH over the endpoint's SFTP session. Directories never
//! enter a file reader.
use super::{Document, FileTarget, Opened};
use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};
use strop_core::Buffer;
use strop_workspace::operation::FsFailureKind;
use strop_workspace::ResourceLocation;

pub(super) struct OpenRead {
    pub target: FileTarget,
    pub browse: bool,
    pub requires_file: bool,
    pub client: strop_remote::RemoteClient,
    pub local: crate::editor::namespace::LocalFs,
    pub previous_directories: Vec<super::super::Directory>,
    pub reveal: Option<ResourceLocation>,
    /// Names the folder's filename draft when the target is a folder.
    pub request: strop_core::worker::WorkerId,
}
impl OpenRead {
    pub fn run(self, cancel: &CancelToken) -> Outcome<Opened> {
        if cancel.is_cancelled() {
            return Outcome::Cancelled(CancelReason::OwnerClosed);
        }
        match &self.target {
            FileTarget::Local(path) => {
                let observation = match observe_local(&self.local, path, cancel) {
                    Ok(observation) => observation,
                    Err(error) => return observe_failed(error),
                };
                match observation {
                    Some(observation)
                        if observation.kind == strop_workspace::EntryKind::Directory =>
                    {
                        if self.requires_file {
                            return file_required();
                        }
                        self.list(ResourceLocation::local(path.clone()), cancel)
                    }
                    Some(observation)
                        if self.browse && observation.kind == strop_workspace::EntryKind::File =>
                    {
                        Outcome::failed(FailureKind::InvalidInput, "browse requires a directory")
                    }
                    observation => open_local_file(&self.local, path, observation, cancel),
                }
            }
            FileTarget::Remote(location) => {
                if self.browse {
                    // Home-relative locations resolve through the
                    // session's own expansion below.
                    if let Some(file) = location.absolute_file() {
                        let resource = ResourceLocation::remote(
                            location.endpoint().clone(),
                            file.path().to_path_buf(),
                        );
                        return self.list(resource, cancel);
                    }
                }
                let result = if self.browse {
                    self.client
                        .list(location, cancel)
                        .map(strop_remote::RemoteResource::Directory)
                } else {
                    self.client.open(location, cancel)
                };
                match result {
                    Ok(strop_remote::RemoteResource::File(snapshot)) => {
                        let canonical = FileTarget::Remote(snapshot.file.clone().into());
                        Outcome::Success(Opened {
                            document: Document::remote_snapshot(*snapshot),
                            canonical,
                        })
                    }
                    Ok(strop_remote::RemoteResource::Directory(snapshot)) => {
                        if self.requires_file {
                            return file_required();
                        }
                        match super::super::namespace::from_remote(snapshot) {
                            Ok(listed) => directory_opened(
                                listed,
                                &self.previous_directories,
                                self.reveal.as_ref(),
                                self.request,
                                cancel,
                            ),
                            Err(error) => Outcome::failed(FailureKind::Protocol, error.to_string()),
                        }
                    }
                    Err(error) if error.is_cancellation() => {
                        Outcome::Cancelled(CancelReason::OwnerClosed)
                    }
                    Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                }
            }
        }
    }
    fn list(&self, location: ResourceLocation, cancel: &CancelToken) -> Outcome<Opened> {
        match super::super::namespace::list(&self.local, &location, &self.client, cancel) {
            Ok(listed) => directory_opened(
                listed,
                &self.previous_directories,
                self.reveal.as_ref(),
                self.request,
                cancel,
            ),
            Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
        }
    }
}
/// The startup file (plan 0108 H6): the guarded local read `:e` runs —
/// devices, FIFOs and sockets refused, a missing file a new buffer —
/// called before the event loop exists. A refusal is the message `:e`
/// would show.
pub fn open_startup_file(path: std::path::PathBuf) -> Result<Buffer, String> {
    let local = crate::editor::namespace::LocalFs::default();
    let (cancel, _handle) = CancelToken::standalone();
    let outcome = match observe_local(&local, &path, &cancel) {
        Ok(Some(observation)) if observation.kind == strop_workspace::EntryKind::Directory => {
            file_required()
        }
        Ok(observation) => open_local_file(&local, &path, observation, &cancel),
        Err(error) => observe_failed(error),
    };
    match outcome {
        Outcome::Success(opened) => Ok(opened.document.buf),
        Outcome::Failed { failure, .. } => {
            Err(format!("open {}: {}", path.display(), failure.message))
        }
        Outcome::Cancelled(_) => Err(format!("open {}: cancelled", path.display())),
    }
}

/// One local path's observation (`None`: nothing is there).
fn observe_local(
    local: &crate::editor::namespace::LocalFs,
    path: &std::path::Path,
    cancel: &CancelToken,
) -> Result<Option<strop_workspace::Observation>, strop_workspace::operation::FsFailure> {
    let location = ResourceLocation::local(path.to_path_buf());
    let mut observations = local.observe(cancel, std::slice::from_ref(&location))?;
    let observed = observations.pop().ok_or_else(|| {
        strop_workspace::operation::FsFailure::new(
            FsFailureKind::Protocol,
            "an observation batch came back empty",
        )
    })?;
    Ok(observed.value)
}

/// The open an observation failure ends in.
fn observe_failed(error: strop_workspace::operation::FsFailure) -> Outcome<Opened> {
    if error.kind == FsFailureKind::Cancelled {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    Outcome::failed(FailureKind::Io, error.to_string())
}

/// A local path that is not a directory: a missing one is a new empty
/// buffer with that path (vim semantics — `:w` creates it); a regular
/// file is read whole; anything else is refused.
fn open_local_file(
    local: &crate::editor::namespace::LocalFs,
    path: &std::path::PathBuf,
    observation: Option<strop_workspace::Observation>,
    cancel: &CancelToken,
) -> Outcome<Opened> {
    let Some(observation) = observation else {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        return Outcome::Success(Opened {
            document: Document::new(Buffer::from_read(
                path.clone(),
                ropey::Rope::new(),
                None,
                canonical.clone(),
                true,
            )),
            canonical: FileTarget::Local(canonical),
        });
    };
    match observation.kind {
        strop_workspace::EntryKind::File => {
            let location = ResourceLocation::local(path.clone());
            let rope = match local.read_rope(cancel, &location) {
                Ok(rope) => rope,
                Err(error) if error.kind == FsFailureKind::Cancelled => {
                    return Outcome::Cancelled(CancelReason::OwnerClosed)
                }
                Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
            };
            // Writability is the observation's evidence, never guessed
            // (readonly = no write bit). The identity path is this
            // namespace's own canonicalization.
            let writable = observation
                .permissions
                .is_none_or(|permissions| permissions.bits() & 0o222 != 0);
            let stamp = observation.modified.map(filetime_to_systemtime);
            let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            Outcome::Success(Opened {
                document: Document::new(Buffer::from_read(
                    path.clone(),
                    rope,
                    stamp,
                    canonical.clone(),
                    writable,
                )),
                canonical: FileTarget::Local(canonical),
            })
        }
        strop_workspace::EntryKind::SymbolicLink => Outcome::failed(
            FailureKind::InvalidInput,
            "symlink target is unavailable; no new-file fallback",
        ),
        _ => Outcome::failed(
            FailureKind::InvalidInput,
            "only regular files and directories can be opened",
        ),
    }
}

fn file_required() -> Outcome<Opened> {
    Outcome::failed(
        FailureKind::InvalidInput,
        "this operation requires a regular file",
    )
}
fn directory_opened(
    listed: super::super::namespace::Listed,
    previous: &[super::super::Directory],
    reveal: Option<&ResourceLocation>,
    request: strop_core::worker::WorkerId,
    cancel: &CancelToken,
) -> Outcome<Opened> {
    let canonical = match FileTarget::from_location(&listed.directory.snapshot.location) {
        Ok(canonical) => canonical,
        Err(error) => return Outcome::failed(FailureKind::Protocol, error.to_string()),
    };
    let mut source = super::super::Directory::from_listing(listed.directory, listed.connection);
    let all_rows = source.visible.clone();
    if let Some(old) = previous.iter().find(|old| old.location == source.location) {
        source.filter = old.filter.clone();
        source.return_to = old.return_to.clone();
        source.show_hidden = old.show_hidden;
        if !source.filter.is_empty() || !source.show_hidden {
            if let Err(error) = super::super::directory::apply_filter(&mut source, cancel) {
                return if cancel.is_cancelled() {
                    Outcome::Cancelled(CancelReason::OwnerClosed)
                } else {
                    Outcome::failed(FailureKind::InvalidInput, error)
                };
            }
        }
    }
    if reveal.is_some_and(|reveal| {
        source.entry_index(reveal).is_some() && source.line_for(reveal).is_none()
    }) {
        source.visible = all_rows;
        source.filter.clear();
        source.show_hidden = true;
        source.measure();
    }
    match source.into_document(request) {
        Ok(document) => Outcome::Success(Opened {
            document,
            canonical,
        }),
        Err(error) => Outcome::failed(FailureKind::InvalidInput, error),
    }
}

/// Exact mtime evidence conversion: the kernel's FileTime and std's
/// SystemTime must compare equal for the same instant, since the
/// save-time "changed on disk" check compares against
/// `fs::metadata().modified()`.
pub(crate) fn filetime_to_systemtime(stamp: strop_workspace::FileTime) -> std::time::SystemTime {
    let epoch = std::time::UNIX_EPOCH;
    if stamp.seconds >= 0 {
        epoch + std::time::Duration::new(stamp.seconds as u64, stamp.nanos)
    } else {
        (epoch - std::time::Duration::new(stamp.seconds.unsigned_abs(), 0))
            + std::time::Duration::new(0, stamp.nanos)
    }
}

/// The exact inverse of [`filetime_to_systemtime`]: the editor's mtime
/// baseline crosses into the Store intent as kernel evidence.
/// Nanoseconds stay a positive offset from the seconds floor, matching
/// the kernel's timespec encoding.
pub(crate) fn systemtime_to_filetime(stamp: std::time::SystemTime) -> strop_workspace::FileTime {
    match stamp.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => strop_workspace::FileTime {
            seconds: duration.as_secs() as i64,
            nanos: duration.subsec_nanos(),
        },
        Err(error) => {
            let duration = error.duration();
            let (seconds, nanos) = if duration.subsec_nanos() == 0 {
                (-(duration.as_secs() as i64), 0)
            } else {
                (
                    -(duration.as_secs() as i64) - 1,
                    1_000_000_000 - duration.subsec_nanos(),
                )
            };
            strop_workspace::FileTime { seconds, nanos }
        }
    }
}
