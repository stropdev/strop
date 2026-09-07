//! Only pure buffer data crosses the forensic boundary, never a live parser.
use super::Opened;
use crate::editor::Document;
use crate::files::FileTarget;
use serde::{Deserialize, Serialize};

impl Serialize for Opened {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Record<'a> {
            buffer: strop_core::BufferSeed,
            canonical: &'a FileTarget,
        }
        Record {
            buffer: self.document.buf.seed(),
            canonical: &self.canonical,
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Opened {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Record {
            buffer: strop_core::BufferSeed,
            canonical: FileTarget,
        }
        let record = Record::deserialize(deserializer)?;
        let buffer = record
            .buffer
            .into_buffer()
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            document: match &record.canonical {
                FileTarget::Local(_) => Document::new(buffer),
                FileTarget::Remote(file) => Document::remote(buffer, file.clone()),
            },
            canonical: record.canonical,
        })
    }
}
