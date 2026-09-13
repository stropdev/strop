//! Worker-only listing adapters. Only native entry names become child locations.
use crate::{failure, io_failure};
use std::path::Path;
use strop_core::worker::CancelToken;
use strop_workspace::operation::{FsFailure, FsFailureKind};
use strop_workspace::{
    DirectoryEntry, DirectorySnapshot, EntryKind, EntryName, Filesystem, ListingState, Observation,
    Permissions, ResourceLocation,
};

const ENTRY_LIMIT: usize = 100_000;
const NAME_BYTES_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ListedDirectory {
    pub snapshot: DirectorySnapshot,
    #[serde(skip)]
    pub connection: Option<strop_remote::ConnectionLease>,
}

pub fn list(
    location: &ResourceLocation,
    remote: &strop_remote::RemoteClient,
    container: Option<&strop_containers::ContainerIdentity>,
    token: &CancelToken,
) -> Result<ListedDirectory, FsFailure> {
    if !location.path.is_absolute() {
        return Err(failure(
            FsFailureKind::InvalidPath,
            "directory scope must be absolute",
        ));
    }
    match &location.filesystem {
        Filesystem::Local => list_local(location, token),
        Filesystem::Remote(endpoint) => {
            let file =
                strop_workspace::RemoteFile::from_path(endpoint.clone(), location.path.clone())
                    .map_err(|error| failure(FsFailureKind::InvalidPath, error.to_string()))?;
            let listed = remote.list(&file.into(), token).map_err(|error| {
                failure(
                    if error.is_cancellation() {
                        FsFailureKind::Cancelled
                    } else {
                        FsFailureKind::Io
                    },
                    error.to_string(),
                )
            })?;
            from_remote(listed)
        }
        Filesystem::Container(id) => {
            let identity = container
                .filter(|identity| identity.id == id.as_str())
                .ok_or_else(|| {
                    failure(
                        FsFailureKind::Unsupported,
                        "attach the container before browsing its filesystem",
                    )
                })?;
            let path = location.path.to_str().ok_or_else(|| {
                failure(
                    FsFailureKind::Unsupported,
                    "container backend requires a UTF-8 path",
                )
            })?;
            let engine = strop_containers::engine(token)
                .map_err(|error| failure(FsFailureKind::Io, error.to_string()))?;
            let reference = strop_containers::ContainerRef::of(identity)
                .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?;
            let entries = strop_containers::list_dir(&engine, &reference, path, token)
                .map_err(|error| failure(FsFailureKind::Io, error.to_string()))?;
            from_container(location.clone(), entries)
        }
    }
}

pub fn from_remote(
    listed: strop_remote::RemoteDirectorySnapshot,
) -> Result<ListedDirectory, FsFailure> {
    let location = ResourceLocation::remote(
        listed.directory.endpoint().clone(),
        listed.directory.path().to_path_buf(),
    );
    let mut entries = Vec::with_capacity(listed.entries.len().min(ENTRY_LIMIT));
    let mut state = ListingState::Complete;
    let mut bytes = 0;
    for entry in listed.entries {
        if entry.file.endpoint() != listed.directory.endpoint()
            || entry.file.path().parent() != Some(listed.directory.path())
        {
            return Err(failure(
                FsFailureKind::Protocol,
                "remote directory child escaped its captured parent",
            ));
        }
        let name =
            entry.file.path().file_name().ok_or_else(|| {
                failure(FsFailureKind::Protocol, "remote child has no native name")
            })?;
        bytes += name.as_encoded_bytes().len();
        if entries.len() == ENTRY_LIMIT || bytes > NAME_BYTES_LIMIT {
            state = ListingState::Limited {
                limit: entries.len(),
            };
            break;
        }
        let kind = match entry.kind {
            strop_remote::RemoteEntryKind::File => EntryKind::File,
            strop_remote::RemoteEntryKind::Directory => EntryKind::Directory,
            strop_remote::RemoteEntryKind::SymbolicLink => EntryKind::SymbolicLink,
            strop_remote::RemoteEntryKind::Fifo => EntryKind::Fifo,
            strop_remote::RemoteEntryKind::Socket => EntryKind::Socket,
            strop_remote::RemoteEntryKind::BlockDevice => EntryKind::BlockDevice,
            strop_remote::RemoteEntryKind::CharacterDevice => EntryKind::CharacterDevice,
            strop_remote::RemoteEntryKind::Unknown => EntryKind::Unknown,
        };
        let mut observation = Observation::unknown(kind);
        observation.permissions = entry
            .permissions
            .map(|permissions| Permissions::from_mode(u32::from(permissions.bits())));
        observation.size = entry.size.map(|size| size.get());
        entries.push(DirectoryEntry {
            name: EntryName::new(name.into())
                .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?,
            observation,
            error: None,
        });
    }
    sort_entries(&mut entries)?;
    Ok(ListedDirectory {
        snapshot: DirectorySnapshot {
            location,
            entries: entries.into(),
            state,
        },
        connection: Some(listed.connection),
    })
}

