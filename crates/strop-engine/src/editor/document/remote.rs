//! Remote provenance stays with its real buffer. Live leases never cross replay.
use super::{Document, DocumentSource, JumpRecord};
use std::fmt::Write;
use strop_core::Buffer;
use strop_remote::{
    ConnectionLease, ReadSelection, RemoteDirectorySnapshot, RemoteEntry, RemoteEntryKind,
    RemoteSnapshot, RemoteWindow,
};
use strop_workspace::RemoteFile;

#[derive(Clone)]
pub struct RemoteDocument {
    pub file: RemoteFile,
    pub window: RemoteWindow,
    pub selection: ReadSelection,
    pub connection: Option<ConnectionLease>,
    pub return_to: Option<JumpRecord>,
    pub(crate) write: Option<crate::editor::remote::save::WritePermit>,
}
impl std::fmt::Debug for RemoteDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteDocument")
            .field("file", &self.file)
            .field("window", &self.window)
            .field("selection", &self.selection)
            .field("connected", &self.connection.is_some())
            .field("write_authorized", &self.write.is_some())
            .finish()
    }
}

#[derive(Clone)]
pub struct RemoteDirectory {
    pub directory: RemoteFile,
    pub entries: std::sync::Arc<[RemoteEntry]>,
    pub visible: Vec<usize>,
    pub filter: String,
    pub connection: Option<ConnectionLease>,
    pub return_to: Option<JumpRecord>,
}
impl std::fmt::Debug for RemoteDirectory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteDirectory")
            .field("directory", &self.directory)
            .field("entries", &self.entries.len())
            .field("filter", &self.filter)
            .field("connected", &self.connection.is_some())
            .finish()
    }
}
impl RemoteDirectory {
    pub fn text(&self) -> String {
        let size_width = self
            .visible
            .iter()
            .filter_map(|&index| self.entries[index].size)
            .map(|size| size.get().checked_ilog10().unwrap_or(0) as usize + 1)
            .max()
            .unwrap_or(1)
            .max(5);
        let mut text = String::new();
        // fmt::Write on String is infallible; parent attributes were not fetched.
        let _ = writeln!(text, "d????????? {:>size_width$} ../", "?");
        for &index in &self.visible {
            let entry = &self.entries[index];
            text.push(entry.kind.marker());
            match entry.permissions {
                Some(permissions) => {
                    let _ = write!(text, "{permissions}");
                }
                None => text.push_str("?????????"),
            }
            match entry.size {
                Some(size) => {
                    let _ = write!(text, " {:>size_width$} ", size.get());
                }
                None => {
                    let _ = write!(text, " {:>size_width$} ", "?");
                }
            }
            let name = entry
                .file
                .path()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            text.push_str(&strop_core::layout::printable_text(name));
            if entry.kind == RemoteEntryKind::Directory {
                text.push('/');
            }
            if entry.kind == RemoteEntryKind::SymbolicLink {
                text.push('@');
            }
            text.push('\n');
        }
        text
    }
    pub fn parent(&self) -> Result<Option<RemoteFile>, strop_workspace::AddressError> {
        self.directory
            .path()
            .parent()
            .map(|parent| self.directory.with_path(parent.to_path_buf()))
            .transpose()
    }
    pub fn entry(&self, line: strop_core::id::LineIndex) -> Option<&RemoteEntry> {
        line.get()
            .checked_sub(1)
            .and_then(|line| self.visible.get(line))
            .and_then(|&index| self.entries.get(index))
    }
    /// Entries are grouped directory-first then native-path sorted. Search
    /// both groups and the sorted visible mapping without scanning the listing.
    pub(crate) fn line_for(&self, file: &RemoteFile) -> Option<strop_core::id::LineIndex> {
        if file.endpoint() != self.directory.endpoint() {
            return None;
        }
        let split = self
            .entries
            .partition_point(|entry| entry.kind == RemoteEntryKind::Directory);
        for (offset, entries) in [(0, &self.entries[..split]), (split, &self.entries[split..])] {
            if let Ok(index) = entries.binary_search_by(|entry| {
                entry.file.path().as_os_str().cmp(file.path().as_os_str())
            }) {
                return self
                    .visible
                    .binary_search(&(offset + index))
                    .ok()
                    .map(|row| strop_core::id::LineIndex::new(row + 1));
            }
        }
        None
    }
}

