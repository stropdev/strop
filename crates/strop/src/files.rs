//! Application open targets. Remote URIs and native local paths are distinct identities.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use strop_remote::{AddressError, RemoteLocation};

/// Local paths keep their existing native-byte wire representation; remote targets
/// have an explicit remote envelope, never an ambiguous legacy local-path string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTarget {
    Local(PathBuf),
    Remote(RemoteLocation),
}

impl Serialize for FileTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct RemoteRecord<'a> {
            remote: &'a RemoteLocation,
        }
        match self {
            Self::Local(path) => strop_core::path_serde::serialize(path, serializer),
            Self::Remote(remote) => RemoteRecord { remote }.serialize(serializer),
        }
    }
}
impl<'de> Deserialize<'de> for FileTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Record {
            Remote { remote: RemoteLocation },
            Local(#[serde(with = "strop_core::path_serde")] PathBuf),
        }
        Ok(match Record::deserialize(deserializer)? {
            Record::Remote { remote } => Self::Remote(remote),
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
            Self::Remote(_) => None,
        }
    }
}
impl std::fmt::Display for FileTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(path) => path.display().fmt(formatter),
            Self::Remote(file) => file.fmt(formatter),
        }
    }
}
