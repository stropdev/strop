//! Bounded retention of ssh(1) diagnostics, drained concurrently with the
//! SFTP session. Only the head is kept: ssh's actionable line comes first.

use parking_lot::Mutex;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::ChildStderr;
use tokio::task::JoinHandle;

/// Enough for ssh's verbose failure output, small enough to be harmless.
const RETENTION: usize = 64 * 1024;

/// The retained head of a diagnostic stream. Lossy rendering is
/// deliberate: this is stderr text, never buffer content.
#[derive(Debug, Default)]
pub(super) struct StderrTail {
    head: Vec<u8>,
    dropped: usize,
}

impl StderrTail {
    pub(super) fn retain(&mut self, chunk: &[u8]) {
        let room = RETENTION.saturating_sub(self.head.len());
        if room == 0 {
            self.dropped += chunk.len();
            return;
        }
        match chunk.len().checked_sub(room) {
            None => self.head.extend_from_slice(chunk),
            Some(overflow) => {
                self.head.extend_from_slice(&chunk[..room]);
                self.dropped += overflow;
            }
        }
    }

    pub(super) fn render(&self) -> Option<String> {
        if self.head.is_empty() {
            return None;
        }
        let mut text = String::from_utf8_lossy(&self.head).into_owned();
        if self.dropped > 0 {
            text.push_str(&format!(
                "\n[{} further stderr bytes dropped past the retention cap]",
                self.dropped
            ));
        }
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    }
}

pub(super) type SharedTail = Arc<Mutex<StderrTail>>;

/// Drain the child's stderr concurrently with the session. The task
/// completes at EOF — when ssh and its local descendants are gone — and
/// everything before that lands in the shared tail.
pub(super) fn spawn_drain(pipe: ChildStderr, log: SharedTail) -> JoinHandle<()> {
    tokio::task::spawn(async move {
        let mut pipe = pipe;
        let mut chunk = [0u8; 4096];
        loop {
            match pipe.read(&mut chunk).await {
                Ok(0) => return,
                Ok(count) => log.lock().retain(&chunk[..count]),
                // A dying child can race the final read; whatever was
                // retained already is the honest diagnostic.
                Err(_) => return,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_is_kept_and_overflow_counted() {
        let mut tail = StderrTail::default();
        let big: Vec<u8> = vec![b'x'; RETENTION + 100];
        tail.retain(&big);
        assert_eq!(tail.head.len(), RETENTION);
        assert_eq!(tail.dropped, 100);
        let text = tail.render().expect("retained text");
        assert!(text.starts_with('x'));
        assert!(text.contains("100 further stderr bytes dropped"));
    }

    #[test]
    fn empty_tail_renders_nothing() {
        assert!(StderrTail::default().render().is_none());
        let whitespace = StderrTail {
            head: b" \n".to_vec(),
            dropped: 0,
        };
        assert!(whitespace.render().is_none());
    }

    #[test]
    fn chunks_accumulate_within_the_cap() {
        let mut tail = StderrTail::default();
        tail.retain(b"Permission denied (publickey).\n");
        tail.retain(b"second line\n");
        assert_eq!(tail.dropped, 0);
        assert_eq!(
            tail.render().as_deref(),
            Some("Permission denied (publickey).\nsecond line")
        );
    }
}
