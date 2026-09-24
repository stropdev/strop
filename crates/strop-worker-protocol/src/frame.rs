//! The byte boundary (0058 WK02): bounded Content-Length framing, the same
//! convention strop-ui-protocol (0056 AR09) and strop-lsp pin at their
//! boundaries (`Content-Length: N\r\n\r\n` + body). Header and body each
//! carry a hard bound and every violation is a typed [`FrameError`], never
//! a skipped-garbage resynchronization.
//!
//! The body bound is deliberately small: control envelopes are compact and
//! bulk payloads flow as bounded stream chunks ([`crate::codec`]), so one
//! frame never needs to carry a file. The bound exists to refuse, not to
//! budget.

use std::io::{self, Read, Write};

/// Header bytes accepted before the terminator must exist (the shared
/// decoder bound).
pub const MAX_HEADER_BYTES: usize = 8192;

/// Body bound: 1 MiB. Stream chunks are further bounded by
/// [`crate::codec::MAX_CHUNK_BYTES`]; control envelopes are far smaller.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// A framing violation. Frame-level corruption poisons the stream — the
/// next boundary is unknowable — so the peer reports this typed error
/// in-band and closes; it never guesses a resynchronization point.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("frame header exceeded the {limit}-byte bound without a terminator")]
    HeaderTooLarge { limit: usize },
    #[error("frame body length {actual} exceeds the {limit}-byte bound")]
    BodyTooLarge { limit: usize, actual: usize },
    #[error("frame header carries no valid Content-Length")]
    MissingLength,
    #[error("frame header line is not `Name: value`")]
    MalformedHeader,
    /// The stream ended with a partial header or body buffered.
    #[error("stream ended mid-frame")]
    Truncated,
}

/// Write one framed body.
pub fn write_frame(mut writer: impl Write, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

/// Incremental decoder over a byte stream: accepts arbitrary fragments,
/// yields complete bodies, and holds partial headers/bodies across reads.
/// Bounds trip before unbounded buffering.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when no partial frame is held — the only clean EOF point.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Append stream bytes. Fails (typed) the moment a header exceeds its
    /// bound; body bounds trip at header parse.
    pub fn accept(&mut self, bytes: &[u8]) -> Result<(), FrameError> {
        self.buffer.extend_from_slice(bytes);
        if !self.header_complete() && self.buffer.len() > MAX_HEADER_BYTES {
            return Err(FrameError::HeaderTooLarge {
                limit: MAX_HEADER_BYTES,
            });
        }
        Ok(())
    }

    /// Take one complete frame body, if one is buffered.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        let Some(header_end) = find(&self.buffer, b"\r\n\r\n") else {
            return Ok(None);
        };
        if header_end + 4 > MAX_HEADER_BYTES {
            return Err(FrameError::HeaderTooLarge {
                limit: MAX_HEADER_BYTES,
            });
        }
        let length = content_length(&self.buffer[..header_end])?;
        if length > MAX_BODY_BYTES {
            return Err(FrameError::BodyTooLarge {
                limit: MAX_BODY_BYTES,
                actual: length,
            });
        }
        let start = header_end + 4;
        if self.buffer.len() < start + length {
            return Ok(None);
        }
        let body = self.buffer[start..start + length].to_vec();
        self.buffer.drain(..start + length);
        Ok(Some(body))
    }

    fn header_complete(&self) -> bool {
        find(&self.buffer, b"\r\n\r\n").is_some()
    }
}

