use super::language_bytes;
use crate::source::selection;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use strop_fs::workspace_manifest::{self, DirectoryWitness};

/// Advisory input to the policy walk. Replay keeps the native identity and
/// hints for display but deliberately cannot recreate a live binding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredBoundary {
    #[serde(with = "strop_core::path_serde")]
    root: PathBuf,
    languages: Option<Vec<String>>,
    #[serde(skip)]
    witness: Option<DirectoryWitness>,
}

impl DeclaredBoundary {
    /// An unbound record is usable as catalog metadata, never as a cwd.
    pub fn new(root: PathBuf, languages: Option<Vec<String>>) -> Option<Self> {
        if root
            .as_os_str()
            .as_encoded_bytes()
            .split(|&b| b == b'/')
            .any(|part| part.is_empty() || part == b"." || part == b".." || part.contains(&0))
        {
            return None;
        }
        let record = Self::from_parts(root, languages, None);
        declarations_fit(std::slice::from_ref(&record)).then_some(record)
    }

    fn from_parts(
        root: PathBuf,
        mut languages: Option<Vec<String>>,
        witness: Option<DirectoryWitness>,
    ) -> Self {
        if let Some(ids) = &mut languages {
            ids.sort_unstable();
            ids.dedup();
            ids.shrink_to_fit();
        }
        Self {
            root,
            languages,
            witness,
        }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// None preserves ordinary inference; Some([]) suppresses speculation.
    pub fn languages(&self) -> Option<&[String]> {
        self.languages.as_deref()
    }

    /// Worker-only. Re-observe current ignore/protected policy, manifest,
    /// workspace and member; use the returned fd, not the checked spelling.
    pub fn open_directory(&self, root: &Path) -> io::Result<File> {
        let witness = self.witness.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "declaration has no live directory binding",
            )
        })?;
        if !selection::observes_folder(root, &root.join(&self.root), &|| false) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "declared directory is outside current project discovery",
            ));
        }
        witness.reopen(root, &self.root)
    }

    pub(super) fn observed(&self, path: &Path) -> bool {
        self.witness.as_ref().is_none_or(|w| w.observes(path))
    }

    pub(crate) fn relative_to(&self, folder: &Path) -> Option<Self> {
        Some(Self {
            root: self.root.strip_prefix(folder).ok()?.to_owned(),
            languages: self.languages.clone(),
            witness: self.witness.clone(),
        })
    }

    fn bytes(&self) -> usize {
        size_of::<Self>() + self.root.capacity() + language_bytes(self.languages.as_ref())
    }
}

/// Decoded data (not serialized TOML bytes). Check at both the reader and
/// the inventory input: stale results and unrepresentable input are distinct.
pub(crate) fn declarations_fit(records: &[DeclaredBoundary]) -> bool {
    records.len() <= 256
        && records
            .iter()
            .try_fold(0usize, |sum, d| sum.checked_add(d.bytes()))
            .is_some_and(|n| n <= 64 * 1024)
}

/// Read one local root off the editor thread. Missing returns no declarations;
/// malformed input returns one diagnostic, never a partially installed list.
pub fn read_workspace_declarations(root: &Path) -> io::Result<Arc<[DeclaredBoundary]>> {
    let records: Arc<[_]> = workspace_manifest::read(root)?
        .into_iter()
        .map(|d| DeclaredBoundary::from_parts(d.path, d.languages, Some(d.witness)))
        .collect();
    if !declarations_fit(&records) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, ".strop/workspace.toml: decoded declarations exceed 64 KiB; reduce the number or size of language hints"));
    }
    Ok(records)
}

#[cfg(test)]
mod tests;
