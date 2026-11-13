//! One worker-owned commit view: the commit's message, its files with
//! their native identity, the buffer text and the typed rows that say
//! what each line is (0100 §4.4), and the sidebar tree.
//!
//! ```text
//! commit 19b4e9aec5          heading: author and age are drawn
//! rename the doc, …          the subject
//!                            (a blank row and the body, when there is one)
//!
//! Files                      heading: count and `+N -M` are drawn
//! R docs/old.md → docs/new.md
//! M src/lib.rs
//! ```
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use super::sidebar::Sidebar;
use ropey::Rope;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strop_core::layout::printable_text;
use strop_git::memory::{ChangedFile, CommitSummary};

/// What a row of the commit view is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesRow {
    /// Row 0: `commit <sha>`.
    Commit,
    Subject,
    /// A line of the message's body.
    Body,
    Blank,
    /// The `Files` heading.
    Files,
    /// The file at this index of the commit's list.
    File(usize),
}

#[derive(Debug, Clone)]
pub struct PreparedFiles(Arc<FileList>);
#[derive(Debug)]
struct FileList {
    summary: CommitSummary,
    files: Vec<ChangedFile>,
    by_path: Vec<usize>,
    rows: Vec<FilesRow>,
    text: Rope,
    sidebar: Sidebar,
}

/// A file's row text: its status letter and path, `R old → new` for a
/// rename or copy.
pub fn file_row_text(file: &ChangedFile) -> String {
    let path = printable_text(file.path.to_string_lossy());
    match &file.old_path {
        Some(old) => format!(
            "{} {} → {path}",
            file.status.letter(),
            printable_text(old.to_string_lossy())
        ),
        None => format!("{} {path}", file.status.letter()),
    }
}

impl PreparedFiles {
    pub fn new(summary: CommitSummary, files: Vec<ChangedFile>) -> Self {
        let mut text = String::new();
        let mut rows = Vec::new();
        let mut push = |line: &str, row: FilesRow| {
            text.push_str(line);
            text.push('\n');
            rows.push(row);
        };
        push(
            &format!("commit {}", summary.sha.get(..10).unwrap_or(&summary.sha)),
            FilesRow::Commit,
        );
        push(&printable_text(summary.subject.as_str()), FilesRow::Subject);
        if !summary.body.is_empty() {
            push("", FilesRow::Blank);
            for line in summary.body.lines() {
                push(&printable_text(line), FilesRow::Body);
            }
        }
        push("", FilesRow::Blank);
        push("Files", FilesRow::Files);
        for (index, file) in files.iter().enumerate() {
            push(&file_row_text(file), FilesRow::File(index));
        }
        let mut by_path: Vec<usize> = (0..files.len()).collect();
        by_path.sort_unstable_by(|&left, &right| files[left].path.cmp(&files[right].path));
        let sidebar = Sidebar::build(&files);
        Self(Arc::new(FileList {
            summary,
            files,
            by_path,
            rows,
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
    /// The commit: its sha, author, time, subject and body.
    pub fn summary(&self) -> &CommitSummary {
        &self.0.summary
    }
    pub fn sha(&self) -> &str {
        &self.0.summary.sha
    }
    /// What row `row` of the buffer is.
    pub fn row(&self, row: usize) -> Option<FilesRow> {
        self.0.rows.get(row).copied()
    }
    /// The row of the first file: where the view opens.
    pub(crate) fn first_file_row(&self) -> Option<usize> {
        self.0
            .rows
            .iter()
            .position(|row| matches!(row, FilesRow::File(_)))
    }
    /// The file a row of the buffer names.
    pub fn file_at_row(&self, row: usize) -> Option<&ChangedFile> {
        match self.row(row)? {
            FilesRow::File(index) => self.0.files.get(index),
            _ => None,
        }
    }
    /// The file at `path`, the commit's identity for it.
    pub fn file_at_path(&self, path: &Path) -> Option<&ChangedFile> {
        self.0.files.get(self.index_of(path)?)
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
        (&self.0.summary, &self.0.files).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for PreparedFiles {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (summary, files) = Deserialize::deserialize(deserializer)?;
        Ok(Self::new(summary, files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_git::memory::FileStatus;

    fn file(path: &str, status: FileStatus, old: Option<&str>) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            status,
            old_path: old.map(Into::into),
            ..Default::default()
        }
    }

    /// The buffer holds the commit's words and each file's letter and
    /// path; the rows say which is which.
    #[test]
    fn the_text_is_the_commit_then_its_files() {
        let summary = CommitSummary {
            sha: "19b4e9aec5ffff".into(),
            subject: "rename the doc".into(),
            body: "why:\nbecause".into(),
            ..Default::default()
        };
        let files = PreparedFiles::new(
            summary,
            vec![
                file("docs/new.md", FileStatus::Renamed, Some("docs/old.md")),
                file("src/lib.rs", FileStatus::Modified, None),
            ],
        );
        assert_eq!(
            files.text().to_string(),
            "commit 19b4e9aec5\nrename the doc\n\nwhy:\nbecause\n\nFiles\n\
             R docs/old.md → docs/new.md\nM src/lib.rs\n"
        );
        use FilesRow::*;
        let rows: Vec<_> = (0..9).map(|row| files.row(row).unwrap()).collect();
        assert_eq!(
            rows,
            [
                Commit,
                Subject,
                Blank,
                Body,
                Body,
                Blank,
                Files,
                File(0),
                File(1)
            ]
        );
        assert_eq!(files.first_file_row(), Some(7));
        assert_eq!(files.file_at_row(8).unwrap().path, Path::new("src/lib.rs"));
        assert!(files.file_at_row(6).is_none());
    }

    /// A one-line message has no body block.
    #[test]
    fn no_body_no_blank_under_the_subject() {
        let files = PreparedFiles::new(
            CommitSummary {
                sha: "abc".into(),
                subject: "fix".into(),
                ..Default::default()
            },
            vec![file("a.rs", FileStatus::Added, None)],
        );
        assert_eq!(
            files.text().to_string(),
            "commit abc\nfix\n\nFiles\nA a.rs\n"
        );
    }
}
