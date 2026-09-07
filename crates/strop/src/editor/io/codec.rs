//! Only pure buffer data crosses the forensic boundary, never a live parser.
use super::Opened;
use crate::editor::Document;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

impl Serialize for Opened {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Record<'a> {
            buffer: strop_core::BufferSeed,
            #[serde(with = "strop_core::path_serde")]
            canonical: &'a PathBuf,
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
            #[serde(with = "strop_core::path_serde")]
            canonical: PathBuf,
        }
        let record = Record::deserialize(deserializer)?;
        let buffer = record
            .buffer
            .into_buffer()
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            document: Document::new(buffer),
            canonical: record.canonical,
        })
    }
}
