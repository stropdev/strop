//! Worker-only listing. The native kernel lists its admitted namespace;
//! detached namespaces arrive as decoded entry data for validation and
//! normalization. Only native entry names become child locations.
use crate::{failure, io_failure, ExecutionContext, NamespaceView};
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
}

/// List the admitted native namespace. Detached namespaces are identity-only
/// data to this kernel: their listings arrive through the client's transport
/// and are validated by [`assemble`].
pub fn list(
    context: &ExecutionContext,
    location: &ResourceLocation,
    token: &CancelToken,
) -> Result<ListedDirectory, FsFailure> {
    context.admit(location)?;
    if !location.path.is_absolute() {
        return Err(failure(
            FsFailureKind::InvalidPath,
            "directory scope must be absolute",
        ));
    }
    match context.namespace() {
        NamespaceView::Native => list_local(location, token),
        namespace => Err(failure(
            FsFailureKind::Unsupported,
            format!(
                "namespace {} is detached from this kernel; list it through its admitted transport",
                namespace.filesystem().label()
            ),
        )),
    }
}

/// One directory child as decoded transport data — never a transport type.
/// The client decodes its wire format; the kernel validates and normalizes.
pub struct ObservedEntry {
    pub name: std::ffi::OsString,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub permissions: Option<Permissions>,
}

/// Validate, bound and normalize a detached namespace's decoded listing.
/// The native namespace lists itself through [`list`]; assembling one from
/// decoded data would bypass native observation and is refused.
pub fn assemble(
    location: ResourceLocation,
    source: Vec<ObservedEntry>,
) -> Result<ListedDirectory, FsFailure> {
    if matches!(location.filesystem, Filesystem::Local) {
        return Err(failure(
            FsFailureKind::Protocol,
            "the native namespace lists through the kernel, not decoded data",
        ));
    }
    let mut entries = Vec::with_capacity(source.len().min(ENTRY_LIMIT));
    let mut state = ListingState::Complete;
    let mut bytes = 0;
    for entry in source {
        bytes += entry.name.as_encoded_bytes().len();
        if entries.len() == ENTRY_LIMIT || bytes > NAME_BYTES_LIMIT {
            state = ListingState::Limited {
                limit: entries.len(),
            };
            break;
        }
        let mut observation = Observation::unknown(entry.kind);
        observation.size = entry.size;
        observation.permissions = entry.permissions;
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
