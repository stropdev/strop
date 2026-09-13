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
                let metadata = match std::fs::metadata(path) {
                    Ok(metadata) => Some(metadata),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if std::fs::symlink_metadata(path)
                            .is_ok_and(|metadata| metadata.file_type().is_symlink())
                        {
                            return Outcome::failed(
                                FailureKind::InvalidInput,
                                "symlink target is unavailable; no new-file fallback",
                            );
                        }
                        None
                    }
                    Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
                };
                if metadata.as_ref().is_some_and(|metadata| metadata.is_dir()) {
                    if self.requires_file {
                        return file_required();
                    }
                    return self.list(ResourceLocation::local(path.clone()), cancel);
                }
                if self.browse {
                    return Outcome::failed(
                        FailureKind::InvalidInput,
                        "browse requires a directory",
                    );
                }
                if metadata
                    .as_ref()
                    .is_some_and(|metadata| !metadata.is_file())
                {
                    return Outcome::failed(
                        FailureKind::InvalidInput,
                        "only regular files and directories can be opened",
                    );
                }
                match Buffer::open(path) {
                    Ok(buffer) => {
                        let canonical = buffer
                            .file_identity()
                            .map_or_else(|| path.clone(), ToOwned::to_owned);
                        Outcome::Success(Opened {
                            document: Document::new(buffer),
                            canonical: FileTarget::Local(canonical),
                        })
                    }
                    Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                }
            }
            FileTarget::Remote(location) => {
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
                        match strop_fs::from_remote(snapshot) {
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
                        match strop_fs::from_container(location, entries) {
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
    fn list(&self, location: ResourceLocation, cancel: &CancelToken) -> Outcome<Opened> {
        match strop_fs::list(&location, &self.client, self.container.as_ref(), cancel) {
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
    listed: strop_fs::ListedDirectory,
    previous: &[super::super::Directory],
    reveal: Option<&ResourceLocation>,
    cancel: &CancelToken,
) -> Outcome<Opened> {
    let canonical = match FileTarget::from_location(&listed.snapshot.location) {
        Ok(canonical) => canonical,
        Err(error) => return Outcome::failed(FailureKind::Protocol, error.to_string()),
    };
    let mut source = super::super::Directory::from_listing(listed);
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
