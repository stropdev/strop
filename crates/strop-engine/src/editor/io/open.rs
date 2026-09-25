//! Worker-only resource inspection and opening. Directories never enter a file reader.
use super::{Document, FileTarget, Opened};
use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};
use strop_core::Buffer;
use strop_workspace::{Filesystem, ResourceLocation};

pub(super) struct OpenRead {
    pub target: FileTarget,
    pub browse: bool,
    pub requires_file: bool,
    pub selection: strop_remote::ReadSelection,
    pub client: strop_remote::RemoteClient,
    pub container: Option<strop_containers::ContainerIdentity>,
    /// The session's local worker lease (0058 WK04): local reads,
    /// observations and listings ride it — no in-process twin.
    pub worker: strop_worker_client::Worker,
    /// The session's remote worker leases (0058 WK07): SSH workspaces
    /// whose host admitted a worker read/list/observe through it; every
    /// other host keeps the read-only SFTP path byte-identically.
    pub remote: crate::editor::remote::workers::RemoteWorkers,
    pub previous_directories: Vec<super::super::Directory>,
    pub reveal: Option<ResourceLocation>,
}
impl OpenRead {
    pub fn run(self, cancel: &CancelToken) -> Outcome<Opened> {
        if cancel.is_cancelled() {
            return Outcome::Cancelled(CancelReason::OwnerClosed);
        }
        match &self.target {
            FileTarget::Local(path) => {
                // WK04: observation, listing and content all ride the
                // local worker lease; the editor keeps no in-process
                // filesystem path for user resources.
                let location = ResourceLocation::local(path.clone());
                let observed = match self
                    .worker
                    .observe(cancel, vec![location.clone()])
                    .map(|mut observations| observations.pop())
                {
                    Ok(Some(observation)) => observation,
                    Ok(None) => {
                        return Outcome::failed(
                            FailureKind::Protocol,
                            "worker answered an empty observation batch",
                        )
                    }
                    Err(error) if error.is_cancellation() => {
                        return Outcome::Cancelled(CancelReason::OwnerClosed)
                    }
                    Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
                };
                let Some(observation) = observed.value else {
                    // Missing file: a new empty buffer with that path (vim
                    // semantics — `:w` creates it), matching Buffer::open.
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
                    strop_workspace::EntryKind::Directory => {
                        if self.requires_file {
                            return file_required();
                        }
                        self.list(location, cancel)
                    }
                    strop_workspace::EntryKind::File => {
                        if self.browse {
                            return Outcome::failed(
                                FailureKind::InvalidInput,
                                "browse requires a directory",
                            );
                        }
                        let payload = match self.worker.read(cancel, location, 0, None) {
                            Ok(payload) => payload,
                            Err(error) if error.is_cancellation() => {
                                return Outcome::Cancelled(CancelReason::OwnerClosed)
                            }
                            Err(error) => {
                                return Outcome::failed(FailureKind::Io, error.to_string())
                            }
                        };
                        let rope = match ropey::Rope::from_reader(payload) {
                            Ok(rope) => rope,
                            Err(error) => {
                                return Outcome::failed(FailureKind::Io, error.to_string())
                            }
                        };
                        // Writability is the observation's evidence,
                        // never guessed (readonly = no write bit, the
                        // exact Buffer::open rule). The identity path is
                        // this namespace's own canonicalization.
                        let writable = observation
                            .permissions
                            .is_none_or(|permissions| permissions.bits() & 0o222 != 0);
                        let stamp = observation.modified.map(filetime_to_systemtime);
                        let canonical =
                            std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
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
            FileTarget::Remote(location) => {
                if self.browse {
                    // Absolute directories route worker-or-SFTP inside
                    // namespace::list; home-relative locations resolve
                    // through the SFTP path as before.
                    if let Some(file) = location.absolute_file() {
                        let resource = ResourceLocation::remote(
                            location.endpoint().clone(),
                            file.path().to_path_buf(),
                        );
                        return self.list(resource, cancel);
                    }
                }
                // WK07: an admitted endpoint with a live lease reads
                // through its worker; anything else keeps the SFTP path
                // (home-relative locations resolve only there).
                if let Some(file) = location.absolute_file() {
                    if let Some(worker) = self
                        .remote
                        .get(location.endpoint())
                        .filter(|worker| worker.worker().session().is_some())
                    {
                        return self.open_remote_worker(&worker, file, cancel);
                    }
                }
                let result = if self.browse {
                    self.client
                        .list(location, cancel)
                        .map(strop_remote::RemoteResource::Directory)
                } else {
                    self.client.open(location, self.selection, cancel)
                };
                match result {
                    Ok(strop_remote::RemoteResource::File(snapshot)) => {
                        let canonical = FileTarget::Remote(snapshot.file.clone().into());
                        Outcome::Success(Opened {
                            document: Document::remote_snapshot(*snapshot, self.selection),
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
            FileTarget::Container { container, path } => {
                let Some(identity) = self
                    .container
                    .as_ref()
                    .filter(|identity| identity.id == container.as_str())
                else {
                    return Outcome::failed(
                        FailureKind::InvalidInput,
                        "attach the container before opening its resources",
                    );
                };
                let Some(path_text) = path.to_str() else {
                    return Outcome::failed(
                        FailureKind::InvalidInput,
                        "container backend requires a UTF-8 path",
                    );
                };
                let engine = match strop_containers::engine(cancel) {
                    Ok(engine) => engine,
                    Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
                };
                let reference = match strop_containers::ContainerRef::of(identity) {
                    Ok(reference) => reference,
                    Err(error) => return Outcome::failed(FailureKind::Protocol, error.to_string()),
                };
                // The streaming directory adapter stops at a non-directory header;
                // this does not retain a whole directory archive to discover its kind.
                match strop_containers::list_dir(&engine, &reference, path_text, cancel) {
                    Ok(entries) => {
                        if self.requires_file {
                            return file_required();
                        }
                        let location = ResourceLocation {
                            filesystem: Filesystem::Container(container.clone()),
                            path: path.clone(),
                        };
                        match super::super::namespace::from_container(location, entries) {
                            Ok(listed) => directory_opened(
                                listed,
                                &self.previous_directories,
                                self.reveal.as_ref(),
                                cancel,
                            ),
                            Err(error) => Outcome::failed(FailureKind::Protocol, error.to_string()),
                        }
                    }
                    Err(strop_containers::ContainerError::CapabilityRefused { .. })
                        if !self.browse =>
                    {
                        const LIMIT: u64 = 4 * 1024 * 1024;
                        match strop_containers::read_file(
                            &engine,
                            &reference,
                            path_text,
                            LIMIT + 1,
                            cancel,
                        ) {
                            Ok(bytes) if bytes.len() as u64 > LIMIT => Outcome::failed(
                                FailureKind::InvalidInput,
                                "container file exceeds the 4 MiB view limit",
                            ),
                            Ok(bytes) => match String::from_utf8(bytes) {
                                Ok(text) => Outcome::Success(Opened {
                                    document: Document::container_file(
                                        Buffer::from_text(&text),
                                        container.clone(),
                                        path.clone(),
                                    ),
                                    canonical: self.target,
                                }),
                                Err(_) => Outcome::failed(
                                    FailureKind::InvalidInput,
                                    "container file is not UTF-8 text",
                                ),
                            },
                            Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                        }
                    }
                    Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                }
            }
        }
    }
    /// Open one absolute remote file through the endpoint's admitted
    /// worker: observe, read the selection window and build the same
    /// remote document shape the SFTP path builds (read-only buffer;
    /// editability stays with the permit flow).
    fn open_remote_worker(
        &self,
        worker: &strop_remote::worker_transport::RemoteWorker,
        file: &strop_workspace::RemoteFile,
        cancel: &CancelToken,
    ) -> Outcome<Opened> {
        use strop_remote::worker_transport::RemoteReadFailure;
        let location = ResourceLocation::remote(file.endpoint().clone(), file.path().to_path_buf());
        let result = worker.read_selection(cancel, location, &self.selection);
        let (text, window) = match result {
            Ok(loaded) => loaded,
            Err(RemoteReadFailure::Client(error)) if error.is_cancellation() => {
                return Outcome::Cancelled(CancelReason::OwnerClosed)
            }
            Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
        };
        let mut buffer = Buffer::from_text(&text);
        // A remote snapshot is never writable; provenance beyond this
        // flag is the caller's document identity.
        buffer.readonly = true;
        let canonical = FileTarget::Remote(file.clone().into());
        Outcome::Success(Opened {
            document: Document::remote(
                buffer,
                super::super::document::RemoteDocument {
                    file: file.clone(),
                    window,
                    selection: self.selection,
                    connection: None,
                    return_to: None,
                    write: None,
                },
            ),
            canonical,
        })
    }

    fn list(&self, location: ResourceLocation, cancel: &CancelToken) -> Outcome<Opened> {
        match super::super::namespace::list(
            &self.worker,
            &self.remote,
            &location,
            &self.client,
            self.container.as_ref(),
            cancel,
        ) {
            Ok(listed) => directory_opened(
                listed,
                &self.previous_directories,
                self.reveal.as_ref(),
                cancel,
            ),
            Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
        }
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
        for (name, before) in &old.marked {
            let location = ResourceLocation {
                filesystem: source.location.filesystem.clone(),
                path: source.location.path.join(name.as_path()),
            };
            if let Some(entry) = source
                .line_for(&location)
                .and_then(|line| source.entry(line))
            {
                if before.same_object(&entry.observation) || before == &entry.observation {
                    source
                        .marked
                        .insert(name.clone(), entry.observation.clone());
                }
            }
        }
        if !source.filter.is_empty() {
            if let Err(error) = super::super::directory::apply_filter(&mut source, cancel) {
                return if cancel.is_cancelled() {
                    Outcome::Cancelled(CancelReason::OwnerClosed)
                } else {
                    Outcome::failed(FailureKind::InvalidInput, error)
                };
            }
        }
        let lost = old.marked.len().saturating_sub(source.marked.len());
        if lost > 0 {
            source
                .summary
                .push_str(&format!(" · {lost} marks not retained"));
        }
    }
    if let Some(reveal) = reveal
        .filter(|reveal| source.entry_index(reveal).is_some() && source.line_for(reveal).is_none())
    {
        source.visible = all_rows;
        source.filter.clear();
        source.summary = format!("folder · filter cleared to reveal {}", reveal.label());
    }
    let buffer = Buffer::from_text(&source.text());
    Outcome::Success(Opened {
        document: Document::directory(buffer, source),
        canonical,
    })
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
