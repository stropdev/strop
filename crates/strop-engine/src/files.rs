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
    /// Only textual user-entry boundaries interpret the scheme. Filesystem/LSP
    /// callers construct Local directly, including filenames containing `ssh:`.
    pub fn parse(value: PathBuf) -> Result<Self, AddressError> {
        match value.to_str().filter(|text| text.starts_with("ssh://")) {
            Some(uri) => RemoteLocation::parse(uri).map(Self::Remote),
            None => Ok(Self::Local(value)),
        }
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
