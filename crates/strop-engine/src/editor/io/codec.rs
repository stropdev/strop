//! Pure completion data crosses replay; parsers, clients and leases never do.
use super::Opened;
use crate::editor::document::{DocumentSource, JumpRecord, RemoteDirectory, RemoteDocument};
use crate::editor::Document;
use crate::files::FileTarget;
use serde::{Deserialize, Serialize};
use strop_remote::{ReadSelection, RemoteEntry, RemoteWindow};

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum RemoteRecord {
    File {
        window: RemoteWindow,
        selection: ReadSelection,
        return_to: Option<JumpRecord>,
    },
    Directory {
        entries: Vec<RemoteEntry>,
        visible: Vec<usize>,
        filter: String,
        return_to: Option<JumpRecord>,
    },
}
#[derive(Deserialize)]
struct Record {
    buffer: strop_core::BufferSeed,
    canonical: FileTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<RemoteRecord>,
}

#[derive(Serialize)]
#[serde(tag = "kind")]
enum RemoteRecordRef<'a> {
    File {
        window: RemoteWindow,
        selection: ReadSelection,
        return_to: &'a Option<JumpRecord>,
    },
    Directory {
        entries: &'a [RemoteEntry],
        visible: &'a [usize],
        filter: &'a str,
        return_to: &'a Option<JumpRecord>,
    },
}
#[derive(Serialize)]
struct RecordRef<'a> {
    buffer: strop_core::BufferSeed,
    canonical: &'a FileTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote: Option<RemoteRecordRef<'a>>,
}
impl Serialize for Opened {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let remote = match &self.document.source {
            DocumentSource::Remote(source) => Some(RemoteRecordRef::File {
                window: source.window,
                selection: source.selection,
                return_to: &source.return_to,
            }),
            DocumentSource::RemoteDirectory(source) => Some(RemoteRecordRef::Directory {
                entries: &source.entries,
                visible: &source.visible,
                filter: &source.filter,
                return_to: &source.return_to,
            }),
            _ => None,
        };
        RecordRef {
            buffer: self.document.buf.seed(),
            canonical: &self.canonical,
            remote,
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
        let document = match (&record.canonical, record.remote) {
            (FileTarget::Local(_), None) => Document::new(buffer),
            (FileTarget::Remote(location), Some(metadata)) => {
                if buffer.path.is_some() || buffer.dirty {
                    return Err(serde::de::Error::custom(
                        "remote completion has local or dirty content",
                    ));
                }
                let file = location.absolute_file().cloned().ok_or_else(|| {
                    serde::de::Error::custom(
                        "remote completion must have a canonical absolute identity",
                    )
                })?;
                match metadata {
                    RemoteRecord::File {
                        window,
                        selection,
                        return_to,
                    } => {
                        if window.length().get() != buffer.len_bytes() as u64
                            || window
                                .start()
                                .get()
                                .checked_add(window.length().get())
                                .is_none_or(|end| end > window.file_size().get())
                        {
                            return Err(serde::de::Error::custom(
                                "remote window does not match buffer bytes",
                            ));
                        }
                        Document::remote(
                            buffer,
                            RemoteDocument {
                                file,
                                window,
                                selection,
                                connection: None,
                                return_to,
                                write: None,
                            },
                        )
                    }
                    RemoteRecord::Directory {
                        entries,
                        visible,
                        filter,
                        return_to,
                    } => {
                        let mut indices = std::collections::HashSet::new();
                        if visible
                            .iter()
                            .any(|&index| index >= entries.len() || !indices.insert(index))
                            || entries.iter().any(|entry| {
                                entry.file.endpoint() != file.endpoint()
                                    || entry.file.path().parent() != Some(file.path())
                            })
                        {
                            return Err(serde::de::Error::custom(
                                "invalid remote directory entry mapping",
                            ));
                        }
                        let source = RemoteDirectory {
                            directory: file,
                            entries: entries.into(),
                            visible,
                            filter,
                            connection: None,
                            return_to,
                        };
                        if buffer.text() != source.text().as_str() {
                            return Err(serde::de::Error::custom(
                                "directory rows disagree with their native targets",
                            ));
                        }
                        Document::directory(buffer, source)
                    }
                }
            }
            _ => {
                return Err(serde::de::Error::custom(
                    "open completion source metadata mismatch; use the recording version",
                ))
            }
        };
        Ok(Self {
            document,
            canonical: record.canonical,
        })
    }
}
