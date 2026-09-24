//! Frame body codec (0058 WK02): one byte of class, then the payload.
//!
//! - `Envelope` (class 0): a UTF-8 JSON control message —
//!   [`ClientMessage`]/[`WorkerMessage`]. Paths and argv inside envelopes
//!   are byte-exact (the workspace path_serde convention / bounded byte
//!   strings), never lossy UTF-8.
//! - `Chunk` (class 1): a binary [`StreamChunk`] — every bulk byte
//!   payload (file reads, frozen save content, process stdio, VT bytes)
//!   flows this way, bounded per chunk, never as a giant JSON array,
//!   base64 copy or full-response buffer.
//!
//! Chunk binary layout, little-endian:
//! `[class=1][stream: u64][sequence: u64][flags: u8][bytes…]`.
//! `flags` bit 0 is `last` (half-close of that stream direction); all
//! other bits are reserved and their presence is a typed violation.

use std::io::{self, Write};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::frame;
use crate::id::StreamId;

/// One stream chunk's payload ceiling (the SFTP codec's packet bound,
/// carried over). Streams of any length flow as bounded chunks.
pub const MAX_CHUNK_BYTES: usize = 256 * 1024;

const CLASS_ENVELOPE: u8 = 0;
const CLASS_CHUNK: u8 = 1;
const FLAG_LAST: u8 = 1;
const CHUNK_HEADER: usize = 1 + 8 + 8 + 1;

/// One bounded payload chunk on one stream. `last` half-closes the
/// stream direction: no further chunks follow, and the terminal status
/// (read end, process exit) arrives separately as a typed envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChunk {
    pub stream: StreamId,
    pub sequence: u64,
    pub last: bool,
    pub bytes: Vec<u8>,
}

/// A decoded frame body, either direction.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming<E> {
    Envelope(E),
    Chunk(StreamChunk),
}

/// A body-codec violation, all typed. [`CodecError::Chunk`] violations
/// are protocol failures (they cannot be data); the session reports them
/// in-band and closes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("empty frame body")]
    EmptyBody,
    #[error("unknown frame body class {0}")]
    UnknownClass(u8),
    #[error("stream chunk truncated: header needs {need} bytes, body has {actual}")]
    ChunkTruncated { need: usize, actual: usize },
    #[error("stream chunk payload {actual} exceeds the {limit}-byte bound")]
    ChunkTooLarge { limit: usize, actual: usize },
    #[error("stream chunk carries reserved flags {0:#04x}")]
    UnknownFlags(u8),
    #[error("envelope is not valid JSON: {0}")]
    Json(String),
}

/// Serialize one control envelope into a framed body and write it.
pub fn write_envelope(writer: impl Write, message: &impl Serialize) -> io::Result<()> {
    let json = serde_json::to_vec(message)?;
    let mut body = Vec::with_capacity(json.len() + 1);
    body.push(CLASS_ENVELOPE);
    body.extend_from_slice(&json);
    frame::write_frame(writer, &body)
}

/// Encode and write one bounded stream chunk.
pub fn write_chunk(writer: impl Write, chunk: &StreamChunk) -> io::Result<()> {
    if chunk.bytes.len() > MAX_CHUNK_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            CodecError::ChunkTooLarge {
                limit: MAX_CHUNK_BYTES,
                actual: chunk.bytes.len(),
            },
        ));
    }
    let mut body = Vec::with_capacity(CHUNK_HEADER + chunk.bytes.len());
    body.push(CLASS_CHUNK);
    body.extend_from_slice(&chunk.stream.0.to_le_bytes());
    body.extend_from_slice(&chunk.sequence.to_le_bytes());
    body.push(if chunk.last { FLAG_LAST } else { 0 });
    body.extend_from_slice(&chunk.bytes);
    frame::write_frame(writer, &body)
}

/// Decode one frame body. `E` is the envelope type for this direction
/// ([`crate::request::ClientMessage`] on the worker side,
/// [`crate::request::WorkerMessage`] on the client side).
pub fn decode_body<E: DeserializeOwned>(body: &[u8]) -> Result<Incoming<E>, CodecError> {
    let (&class, payload) = body.split_first().ok_or(CodecError::EmptyBody)?;
    match class {
        CLASS_ENVELOPE => {
            let envelope = serde_json::from_slice(payload)
                .map_err(|error| CodecError::Json(error.to_string()))?;
            Ok(Incoming::Envelope(envelope))
        }
        CLASS_CHUNK => decode_chunk(payload).map(Incoming::Chunk),
        unknown => Err(CodecError::UnknownClass(unknown)),
    }
}

