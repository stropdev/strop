//! One worker-owned commit catalog: native identity, real buffer text and tree.
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use super::sidebar::Sidebar;
use ropey::Rope;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strop_git::memory::ChangedFile;

#[derive(Debug, Clone)]
pub struct PreparedFiles(Arc<FileList>);
#[derive(Debug)]
struct FileList {
    sha: String,
    files: Vec<ChangedFile>,
    by_path: Vec<usize>,
    text: Rope,
    sidebar: Sidebar,
}
impl PreparedFiles {
    pub fn new(sha: String, files: Vec<ChangedFile>) -> Self {
        let mut text = format!("commit {}\n\n", sha.get(..10).unwrap_or(&sha));
        for file in &files {
            text.push_str(&strop_core::layout::printable_text(
                file.path.to_string_lossy(),
            ));
            text.push('\n');
        }
        let mut by_path: Vec<usize> = (0..files.len()).collect();
        by_path.sort_unstable_by(|&left, &right| files[left].path.cmp(&files[right].path));
        let sidebar = Sidebar::build(&files);
        Self(Arc::new(FileList {
            sha,
            files,
            by_path,
            text: Rope::from_str(&text),
            sidebar,
        }))
    }
    pub(crate) fn text(&self) -> Rope {
        self.0.text.clone()
    }
    pub fn sidebar(&self) -> &Sidebar {
        &self.0.sidebar
    }
    pub(crate) fn index_of(&self, path: &Path) -> Option<usize> {
        let index = self
            .0
            .by_path
            .binary_search_by(|&index| self.0.files[index].path.as_path().cmp(path))
            .ok()?;
        Some(self.0.by_path[index])
    }
}
impl Deref for PreparedFiles {
    type Target = [ChangedFile];
    fn deref(&self) -> &Self::Target {
        &self.0.files
    }
}
impl<'a> IntoIterator for &'a PreparedFiles {
    type Item = &'a ChangedFile;
    type IntoIter = std::slice::Iter<'a, ChangedFile>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl Serialize for PreparedFiles {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (&self.0.sha, &self.0.files).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for PreparedFiles {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (sha, files) = Deserialize::deserialize(deserializer)?;
        Ok(Self::new(sha, files))
    }
}
