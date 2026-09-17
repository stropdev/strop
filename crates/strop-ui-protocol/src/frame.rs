//! The byte boundary (0056 AR09): bounded Content-Length framing, the
//! same convention strop-lsp pins at its trace boundary
//! (`Content-Length: N\r\n\r\n` + JSON body). Header and body each carry
//! a hard bound — the body bound is AR06's 32 MiB snapshot ceiling — and
//! every violation is a typed [`FrameError`], never a skipped-garbage
//! resynchronization.

use std::io::{self, Read, Write};

/// Header bytes accepted before the terminator must exist (the LSP
/// trace decoder's bound).
pub const MAX_HEADER_BYTES: usize = 8192;

/// Body bound: AR06's snapshot ceiling. View windows are far smaller;
/// the bound exists to refuse, not to budget.
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

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

/// Serialize and write one message.
pub fn write_message(writer: impl Write, message: &impl serde::Serialize) -> io::Result<()> {
    write_frame(writer, &serde_json::to_vec(message)?)
}

/// Incremental decoder over a byte stream: accepts arbitrary fragments,
/// yields complete bodies, and holds partial headers/bodies across
/// reads. Bounds trip before unbounded buffering.
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

    /// Append stream bytes. Fails (typed) the moment a header exceeds
    /// its bound; body bounds trip at header parse.
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
/// boundary; EOF mid-frame is `UnexpectedEof`. Typed violations surface
/// as `InvalidData` carrying the [`FrameError`] text — stream users that
/// must classify them drive `accept`/`next_frame` directly.
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

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, body).unwrap();
        bytes
    }

    #[test]
    fn roundtrip_whole_and_concatenated() {
        let mut bytes = frame(b"first");
        bytes.extend(frame(b"second"));
        let mut decoder = FrameDecoder::new();
        decoder.accept(&bytes).unwrap();
        assert_eq!(
            decoder.next_frame().unwrap().as_deref(),
            Some(&b"first"[..])
        );
        assert_eq!(
            decoder.next_frame().unwrap().as_deref(),
            Some(&b"second"[..])
        );
        assert!(decoder.next_frame().unwrap().is_none());
        assert!(decoder.is_empty());
    }

    #[test]
    fn partial_io_reassembles_byte_by_byte() {
        let bytes = frame(b"{\"hello\":\"world\"}");
        let mut decoder = FrameDecoder::new();
        let mut frames = Vec::new();
        for byte in bytes {
            decoder.accept(&[byte]).unwrap();
            while let Some(body) = decoder.next_frame().unwrap() {
                frames.push(body);
            }
        }
        assert_eq!(frames, vec![b"{\"hello\":\"world\"}".to_vec()]);
    }

    #[test]
    fn extra_headers_are_ignored_like_lsp() {
        let mut bytes = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n".to_vec();
        bytes.extend(frame(b"body"));
        // splice: header above plus the framed one's own header+body
        let mut decoder = FrameDecoder::new();
        decoder.accept(&bytes).unwrap();
        assert_eq!(decoder.next_frame().unwrap().as_deref(), Some(&b"body"[..]));
    }

    #[test]
    fn oversized_header_is_typed_before_unbounded_buffering() {
        let mut decoder = FrameDecoder::new();
        let error = decoder
            .accept(&vec![b'x'; MAX_HEADER_BYTES + 1])
            .unwrap_err();
        assert_eq!(
            error,
            FrameError::HeaderTooLarge {
                limit: MAX_HEADER_BYTES
            }
        );
    }

    #[test]
    fn oversized_body_is_typed_at_the_header() {
        let declared = MAX_BODY_BYTES + 1;
        let mut decoder = FrameDecoder::new();
        decoder
            .accept(format!("Content-Length: {declared}\r\n\r\n").as_bytes())
            .unwrap();
        assert_eq!(
            decoder.next_frame().unwrap_err(),
            FrameError::BodyTooLarge {
                limit: MAX_BODY_BYTES,
                actual: declared
            }
        );
    }

    #[test]
    fn missing_and_malformed_headers_are_typed() {
        let mut decoder = FrameDecoder::new();
        decoder.accept(b"Content-Type: text\r\n\r\n").unwrap();
        assert_eq!(decoder.next_frame().unwrap_err(), FrameError::MissingLength);

        let mut decoder = FrameDecoder::new();
        decoder.accept(b"no-colon-here\r\n\r\n").unwrap();
        assert_eq!(
            decoder.next_frame().unwrap_err(),
            FrameError::MalformedHeader
        );

        let mut decoder = FrameDecoder::new();
        decoder.accept(b"Content-Length: many\r\n\r\n").unwrap();
        assert_eq!(decoder.next_frame().unwrap_err(), FrameError::MissingLength);
    }

    #[test]
    fn blocking_read_distinguishes_clean_eof_from_truncation() {
        let mut bytes = frame(b"done");
        let mut decoder = FrameDecoder::new();
        let mut cursor = io::Cursor::new(bytes.clone());
        assert_eq!(
            read_frame(&mut cursor, &mut decoder).unwrap().as_deref(),
            Some(&b"done"[..])
        );
        assert!(read_frame(&mut cursor, &mut decoder).unwrap().is_none());

        bytes.truncate(bytes.len() - 2); // body cut short
        let mut decoder = FrameDecoder::new();
        let mut cursor = io::Cursor::new(bytes);
        let error = read_frame(&mut cursor, &mut decoder).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }
}