fn decode_chunk(payload: &[u8]) -> Result<StreamChunk, CodecError> {
    let need = CHUNK_HEADER - 1;
    if payload.len() < need {
        return Err(CodecError::ChunkTruncated {
            need,
            actual: payload.len(),
        });
    }
    let truncated = || CodecError::ChunkTruncated {
        need,
        actual: payload.len(),
    };
    let stream = u64::from_le_bytes(payload[..8].try_into().map_err(|_| truncated())?);
    let sequence = u64::from_le_bytes(payload[8..16].try_into().map_err(|_| truncated())?);
    let bytes = &payload[need..];
    if bytes.len() > MAX_CHUNK_BYTES {
        return Err(CodecError::ChunkTooLarge {
            limit: MAX_CHUNK_BYTES,
            actual: bytes.len(),
        });
    }
    let flags = payload[16];
    if flags & !FLAG_LAST != 0 {
        return Err(CodecError::UnknownFlags(flags));
    }
    Ok(StreamChunk {
        stream: StreamId(stream),
        sequence,
        last: flags & FLAG_LAST != 0,
        bytes: bytes.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::LeaseId;
    use crate::id::Session;
    use crate::request::{ClientMessage, Request};

    #[test]
    fn chunk_binary_shape_is_pinned() {
        let chunk = StreamChunk {
            stream: StreamId(0x0102),
            sequence: 7,
            last: true,
            bytes: vec![0xde, 0xad],
        };
        let mut frame_bytes = Vec::new();
        write_chunk(&mut frame_bytes, &chunk).unwrap();
        let body = b"\x01\x02\x01\0\0\0\0\0\0\x07\0\0\0\0\0\0\0\x01\xde\xad";
        let mut expected = Vec::new();
        frame::write_frame(&mut expected, body).unwrap();
        assert_eq!(frame_bytes, expected);
        match decode_body::<serde_json::Value>(body).unwrap() {
            Incoming::Chunk(back) => assert_eq!(back, chunk),
            Incoming::Envelope(_) => panic!("chunk decoded as envelope"),
        }
    }

    #[test]
    fn chunk_bounds_and_flags_are_typed() {
        let oversized = StreamChunk {
            stream: StreamId(1),
            sequence: 0,
            last: false,
            bytes: vec![0; MAX_CHUNK_BYTES + 1],
        };
        let error = write_chunk(Vec::new(), &oversized).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

        // Reserved flags set: class byte, 16-byte stream/sequence, flags.
        let mut body = vec![CLASS_CHUNK];
        body.extend_from_slice(&[0u8; 16]);
        body.push(0x02);
        assert_eq!(
            decode_body::<serde_json::Value>(&body),
            Err(CodecError::UnknownFlags(2))
        );
        // Truncated header.
        assert_eq!(
            decode_body::<serde_json::Value>(b"\x01\x01\0"),
            Err(CodecError::ChunkTruncated {
                need: CHUNK_HEADER - 1,
                actual: 2,
            })
        );
        // Unknown class and empty body.
        assert_eq!(
            decode_body::<serde_json::Value>(b"\x07"),
            Err(CodecError::UnknownClass(7))
        );
        assert_eq!(
            decode_body::<serde_json::Value>(b""),
            Err(CodecError::EmptyBody)
        );
    }

    #[test]
    fn envelope_round_trip_with_bytes_at_bound() {
        let session = Session {
            incarnation: 1,
            lease: LeaseId(1),
        };
        let message = ClientMessage::Request {
            session,
            id: crate::id::RequestId(5),
            body: Box::new(Request::Health),
        };
        let mut bytes = Vec::new();
        write_envelope(&mut bytes, &message).unwrap();
        let mut cursor = io::Cursor::new(bytes);
        let mut decoder = frame::FrameDecoder::new();
        let body = frame::read_frame(&mut cursor, &mut decoder)
            .unwrap()
            .unwrap();
        match decode_body::<ClientMessage>(&body).unwrap() {
            Incoming::Envelope(back) => assert_eq!(back, message),
            Incoming::Chunk(_) => panic!("envelope decoded as chunk"),
        }

        // A chunk exactly at the bound is admitted.
        let chunk = StreamChunk {
            stream: StreamId(2),
            sequence: 0,
            last: false,
            bytes: vec![0x5a; MAX_CHUNK_BYTES],
        };
        let mut bytes = Vec::new();
        write_chunk(&mut bytes, &chunk).unwrap();
        let body_start = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("header terminator")
            + 4;
        match decode_body::<serde_json::Value>(&bytes[body_start..]).unwrap() {
            Incoming::Chunk(back) => assert_eq!(back, chunk),
            Incoming::Envelope(_) => panic!("chunk decoded as envelope"),
        }
    }
}
