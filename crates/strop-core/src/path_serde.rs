//! Versioned native path representation. No lossy round-trip can select a file.
use serde::{Deserialize, Serialize};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("unsupported path encoding version")]
    Encoding,
    #[error("path is empty or contains a native NUL")]
    Invalid,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "encoding", deny_unknown_fields)]
enum Native {
    #[serde(rename = "unix-bytes")]
    Unix { version: u32, bytes: Vec<u8> },
}

pub fn serialize<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    Native::Unix {
        version: 1,
        bytes: path.as_os_str().as_bytes().to_vec(),
    }
    .serialize(serializer)
}

pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    decode(Native::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

fn decode(native: Native) -> Result<PathBuf, PathError> {
    let Native::Unix { version, bytes } = native;
    if version != 1 {
        return Err(PathError::Encoding);
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    validate(&path)?;
    Ok(path)
}

pub fn validate(path: &Path) -> Result<(), PathError> {
    if path.as_os_str().is_empty() || path.as_os_str().as_bytes().contains(&0) {
        return Err(PathError::Invalid);
    }
    Ok(())
}

pub mod option {
    use super::*;
    struct PathRef<'a>(&'a Path);
    impl Serialize for PathRef<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            super::serialize(self.0, serializer)
        }
    }
    pub fn serialize<S: serde::Serializer>(
        path: &Option<PathBuf>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        path.as_deref().map(PathRef).serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<PathBuf>, D::Error> {
        Option::<Native>::deserialize(deserializer)?
            .map(decode)
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}
