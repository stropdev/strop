//! Application open targets (0042). `FileTarget` is the *unresolved* input an
//! open/browse command was given — a local path or a remote URI spelling — as
//! opposed to `strop_workspace::ResourceLocation`, the resolved identity of a
//! resource once it is known. Remote URIs and native local paths stay distinct.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use strop_workspace::{AddressError, RemoteLocation};

/// Local paths keep their existing native-byte wire representation; remote targets
/// have an explicit remote envelope, never an ambiguous legacy local-path string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTarget {
    Local(PathBuf),
    Remote(RemoteLocation),
    /// A path inside a running container (0037 DC1b) — the path names
    /// container bytes; there is never a local interpretation.
    Container {
        container: strop_workspace::ContainerId,
        path: PathBuf,
    },
}

impl Serialize for FileTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct RemoteRecord<'a> {
            remote: &'a RemoteLocation,
        }
        #[derive(Serialize)]
        struct ContainerRecord<'a> {
            container: &'a strop_workspace::ContainerId,
            #[serde(with = "strop_core::path_serde")]
            path: &'a PathBuf,
        }
        match self {
            Self::Local(path) => strop_core::path_serde::serialize(path, serializer),
            Self::Remote(remote) => RemoteRecord { remote }.serialize(serializer),
            Self::Container { container, path } => {
                ContainerRecord { container, path }.serialize(serializer)
            }
        }
    }
}
impl<'de> Deserialize<'de> for FileTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Record {
            Remote {
                remote: RemoteLocation,
            },
            Container {
                container: strop_workspace::ContainerId,
                #[serde(with = "strop_core::path_serde")]
                path: PathBuf,
            },
            Local(#[serde(with = "strop_core::path_serde")] PathBuf),
        }
        Ok(match Record::deserialize(deserializer)? {
            Record::Remote { remote } => Self::Remote(remote),
            Record::Container { container, path } => Self::Container { container, path },
            Record::Local(path) => Self::Local(path),
        })
    }
}
impl FileTarget {
    pub fn resource_location(&self) -> Option<strop_workspace::ResourceLocation> {
        use strop_workspace::{Filesystem, ResourceLocation};
        match self {
            Self::Local(path) if path.is_absolute() => Some(ResourceLocation::local(path.clone())),
            Self::Remote(location) => location.absolute_file().map(|file| {
                ResourceLocation::remote(file.endpoint().clone(), file.path().to_path_buf())
            }),
            Self::Container { container, path } if path.is_absolute() => Some(ResourceLocation {
                filesystem: Filesystem::Container(container.clone()),
                path: path.clone(),
            }),
            _ => None,
        }
    }
    pub fn from_location(
        location: &strop_workspace::ResourceLocation,
    ) -> Result<Self, AddressError> {
        use strop_workspace::Filesystem;
        if !location.path.is_absolute() {
            return Err(AddressError::RelativePath);
        }
        Ok(match &location.filesystem {
            Filesystem::Local => Self::Local(location.path.clone()),
            Filesystem::Remote(endpoint) => Self::Remote(
                strop_workspace::RemoteFile::from_path(endpoint.clone(), location.path.clone())?
                    .into(),
            ),
            Filesystem::Container(container) => Self::Container {
                container: container.clone(),
                path: location.path.clone(),
            },
        })
    }

    pub fn matches_location(&self, location: &strop_workspace::ResourceLocation) -> bool {
        use strop_workspace::Filesystem;
        match (self, &location.filesystem) {
            (Self::Local(path), Filesystem::Local) => path == &location.path,
            (Self::Remote(remote), Filesystem::Remote(endpoint)) => remote
                .absolute_file()
                .is_some_and(|file| file.endpoint() == endpoint && file.path() == location.path),
            (Self::Container { container, path }, Filesystem::Container(expected)) => {
                container == expected && path == &location.path
            }
            _ => false,
        }
    }
    /// Only textual user-entry boundaries interpret the scheme. Filesystem/LSP
    /// callers construct Local directly, including filenames containing `ssh:`.
    pub fn parse(value: PathBuf) -> Result<Self, AddressError> {
        if let Some(text) = value.to_str() {
            if text.starts_with("ssh://") {
                return RemoteLocation::parse(text).map(Self::Remote);
            }
            if text.starts_with("file://") || text.starts_with("container:") {
                return strop_workspace::ResourceLocation::parse_uri(text)
                    .and_then(|location| Self::from_location(&location));
            }
        }
        Ok(Self::Local(value))
    }
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            Self::Local(path) => Some(path),
            _ => None,
        }
    }
}
impl std::fmt::Display for FileTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(path) => path.display().fmt(formatter),
            Self::Remote(file) => file.fmt(formatter),
            Self::Container { container, path } => write!(
                formatter,
                "container:{}{}",
                &container.as_str()[..12],
                path.display()
            ),
        }
    }
}
