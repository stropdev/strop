//! async-lsp allocates Content-Length bytes before reading a body, then
//! decodes it into a `serde_json::Value` tree that costs far more than the
//! body (0101 §4.1). This reader holds each frame until its last byte is
//! counted and releases it whole, so a frame too dear to decode never
//! reaches the decoder: an over-budget reply is answered as an error for
//! its request, an over-budget notification is dropped, and only an
//! over-budget server request ends the connection.
use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

mod scan;
#[cfg(test)]
mod tests;

pub use scan::structure_tokens;
use scan::{Scan, Shape};

pub(super) const MAX_HEADER_BYTES: usize = 8192;
pub(super) const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// The heap one frame may cost to decode.
pub const MAX_DECODE_BYTES: usize = 512 * 1024 * 1024;
/// The decoder's worst measured cost of one structure token (371 bytes,
/// for `[{"a":0},…]`) with a tenth of margin; `tests/decode_cost.rs` holds
/// every adversarial shape under it.
pub const DECODE_BYTES_PER_TOKEN: usize = 408;
/// The most structure tokens a frame may carry: 1,315,860.
pub const MAX_DECODE_TOKENS: usize = MAX_DECODE_BYTES / DECODE_BYTES_PER_TOKEN;

/// JSON-RPC's internal error, the code an unreadable reply is answered with.
const INTERNAL_ERROR: i32 = -32603;

/// Called once per connection, with the token count of the first
/// notification dropped for being over the budget.
type Report = Box<dyn FnMut(usize) + Send>;

pub(super) struct BoundedFrames<T> {
    inner: T,
    guard: FrameGuard,
    report: Option<Report>,
}
impl<T> BoundedFrames<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            guard: FrameGuard::default(),
            report: None,
        }
    }

    /// Say once that a notification over the budget was dropped.
    pub fn reporting(mut self, report: impl FnMut(usize) + Send + 'static) -> Self {
        self.report = Some(Box::new(report));
        self
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for BoundedFrames<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buffer.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if this.guard.drain(buffer) {
                return Poll::Ready(Ok(()));
            }
            // Read into the caller's spare room, take the bytes, give the
            // room back: the frame is released only when it is whole.
            let before = buffer.filled().len();
            ready!(Pin::new(&mut this.inner).poll_read(context, buffer))?;
            if buffer.filled().len() == before {
                return Poll::Ready(if this.guard.mid_frame() {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "language-server output ended inside a frame",
                    ))
                } else {
                    Ok(())
                });
            }
            let pushed = this.guard.push(&buffer.filled()[before..]);
            buffer.set_filled(before);
            pushed?;
            if let (Some(tokens), Some(report)) = (this.guard.take_report(), &mut this.report) {
                report(tokens);
            }
        }
    }
}

#[derive(Default)]
struct FrameGuard {
    header: Vec<u8>,
    body: Option<Body>,
    /// Whole frames waiting for the reader, the first `at` bytes of the
    /// front one already given.
    ready: VecDeque<Vec<u8>>,
    at: usize,
    dropped: usize,
    report: Option<usize>,
}
struct Body {
    /// Header and body so far; emptied once the frame is over the budget,
    /// since it will never be released.
    frame: Vec<u8>,
    remaining: usize,
    scan: Scan,
}

impl FrameGuard {
    fn push(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            if let Some(body) = &mut self.body {
                let take = bytes.len().min(body.remaining);
                let chunk = &bytes[..take];
                let within = body.scan.tokens() <= MAX_DECODE_TOKENS;
                body.scan.push(chunk);
                if body.scan.tokens() <= MAX_DECODE_TOKENS {
                    body.frame.extend_from_slice(chunk);
                } else if within {
                    body.frame = Vec::new();
                }
                body.remaining -= take;
                bytes = &bytes[take..];
                if body.remaining == 0 {
                    if let Some(body) = self.body.take() {
                        self.finish(body)?;
                    }
                }
            } else {
                if self.header.len() == MAX_HEADER_BYTES {
                    return Err(invalid("language-server header exceeds 8192 bytes"));
                }
                self.header.push(bytes[0]);
                bytes = &bytes[1..];
                if self.header.ends_with(b"\r\n\r\n") {
                    let length = content_length(&self.header)?;
                    if length > MAX_FRAME_BYTES {
                        return Err(invalid(
                            "language-server frame exceeds the 32 MiB allocation bound",
                        ));
                    }
                    let mut frame = std::mem::take(&mut self.header);
                    frame.reserve_exact(length);
                    self.body = Some(Body {
                        frame,
                        remaining: length,
                        scan: Scan::default(),
                    });
                }
            }
        }
        Ok(())
    }

    /// A frame's last byte is in: release it, or what stands for it.
    fn finish(&mut self, body: Body) -> io::Result<()> {
        let tokens = body.scan.tokens();
        if tokens <= MAX_DECODE_TOKENS {
            self.ready.push_back(body.frame);
            return Ok(());
        }
        match body.scan.shape() {
            Shape::Response(id) => {
                self.ready.push_back(too_large(&id, tokens));
                Ok(())
            }
            Shape::Notification => {
                strop_trace::record_with(strop_trace::EventKind::Error, || {
                    serde_json::json!({
                        "source": "lsp_frame",
                        "message": "notification over the decode budget dropped",
                        "tokens": tokens,
                        "limit": MAX_DECODE_TOKENS,
                    })
                });
                if self.dropped == 0 {
                    self.report = Some(tokens);
                }
                self.dropped += 1;
                Ok(())
            }
            Shape::Request => Err(invalid(
                "language-server request exceeds the 512 MiB decode budget",
            )),
        }
    }

    /// Give the reader what is ready; false when nothing is.
    fn drain(&mut self, buffer: &mut ReadBuf<'_>) -> bool {
        let Some(front) = self.ready.front() else {
            return false;
        };
        let given = buffer.remaining().min(front.len() - self.at);
        buffer.put_slice(&front[self.at..self.at + given]);
        self.at += given;
        if self.at == front.len() {
            self.ready.pop_front();
            self.at = 0;
        }
        true
    }

    fn mid_frame(&self) -> bool {
        !self.header.is_empty() || self.body.is_some()
    }

    fn take_report(&mut self) -> Option<usize> {
        self.report.take()
    }
}

/// The error reply that stands for a reply too large to decode: the
/// request fails, the connection lives.
fn too_large(id: &serde_json::Value, tokens: usize) -> Vec<u8> {
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"error\":{{\"code\":{INTERNAL_ERROR},\
         \"message\":\"reply too large to decode: {tokens} tokens, limit {MAX_DECODE_TOKENS} \
         (512 MiB)\"}}}}"
    );
    let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    frame.extend_from_slice(body.as_bytes());
    frame
}

pub(super) fn content_length(header: &[u8]) -> io::Result<usize> {
    let header = std::str::from_utf8(header)
        .map_err(|_| invalid("invalid language-server header encoding"))?;
    let mut length = None;
    for line in header.split("\r\n").filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(": ")
            .ok_or_else(|| invalid("invalid language-server header"))?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err(invalid("duplicate language-server Content-Length"));
            }
            let value = value
                .parse::<usize>()
                .map_err(|_| invalid("invalid language-server Content-Length"))?;
            if value == 0 {
                return Err(invalid("LSP frame is empty"));
            }
            length = Some(value);
        }
    }
    length.ok_or_else(|| invalid("language-server frame has no Content-Length"))
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