/// Blocking read of one frame: `Ok(None)` is a clean EOF at a frame
/// boundary; EOF mid-frame is `UnexpectedEof`. Typed violations surface as
/// `InvalidData` carrying the [`FrameError`] text — stream users that must
/// classify them drive `accept`/`next_frame` directly.
pub fn read_frame(
    reader: &mut impl Read,
    decoder: &mut FrameDecoder,
) -> io::Result<Option<Vec<u8>>> {
    let mut chunk = [0u8; 8192];
    loop {
        match decoder.next_frame() {
            Ok(Some(body)) => return Ok(Some(body)),
            Ok(None) => {}
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        }
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return if decoder.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream ended mid-frame",
                ))
            };
        }
        decoder
            .accept(&chunk[..read])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn content_length(header: &[u8]) -> Result<usize, FrameError> {
    let header = std::str::from_utf8(header).map_err(|_| FrameError::MalformedHeader)?;
    let mut length = None;
    for line in header.split("\r\n") {
        let (name, value) = line.split_once(':').ok_or(FrameError::MalformedHeader)?;
        if name.trim().is_empty() {
            return Err(FrameError::MalformedHeader);
        }
        if name.trim() == "Content-Length" {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| FrameError::MissingLength)?,
            );
        }
    }
    length.ok_or(FrameError::MissingLength)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(decoder: &mut FrameDecoder, bytes: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        decoder.accept(bytes)?;
        let mut out = Vec::new();
        while let Some(body) = decoder.next_frame()? {
            out.push(body);
        }
        Ok(out)
    }

    #[test]
    fn round_trip_one_frame() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"hello").unwrap();
        assert_eq!(bytes, b"Content-Length: 5\r\n\r\nhello");
        let mut decoder = FrameDecoder::new();
        assert_eq!(
            frames(&mut decoder, &bytes).unwrap(),
            vec![b"hello".to_vec()]
        );
        assert!(decoder.is_empty());
    }

    #[test]
    fn fragmented_and_coalesced_frames() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"ab").unwrap();
        write_frame(&mut bytes, b"cde").unwrap();
        let mut decoder = FrameDecoder::new();
        let mut out = Vec::new();
        for byte in bytes {
            out.extend(frames(&mut decoder, &[byte]).unwrap());
        }
        assert_eq!(out, vec![b"ab".to_vec(), b"cde".to_vec()]);
    }

    #[test]
    fn oversized_header_refused() {
        let mut decoder = FrameDecoder::new();
        let junk = vec![b'x'; MAX_HEADER_BYTES + 1];
        assert_eq!(
            decoder.accept(&junk),
            Err(FrameError::HeaderTooLarge {
                limit: MAX_HEADER_BYTES
            })
        );
    }

    #[test]
    fn oversized_body_refused_at_header_parse() {
        let header = format!("Content-Length: {}\r\n\r\n", MAX_BODY_BYTES + 1);
        let mut decoder = FrameDecoder::new();
        decoder.accept(header.as_bytes()).unwrap();
        assert_eq!(
            decoder.next_frame(),
            Err(FrameError::BodyTooLarge {
                limit: MAX_BODY_BYTES,
                actual: MAX_BODY_BYTES + 1,
            })
        );
    }
    #[test]
    fn malformed_headers_are_typed() {
        // No `Name: value` line at all.
        let mut decoder = FrameDecoder::new();
        assert_eq!(
            frames(&mut decoder, b"nonsense\r\n\r\nx"),
            Err(FrameError::MalformedHeader)
        );
        // A bare header name without a colon.
        let mut decoder = FrameDecoder::new();
        assert_eq!(
            frames(&mut decoder, b"Content-Length\r\n\r\nx"),
            Err(FrameError::MalformedHeader)
        );
        // A well-formed header that names no Content-Length.
        let mut decoder = FrameDecoder::new();
        assert_eq!(
            frames(&mut decoder, b"Content-Type: text\r\n\r\nx"),
            Err(FrameError::MissingLength)
        );
        // An unparseable Content-Length value.
        let mut decoder = FrameDecoder::new();
        assert_eq!(
            frames(&mut decoder, b"Content-Length: nope\r\n\r\nx"),
            Err(FrameError::MissingLength)
        );
    }

    #[test]
    fn clean_eof_only_at_boundary() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"partial-body").unwrap();
        bytes.truncate(bytes.len() - 3);
        let mut cursor = io::Cursor::new(bytes);
        let mut decoder = FrameDecoder::new();
        let error = read_frame(&mut cursor, &mut decoder).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);

        let mut cursor = io::Cursor::new(Vec::new());
        let mut decoder = FrameDecoder::new();
        assert_eq!(read_frame(&mut cursor, &mut decoder).unwrap(), None);
    }
}
