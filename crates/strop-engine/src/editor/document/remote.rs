//! Remote provenance stays with its real buffer. Live leases never cross replay.
use super::{Document, DocumentSource, JumpRecord};
use strop_core::Buffer;
use strop_remote::{ConnectionLease, ReadSelection, RemoteSnapshot, RemoteWindow};
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
    pub fn remote_metadata(&self) -> Option<&RemoteDocument> {
        match &self.source {
            DocumentSource::Remote(source) => Some(source),
            _ => None,
        }
    }
}

impl Document {
    pub(crate) fn return_point(&self) -> Option<&JumpRecord> {
        match &self.source {
            DocumentSource::Surface(surface) => surface.content.return_point(),
            DocumentSource::Remote(source) => source.return_to.as_ref(),
            DocumentSource::Directory(source) => source.return_to.as_ref(),
            DocumentSource::Output { return_to } => return_to.as_ref(),
            _ => None,
        }
    }
    pub(crate) fn return_point_mut(&mut self) -> Option<&mut JumpRecord> {
        match &mut self.source {
            DocumentSource::Surface(surface) => surface.content.return_slot().as_mut(),
            DocumentSource::Remote(source) => source.return_to.as_mut(),
            DocumentSource::Directory(source) => source.return_to.as_mut(),
            DocumentSource::Output { return_to } => return_to.as_mut(),
            _ => None,
        }
    }
    pub(crate) fn set_return_point(&mut self, point: JumpRecord) {
        match &mut self.source {
            DocumentSource::Surface(surface) => surface.content.set_return_point(point),
            DocumentSource::Remote(source) => source.return_to = Some(point),
            DocumentSource::Directory(source) => source.return_to = Some(point),
            DocumentSource::Output { return_to } => *return_to = Some(point),
            _ => {}
        }
    }
}
