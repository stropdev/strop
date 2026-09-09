//! Request text is an owned rope slice. Capturing/cloning a long line is O(log N),
//! and UTF-16 conversion runs on the wire worker. The recorded format stays text.
use crate::protocol::{PositionEncoding, ServerColumn};
use ropey::{Rope, RopeSlice};
use serde::{Deserialize, Serialize};
use strop_core::id::ByteColumn;

#[derive(Debug, Clone)]
pub struct FrozenLine(Rope);
impl FrozenLine {
    pub fn from_slice(line: RopeSlice<'_>) -> Self {
        Self(Rope::from(line))
    }
    pub fn as_slice(&self) -> RopeSlice<'_> {
        self.0.slice(..)
    }
}
impl From<&str> for FrozenLine {
    fn from(text: &str) -> Self {
        Self(Rope::from_str(text))
    }
}
impl From<String> for FrozenLine {
    fn from(text: String) -> Self {
        Self::from(text.as_str())
    }
}
impl Serialize for FrozenLine {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for FrozenLine {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}

pub fn to_server_col_slice(
    line: RopeSlice<'_>,
    byte_col: ByteColumn,
    encoding: PositionEncoding,
) -> ServerColumn {
    ServerColumn::new(match encoding {
        PositionEncoding::Utf8 => byte_col.get(),
        PositionEncoding::Utf16 => {
            let byte = byte_col.get();
            let end =
                if byte <= line.len_bytes() && line.char_to_byte(line.byte_to_char(byte)) == byte {
                    byte
                } else {
                    line.len_bytes()
                };
            line.byte_slice(..end).chars().map(char::len_utf16).sum()
        }
    })
}
pub fn to_byte_col_slice(
    line: RopeSlice<'_>,
    server_col: ServerColumn,
    encoding: PositionEncoding,
) -> ByteColumn {
    if encoding == PositionEncoding::Utf8 {
        return ByteColumn::new(server_col.get());
    }
    let mut units = 0;
    let mut byte = 0;
    for character in line.chars() {
        if units >= server_col.get() {
            return ByteColumn::new(byte);
        }
        units += character.len_utf16();
        byte += character.len_utf8();
    }
    ByteColumn::new(byte)
}
