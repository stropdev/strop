//! Observe bytes actually accepted by the transport. JSON-RPC bodies are parsed
//! only when tracing; their text is never emitted, even in full-content mode.
use serde::Serialize;
use serde_json::Value;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use strop_trace::{record, EventKind};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Direction {
    Tx,
    Rx,
}

pub(super) struct Observed<T> {
    inner: T,
    decoder: Option<Decoder>,
}
impl<T> Observed<T> {
    pub fn new(inner: T, server: &str, direction: Direction) -> Self {
        Self {
            inner,
            decoder: strop_trace::enabled().then(|| Decoder::new(server, direction)),
        }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for Observed<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(context, buffer);
        if let Poll::Ready(Ok(())) = &result {
            if let Some(decoder) = &mut this.decoder {
                decoder.push(&buffer.filled()[before..]);
            }
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Observed<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write(context, bytes);
        if let Poll::Ready(Ok(written)) = &result {
            if let Some(decoder) = &mut this.decoder {
                decoder.push(&bytes[..*written]);
            }
        }
        result
    }
    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }
    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

#[derive(Debug, Serialize)]
struct Message {
    server: String,
    direction: Direction,
    payload_bytes: usize,
    id: Option<Value>,
    method: Option<String>,
    response: bool,
    error_code: Option<Value>,
    error_message: Option<String>,
    /// The full frame body, only under --log-content (0.21.0 field
    /// report: diagnosing an LSP failure meant reimplementing a client
    /// because the trace carried metadata but never the payload).
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<String>,
}

struct Decoder {
    server: String,
    direction: Direction,
    buffer: Vec<u8>,
    body_bytes: Option<usize>,
    omitted_bytes: usize,
}
impl Decoder {
    fn new(server: &str, direction: Direction) -> Self {
        Self {
            server: server.into(),
            direction,
            buffer: Vec::new(),
            body_bytes: None,
            omitted_bytes: 0,
        }
    }
    fn push(&mut self, bytes: &[u8]) {
        self.decode(bytes, |message| record(EventKind::LspMessage, &message));
    }

    fn decode(&mut self, mut bytes: &[u8], mut emit: impl FnMut(Message)) {
        while !bytes.is_empty() {
            if self.omitted_bytes > 0 {
                let skipped = self.omitted_bytes.min(bytes.len());
                self.omitted_bytes -= skipped;
                bytes = &bytes[skipped..];
                continue;
            }
            if let Some(length) = self.body_bytes {
                let take = (length - self.buffer.len()).min(bytes.len());
                self.buffer.extend_from_slice(&bytes[..take]);
                bytes = &bytes[take..];
                if self.buffer.len() == length {
                    match metadata(&self.buffer, &self.server, self.direction) {
                        Ok(message) => emit(message),
                        Err(error) => self.error(&format!("invalid JSON-RPC payload: {error}")),
                    }
                    self.buffer.clear();
                    self.body_bytes = None;
                }
            } else {
                self.buffer.push(bytes[0]);
                bytes = &bytes[1..];
                if self.buffer.ends_with(b"\r\n\r\n") {
                    let length = std::str::from_utf8(&self.buffer).ok().and_then(|header| {
                        header.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                    });
                    self.buffer.clear();
                    match length {
                        Some(length) if length > 0 && length <= 32 * 1024 * 1024 => {
                            self.body_bytes = Some(length)
                        }
                        Some(length) => {
                            self.error(
                                "LSP trace payload omitted: empty or exceeds 32 MiB capture bound",
                            );
                            self.omitted_bytes = length;
                        }
                        None => self.error("LSP frame has no valid Content-Length"),
                    }
                } else if self.buffer.len() > 8192 {
                    self.error("LSP trace header exceeds 8192-byte capture bound");
                    self.buffer.clear();
                }
            }
        }
    }
    fn error(&self, message: &str) {
        strop_trace::record_with(EventKind::Error, || {
            serde_json::json!({
                "source":"lsp_trace","server":self.server,"direction":self.direction,"message":message,
            })
        });
    }
}
fn metadata(
    bytes: &[u8],
    server: &str,
    direction: Direction,
) -> Result<Message, serde_json::Error> {
    let value: Value = serde_json::from_slice(bytes)?;
    Ok(Message {
        server: server.into(),
        direction,
        payload_bytes: bytes.len(),
        id: value.get("id").cloned(),
        method: value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string),
        response: value.get("result").is_some() || value.get("error").is_some(),
        error_code: value
            .get("error")
            .and_then(|error| error.get("code"))
            .cloned(),
        error_message: value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .map(strop_trace::preview),
        payload: strop_trace::capture_content()
            .then(|| String::from_utf8_lossy(bytes).into_owned()),
    })
}

/// Trace install is process-global; tests that start a Full-content
/// session or assert the default policy serialize on this.
#[cfg(test)]
pub(crate) static TRACE_SESSION: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_metadata_never_contains_text_payloads() {
        let _session = super::TRACE_SESSION.lock();
        let payload = br#"{"jsonrpc":"2.0","id":17,"method":"textDocument/didChange","params":{"secret":"private document"}}"#;
        let message = metadata(payload, "fake-language-server", Direction::Tx).unwrap();
        assert_eq!(message.method.as_deref(), Some("textDocument/didChange"));
        assert_eq!(message.payload_bytes, payload.len());
        assert_eq!(message.id, Some(Value::from(17)));
        assert!(!serde_json::to_string(&message)
            .unwrap()
            .contains("private document"));
    }
    #[test]
    fn split_and_concatenated_frames_emit_exact_response_metadata() {
        let payloads = [
            r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"backend failure"}}"#,
        ];
        let frames: String = payloads
            .iter()
            .map(|payload| format!("Content-Length: {}\r\n\r\n{payload}", payload.len()))
            .collect();
        for chunk_width in 1..frames.len() {
            let mut decoder = Decoder::new("server", Direction::Rx);
            let mut messages = Vec::new();
            for chunk in frames.as_bytes().chunks(chunk_width) {
                decoder.decode(chunk, |message| messages.push(message));
            }
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[0].id, Some(Value::from(1)));
            assert_eq!(messages[1].id, Some(Value::from(2)));
            assert_eq!(messages[1].error_code, Some(Value::from(-32603)));
            assert_eq!(
                messages[1].error_message.as_deref(),
                Some("backend failure")
            );
            assert_eq!(messages[1].payload_bytes, payloads[1].len());
        }
    }
}
