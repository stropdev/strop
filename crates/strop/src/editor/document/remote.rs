//! Remote provenance stays with its real buffer. Live leases never cross replay.
use super::{Document, DocumentSource, ReturnPoint};
use strop_core::Buffer;
use strop_remote::{
    ConnectionLease, ReadSelection, RemoteDirectorySnapshot, RemoteEntry, RemoteEntryKind,
    RemoteFile, RemoteSnapshot, RemoteWindow,
};

#[derive(Clone)]
pub struct RemoteDocument {
    pub file: RemoteFile,
    pub window: RemoteWindow,
    pub selection: ReadSelection,
    pub connection: Option<ConnectionLease>,
    pub return_to: Option<ReturnPoint>,
}
impl std::fmt::Debug for RemoteDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteDocument")
            .field("file", &self.file)
            .field("window", &self.window)
            .field("selection", &self.selection)
            .field("connected", &self.connection.is_some())
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
    pub return_to: Option<ReturnPoint>,
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
        let mut text = String::from("../\n");
        for &index in &self.visible {
            let entry = &self.entries[index];
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
            text.push('\n');
        }
        text
    }
    pub fn parent(&self) -> Result<Option<RemoteFile>, strop_remote::AddressError> {
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
}

impl Document {
    pub(crate) fn remote(mut buffer: Buffer, source: RemoteDocument) -> Self {
        debug_assert!(
            buffer.path.is_none(),
            "remote identity cannot become a local file"
        );
        buffer.name = Some(source.file.to_string());
        buffer.readonly = true;
        let highlighter = strop_syntax::Highlighter::for_path(source.file.path(), buffer.text());
        Self {
            buf: buffer,
            highlighter,
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
    pub(crate) fn directory(mut buffer: Buffer, source: RemoteDirectory) -> Self {
        buffer.name = Some(source.directory.to_string());
        buffer.readonly = true;
        Self {
            buf: buffer,
            highlighter: None,
            source: DocumentSource::RemoteDirectory(Box::new(source)),
        }
    }
    pub(crate) fn remote_metadata(&self) -> Option<&RemoteDocument> {
        match &self.source {
            DocumentSource::Remote(source) => Some(source),
            _ => None,
        }
    }
    pub(crate) fn directory_metadata_ref(&self) -> Option<&RemoteDirectory> {
        match &self.source {
            DocumentSource::RemoteDirectory(source) => Some(source),
            _ => None,
        }
    }
}

impl Document {
    pub(crate) fn return_point(&self) -> Option<&ReturnPoint> {
        match &self.source {
            DocumentSource::Surface(surface) => surface.content.return_point(),
            DocumentSource::Remote(source) => source.return_to.as_ref(),
            DocumentSource::RemoteDirectory(source) => source.return_to.as_ref(),
            _ => None,
        }
    }
    pub(crate) fn set_return_point(&mut self, point: ReturnPoint) {
        match &mut self.source {
            DocumentSource::Surface(surface) => surface.content.set_return_point(point),
            DocumentSource::Remote(source) => source.return_to = Some(point),
            DocumentSource::RemoteDirectory(source) => source.return_to = Some(point),
            _ => {}
        }
    }
}