pub fn from_container(
    location: ResourceLocation,
    source: Vec<strop_containers::DirEntry>,
) -> Result<ListedDirectory, FsFailure> {
    if !matches!(location.filesystem, Filesystem::Container(_)) {
        return Err(failure(
            FsFailureKind::Protocol,
            "container listing has a different filesystem namespace",
        ));
    }
    let mut entries = Vec::with_capacity(source.len().min(ENTRY_LIMIT));
    let mut state = ListingState::Complete;
    let mut bytes = 0;
    for entry in source {
        bytes += entry.name.len();
        if entries.len() == ENTRY_LIMIT || bytes > NAME_BYTES_LIMIT {
            state = ListingState::Limited {
                limit: entries.len(),
            };
            break;
        }
        let kind = match entry.kind {
            strop_containers::DirEntryKind::File => EntryKind::File,
            strop_containers::DirEntryKind::Dir => EntryKind::Directory,
            strop_containers::DirEntryKind::Symlink => EntryKind::SymbolicLink,
            strop_containers::DirEntryKind::Other => EntryKind::Unknown,
        };
        let mut observation = Observation::unknown(kind);
        observation.size = entry.size;
        entries.push(DirectoryEntry {
            name: EntryName::new(entry.name.into())
                .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?,
            observation,
            error: None,
        });
    }
    sort_entries(&mut entries)?;
    Ok(ListedDirectory {
        snapshot: DirectorySnapshot {
            location,
            entries: entries.into(),
            state,
        },
        connection: None,
    })
}

fn list_local(
    location: &ResourceLocation,
    token: &CancelToken,
) -> Result<ListedDirectory, FsFailure> {
    let reader = std::fs::read_dir(&location.path).map_err(io_failure)?;
    let mut entries = Vec::new();
    let mut state = ListingState::Complete;
    let mut bytes = 0;
    for entry in reader {
        if token.is_cancelled() {
            return Err(failure(
                FsFailureKind::Cancelled,
                "directory read cancelled",
            ));
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                state = ListingState::Failed {
                    message: error.to_string(),
                };
                break;
            }
        };
        let native_name = entry.file_name();
        bytes += native_name.as_encoded_bytes().len();
        if entries.len() == ENTRY_LIMIT || bytes > NAME_BYTES_LIMIT {
            state = ListingState::Limited {
                limit: entries.len(),
            };
            break;
        }
        let name = EntryName::new(native_name.into())
            .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?;
        let (observation, error) = match std::fs::symlink_metadata(entry.path()) {
            Ok(metadata) => (crate::observation::metadata(&metadata), None),
            Err(error) => (
                Observation::unknown(entry.file_type().map_or(EntryKind::Unknown, file_kind)),
                Some(error.to_string()),
            ),
        };
        entries.push(DirectoryEntry {
            name,
            observation,
            error,
        });
    }
    sort_entries(&mut entries)?;
    Ok(ListedDirectory {
        snapshot: DirectorySnapshot {
            location: location.clone(),
            entries: entries.into(),
            state,
        },
        connection: None,
    })
}

pub(crate) fn file_kind(kind: std::fs::FileType) -> EntryKind {
    if kind.is_dir() {
        return EntryKind::Directory;
    }
    if kind.is_file() {
        return EntryKind::File;
    }
    if kind.is_symlink() {
        return EntryKind::SymbolicLink;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if kind.is_fifo() {
            return EntryKind::Fifo;
        }
        if kind.is_socket() {
            return EntryKind::Socket;
        }
        if kind.is_block_device() {
            return EntryKind::BlockDevice;
        }
        if kind.is_char_device() {
            return EntryKind::CharacterDevice;
        }
    }
    EntryKind::Unknown
}

pub(crate) fn sort_entries(entries: &mut [DirectoryEntry]) -> Result<(), FsFailure> {
    let mut seen = std::collections::HashSet::with_capacity(entries.len());
    if entries.iter().any(|entry| !seen.insert(&entry.name)) {
        return Err(failure(
            FsFailureKind::Protocol,
            "directory listing contains duplicate native names",
        ));
    }
    drop(seen);
    entries.sort_unstable_by(|a, b| {
        let group =
            |entry: &DirectoryEntry| usize::from(entry.observation.kind != EntryKind::Directory);
        group(a).cmp(&group(b)).then_with(|| a.name.cmp(&b.name))
    });
    Ok(())
}

pub fn parent(location: &ResourceLocation) -> Option<ResourceLocation> {
    location
        .path
        .parent()
        .filter(|path| *path != Path::new(""))
        .map(|path| ResourceLocation {
            filesystem: location.filesystem.clone(),
            path: path.to_path_buf(),
        })
}
