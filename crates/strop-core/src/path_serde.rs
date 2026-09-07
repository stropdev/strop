//! Versioned native path representation. No lossy round-trip can select a file.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("legacy path contains an ambiguous replacement character")]
    AmbiguousLegacy,
    #[error("unsupported path encoding version or platform")]
    Encoding,
    #[error("path is empty or contains a native NUL")]
    Invalid,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "encoding", deny_unknown_fields)]
enum Native {
    #[serde(rename = "unix-bytes")]
    Unix { version: u32, bytes: Vec<u8> },
    #[serde(rename = "windows-utf16")]
    Windows { version: u32, units: Vec<u16> },
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Wire {
    Legacy(String),
    Native(Native),
}

pub fn serialize<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Native::Unix {
            version: 1,
            bytes: path.as_os_str().as_bytes().to_vec(),
        }
        .serialize(serializer)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        Native::Windows {
            version: 1,
            units: path.as_os_str().encode_wide().collect(),
        }
        .serialize(serializer)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(serde::ser::Error::custom(PathError::Encoding))
    }
}

pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    decode(Wire::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

fn decode(wire: Wire) -> Result<PathBuf, PathError> {
    let path = match wire {
        Wire::Legacy(value) => {
            if value.contains('\u{fffd}') {
                return Err(PathError::AmbiguousLegacy);
            }
            PathBuf::from(value)
        }
        Wire::Native(native) => match native {
            #[cfg(unix)]
            Native::Unix { version: 1, bytes } => {
                use std::os::unix::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_vec(bytes))
            }
            #[cfg(windows)]
            Native::Windows { version: 1, units } => {
                use std::os::windows::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_wide(&units))
            }
            _ => return Err(PathError::Encoding),
        },
    };
    validate(&path)?;
    Ok(path)
}

pub fn validate(path: &Path) -> Result<(), PathError> {
    #[cfg(unix)]
    let nul = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().contains(&0)
    };
    #[cfg(windows)]
    let nul = {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().any(|unit| unit == 0)
    };
    #[cfg(not(any(unix, windows)))]
    let nul = true;
    if path.as_os_str().is_empty() || nul {
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
        Option::<Wire>::deserialize(deserializer)?
            .map(decode)
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}
