//! Pure completion data crosses replay; parsers, clients and live leases never do.
use super::Opened;
use crate::editor::document::{Directory, DocumentSource, JumpRecord, RemoteDocument};
use crate::editor::Document;
use crate::files::FileTarget;
use serde::{Deserialize, Serialize};
use strop_remote::Stamp;

#[derive(Deserialize)]
struct RemoteRecord {
    stamp: Stamp,
    return_to: Option<JumpRecord>,
}
#[derive(Deserialize)]
struct Record {
    buffer: strop_core::BufferSeed,
    canonical: FileTarget,
    remote: Option<RemoteRecord>,
    directory: Option<Directory>,
}
#[derive(Serialize)]
struct RemoteRecordRef<'a> {
    stamp: Stamp,
    return_to: &'a Option<JumpRecord>,
}
#[derive(Serialize)]
struct RecordRef<'a> {
    buffer: strop_core::BufferSeed,
    canonical: &'a FileTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote: Option<RemoteRecordRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    directory: Option<&'a Directory>,
}
impl Serialize for Opened {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let remote = self
            .document
            .remote_metadata()
            .map(|source| RemoteRecordRef {
                stamp: source.stamp,
                return_to: &source.return_to,
            });
        RecordRef {
            buffer: self.document.buf.seed(),
            canonical: &self.canonical,
            remote,
            directory: self.document.directory_metadata_ref(),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Opened {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let record = Record::deserialize(deserializer)?;
        let buffer = record
            .buffer
            .into_buffer()
            .map_err(serde::de::Error::custom)?;
        let document = if let Some(source) = record.directory {
            if record.remote.is_some()
                || buffer.path.is_some()
                || buffer.is_dirty()
                || !source.validate()
                || !record.canonical.matches_location(&source.location)
                || buffer.text() != source.text().as_str()
            {
                return Err(serde::de::Error::custom(
                    "directory completion rows or resource identity are inconsistent",
                ));
            }
            Document::directory(buffer, source)
        } else {
            match (&record.canonical, record.remote) {
                (FileTarget::Local(_), None) => Document::new(buffer),
                (FileTarget::Remote(location), Some(metadata)) => {
                    if buffer.path.is_some() || buffer.is_dirty() {
                        return Err(serde::de::Error::custom(
                            "remote completion has local or dirty content",
                        ));
                    }
                    let file = location.absolute_file().cloned().ok_or_else(|| {
                        serde::de::Error::custom(
                            "remote completion must have canonical absolute identity",
                        )
                    })?;
                    Document::remote(
                        buffer,
                        RemoteDocument {
                            file,
                            stamp: metadata.stamp,
                            connection: None,
                            return_to: metadata.return_to,
                        },
                    )
                }
                _ => {
                    return Err(serde::de::Error::custom(
                        "open completion source metadata mismatch; use the recording version",
                    ))
                }
            }
        };
        debug_assert!(matches!(
            document.source,
            DocumentSource::File
                | DocumentSource::Scratch
                | DocumentSource::Directory(_)
                | DocumentSource::Remote(_)
        ));
        Ok(Self {
            document,
            canonical: record.canonical,
        })
    }
}
