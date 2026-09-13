//! Pure directory identity and metadata, shared by local, SSH and container views.
use crate::ResourceLocation;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// One native child component. Display text never becomes path authority.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryName(PathBuf);
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a directory entry must be one nonempty native name, without NUL or a parent component")]
pub struct EntryNameError;
impl EntryName {
    pub fn new(name: PathBuf) -> Result<Self, EntryNameError> {
        let mut components = name.components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || name.file_name() != Some(name.as_os_str())
            || name.as_os_str().as_encoded_bytes().contains(&0)
        {
            return Err(EntryNameError);
        }
        Ok(Self(name))
    }
    pub fn as_path(&self) -> &Path {
        &self.0
    }
    pub fn display(&self) -> String {
        display_path(&self.0)
    }
}
impl serde::Serialize for EntryName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        strop_core::path_serde::serialize(&self.0, serializer)
    }
}
impl<'de> serde::Deserialize<'de> for EntryName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(strop_core::path_serde::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

/// Lossless display: valid Unicode remains readable; controls, backslashes and
/// invalid native bytes have distinct visible spellings. This is not an input URI.
pub fn display_path(path: &Path) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for chunk in path.as_os_str().as_encoded_bytes().utf8_chunks() {
        for character in chunk.valid().chars() {
            match character {
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                value
                    if value.is_control()
                        || matches!(value, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
                {
                    let _ = write!(out, "\\u{{{:X}}}", value as u32);
                }
                value => out.push(value),
            }
        }
        for byte in chunk.invalid() {
            let _ = write!(out, "\\x{byte:02X}");
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EntryKind {
    File,
    Directory,
    SymbolicLink,
    Fifo,
    Socket,
    BlockDevice,
    CharacterDevice,
    Unknown,
}
impl EntryKind {
    pub const fn marker(self) -> char {
        match self {
            Self::File => '-',
            Self::Directory => 'd',
            Self::SymbolicLink => 'l',
            Self::Fifo => 'p',
            Self::Socket => 's',
            Self::BlockDevice => 'b',
            Self::CharacterDevice => 'c',
            Self::Unknown => '?',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct Permissions(u16);
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("permission bits exceed POSIX access and special bits")]
pub struct PermissionBitsError;
impl Permissions {
    pub const fn new(bits: u16) -> Result<Self, PermissionBitsError> {
        if bits & !0o7777 != 0 {
            Err(PermissionBitsError)
        } else {
            Ok(Self(bits))
        }
    }
    pub const fn from_mode(mode: u32) -> Self {
        Self((mode & 0o7777) as u16)
    }
    pub const fn bits(self) -> u16 {
        self.0
    }
}
impl TryFrom<u16> for Permissions {
    type Error = PermissionBitsError;
    fn try_from(bits: u16) -> Result<Self, Self::Error> {
        Self::new(bits)
    }
}
impl From<Permissions> for u16 {
    fn from(value: Permissions) -> Self {
        value.bits()
    }
}
impl std::fmt::Display for Permissions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (read, write, execute, special, lower, upper) in [
            (0o400, 0o200, 0o100, 0o4000, 's', 'S'),
            (0o040, 0o020, 0o010, 0o2000, 's', 'S'),
            (0o004, 0o002, 0o001, 0o1000, 't', 'T'),
        ] {
            let r = if self.0 & read != 0 { 'r' } else { '-' };
            let w = if self.0 & write != 0 { 'w' } else { '-' };
            let x = match (self.0 & execute != 0, self.0 & special != 0) {
                (true, true) => lower,
                (false, true) => upper,
                (true, false) => 'x',
                (false, false) => '-',
            };
            write!(formatter, "{r}{w}{x}")?;
        }
        Ok(())
    }
}

/// Identity and time observations are facts supplied by the backend, never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ObjectId {
    pub device: u64,
    pub inode: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FileTime {
    pub seconds: i64,
    pub nanos: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Observation {
    pub kind: EntryKind,
    pub identity: Option<ObjectId>,
    pub size: Option<u64>,
    pub modified: Option<FileTime>,
    pub changed: Option<FileTime>,
    pub permissions: Option<Permissions>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub links: Option<u64>,
    pub digest: Option<[u8; 32]>,
}
impl Observation {
    pub fn unknown(kind: EntryKind) -> Self {
        Self {
            kind,
            identity: None,
            size: None,
            modified: None,
            changed: None,
            permissions: None,
            uid: None,
            gid: None,
            links: None,
            digest: None,
        }
    }
    pub fn same_object(&self, other: &Self) -> bool {
        self.identity.is_some() && self.identity == other.identity && self.kind == other.kind
    }
    /// Compare metadata facts separately from an optional content observation.
    pub fn same_metadata(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.identity == other.identity
            && self.size == other.size
            && self.modified == other.modified
            && self.changed == other.changed
            && self.permissions == other.permissions
            && self.uid == other.uid
            && self.gid == other.gid
            && self.links == other.links
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DirectoryEntry {
    pub name: EntryName,
    pub observation: Observation,
    pub error: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ListingState {
    Complete,
    Limited { limit: usize },
    Failed { message: String },
}
impl ListingState {
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DirectorySnapshot {
    pub location: ResourceLocation,
    pub entries: Arc<[DirectoryEntry]>,
    pub state: ListingState,
}
impl DirectorySnapshot {
    pub fn location_of(&self, entry: &DirectoryEntry) -> ResourceLocation {
        ResourceLocation {
            filesystem: self.location.filesystem.clone(),
            path: self.location.path.join(entry.name.as_path()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_native_components_not_paths_or_urls() {
        for rejected in ["", ".", "..", "/absolute", "parent/child", "child/", "a\0b"] {
            assert!(EntryName::new(rejected.into()).is_err());
        }
        for accepted in ["ssh:literal", "-dash", "space name", "line\nbreak", "界"] {
            let name = EntryName::new(accepted.into()).unwrap();
            assert_eq!(name.as_path(), Path::new(accepted));
            assert!(!name.display().contains('\n'));
        }
    }
}
