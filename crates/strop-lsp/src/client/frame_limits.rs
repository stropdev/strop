//! async-lsp allocates Content-Length bytes before reading a body. Enforce the
//! frame and JSON-structure budgets before its reader sees a complete header.
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

pub(super) const MAX_HEADER_BYTES: usize = 8192;
pub(super) const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const MAX_STRUCTURE_TOKENS: usize = 256 * 1024;

pub(super) struct BoundedFrames<T> {
    inner: T,
    guard: FrameGuard,
}
impl<T> BoundedFrames<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            guard: FrameGuard::default(),
        }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for BoundedFrames<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        match Pin::new(&mut this.inner).poll_read(context, buffer) {
            Poll::Ready(Ok(())) => {
                this.guard.push(&buffer.filled()[before..])?;
                Poll::Ready(Ok(()))
            }
            result => result,
        }
    }
}

#[derive(Default)]
struct FrameGuard {
    header: Vec<u8>,
    body: Option<Body>,
}
struct Body {
    remaining: usize,
    structures: usize,
    string: StringState,
}
#[derive(Clone, Copy)]
enum StringState {
    Outside,
    Quoted,
    Escape,
}

impl FrameGuard {
    fn push(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            if let Some(body) = &mut self.body {
                let take = bytes.len().min(body.remaining);
                for byte in &bytes[..take] {
                    match (body.string, *byte) {
                        (StringState::Outside, b'"') => body.string = StringState::Quoted,
                        (StringState::Quoted, b'\\') => body.string = StringState::Escape,
                        (StringState::Quoted, b'"') => body.string = StringState::Outside,
                        (StringState::Escape, _) => body.string = StringState::Quoted,
                        (StringState::Outside, b'{' | b'[' | b',' | b':') => {
                            body.structures += 1;
                            if body.structures > MAX_STRUCTURE_TOKENS {
                                return Err(invalid(
                                    "language-server JSON exceeds the structural allocation bound",
                                ));
                            }
                        }
                        _ => {}
                    }
                }
                body.remaining -= take;
                bytes = &bytes[take..];
                if body.remaining == 0 {
                    self.body = None;
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
                    self.header.clear();
                    self.body = Some(Body {
                        remaining: length,
                        structures: 0,
                        string: StringState::Outside,
                    });
                }
            }
        }
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn oversized_declarations_fail_at_the_header_without_waiting_for_body_bytes() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            for length in [MAX_FRAME_BYTES + 1, usize::MAX] {
                let header = format!("Content-Length: {length}\r\n\r\n");
                let mut reader = BoundedFrames::new(header.as_bytes());
                let mut received = [0; 128];
                let error = reader.read(&mut received).await.unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            }
        });
    }

    #[test]
    fn duplicate_lengths_cannot_bypass_the_allocation_bound() {
        for header in [
            "Content-Length: 1\r\nContent-Length: 33554433\r\n\r\n",
            "Content-Length: 33554433\r\nContent-Length: 1\r\n\r\n",
        ] {
            let mut guard = FrameGuard::default();
            let error = guard.push(header.as_bytes()).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn structural_floods_are_rejected_but_quoted_protocol_text_is_not_a_tree() {
        let quoted = serde_json::to_vec(
            &serde_json::json!({"result": "[,{}:\"".repeat(MAX_STRUCTURE_TOKENS)}),
        )
        .unwrap();
        let mut guard = FrameGuard::default();
        guard
            .push(format!("Content-Length: {}\r\n\r\n", quoted.len()).as_bytes())
            .unwrap();
        for chunk in quoted.chunks(127) {
            guard.push(chunk).unwrap();
        }
        // A subsequent frame resets its budget; the many real array entries
        // still refuse before async-lsp can allocate their Value tree.
        let flood = format!("[{}0]", "0,".repeat(MAX_STRUCTURE_TOKENS));
        guard
            .push(format!("Content-Length: {}\r\n\r\n", flood.len()).as_bytes())
            .unwrap();
        assert_eq!(
            guard.push(flood.as_bytes()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