impl Document {
    pub fn remote(mut buffer: Buffer, source: RemoteDocument) -> Self {
        debug_assert!(
            buffer.path.is_none(),
            "remote identity cannot become a local file"
        );
        buffer.name = Some(source.file.to_string());
        buffer.readonly = true;
        let detection = Some(super::detect_indent(buffer.text()));
        Self {
            buf: buffer,
            syntax_hint: None,
            indent: super::Indent::default(),
            indent_override: super::IndentOverride::default(),
            detection,
            source: DocumentSource::Remote(Box::new(source)),
        }
    }
    pub(crate) fn remote_snapshot(snapshot: RemoteSnapshot, selection: ReadSelection) -> Self {
        Self::remote(
            snapshot.buffer,
            RemoteDocument {
                file: snapshot.file,
                window: snapshot.window,
                selection,
                connection: Some(snapshot.connection),
                return_to: None,
                write: None,
            },
        )
    }
    pub(crate) fn remote_directory(snapshot: RemoteDirectorySnapshot) -> Self {
        let mut entries = snapshot.entries;
        entries.sort_by(|left, right| {
            let left_directory = left.kind == RemoteEntryKind::Directory;
            let right_directory = right.kind == RemoteEntryKind::Directory;
            right_directory.cmp(&left_directory).then_with(|| {
                left.file
                    .path()
                    .as_os_str()
                    .cmp(right.file.path().as_os_str())
            })
        });
        let directory = RemoteDirectory {
            directory: snapshot.directory,
            visible: (0..entries.len()).collect(),
            entries: entries.into(),
            filter: String::new(),
            connection: Some(snapshot.connection),
            return_to: None,
        };
        let buffer = Buffer::from_text(&directory.text());
        Self::directory(buffer, directory)
    }
    pub fn directory(mut buffer: Buffer, source: RemoteDirectory) -> Self {
        buffer.name = Some(source.directory.to_string());
        buffer.readonly = true;
        Self {
            buf: buffer,
            syntax_hint: None,
            indent: super::Indent::default(),
            indent_override: super::IndentOverride::default(),
            detection: None,
            source: DocumentSource::RemoteDirectory(Box::new(source)),
        }
    }
    pub fn remote_metadata(&self) -> Option<&RemoteDocument> {
        match &self.source {
            DocumentSource::Remote(source) => Some(source),
            _ => None,
        }
    }
    pub fn directory_metadata_ref(&self) -> Option<&RemoteDirectory> {
        match &self.source {
            DocumentSource::RemoteDirectory(source) => Some(source),
            _ => None,
        }
    }
}

impl Document {
    pub(crate) fn return_point(&self) -> Option<&JumpRecord> {
        match &self.source {
            DocumentSource::Surface(surface) => surface.content.return_point(),
            DocumentSource::Remote(source) => source.return_to.as_ref(),
            DocumentSource::RemoteDirectory(source) => source.return_to.as_ref(),
            DocumentSource::Output { return_to } => return_to.as_ref(),
            _ => None,
        }
    }
    pub(crate) fn return_point_mut(&mut self) -> Option<&mut JumpRecord> {
        match &mut self.source {
            DocumentSource::Surface(surface) => surface.content.return_slot().as_mut(),
            DocumentSource::Remote(source) => source.return_to.as_mut(),
            DocumentSource::RemoteDirectory(source) => source.return_to.as_mut(),
            DocumentSource::Output { return_to } => return_to.as_mut(),
            _ => None,
        }
    }
    pub(crate) fn set_return_point(&mut self, point: JumpRecord) {
        match &mut self.source {
            DocumentSource::Surface(surface) => surface.content.set_return_point(point),
            DocumentSource::Remote(source) => source.return_to = Some(point),
            DocumentSource::RemoteDirectory(source) => source.return_to = Some(point),
            DocumentSource::Output { return_to } => *return_to = Some(point),
            _ => {}
        }
    }
}
