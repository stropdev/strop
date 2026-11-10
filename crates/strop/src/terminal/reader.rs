//! The input threads: tty bytes through the [`Decoder`], and SIGWINCH into
//! a size report. Both block on the OS only — never the UI thread — and park
//! against backpressure instead of dropping input (0056 AR06).
//!
//! Input comes from `/dev/tty`, not stdin, so `cmd | strop -` keeps a
//! keyboard while stdin carries the document: a third thread streams
//! stdin into the editor as it arrives (`tail -f log | strop -`).
use super::decode::{Decoded, Decoder};
use crate::editor::events::{AppEvent, EventSender};
use std::io::{self, Read};
use strop_core::frontend_input::Input;
use strop_trace::{record_with, EventKind};

/// Large enough that one read holds any burst a terminal writes at once;
/// a read that fills it says more bytes are already waiting.
const READ_BYTES: usize = 4096;

pub(super) fn spawn(sender: EventSender) -> io::Result<()> {
    let tty = std::fs::File::open("/dev/tty")?;
    let mut signals = signal_hook::iterator::Signals::new([signal_hook::consts::SIGWINCH])?;
    let resizes = sender.clone();
    std::thread::spawn(move || {
        for _ in signals.forever() {
            let Ok((columns, rows)) = crossterm::terminal::size() else {
                continue;
            };
            record_with(
                EventKind::Resize,
                || serde_json::json!({"columns":columns,"rows":rows}),
            );
            if resizes
                .send_blocking(AppEvent::Resize { columns, rows })
                .is_err()
            {
                return;
            }
        }
    });
    std::thread::spawn(move || read_keys(tty, sender));
    Ok(())
}

/// `strop -`: forward stdin as text chunks, then its end. Chunks split
/// only at character boundaries; invalid bytes become U+FFFD.
pub(super) fn spawn_stdin(sender: EventSender) {
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut bytes = [0u8; READ_BYTES];
        let mut pending = Vec::new();
        loop {
            let read = match stdin.read(&mut bytes) {
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    record_with(
                        EventKind::Error,
                        || serde_json::json!({"source":"stdin_read","message":error.to_string()}),
                    );
                    0
                }
            };
            pending.extend_from_slice(&bytes[..read]);
            let text = take_text(&mut pending, read == 0);
            if !text.is_empty() && sender.send_blocking(AppEvent::Stdin(Some(text))).is_err() {
                return;
            }
            if read == 0 {
                let _ = sender.send_blocking(AppEvent::Stdin(None));
                return;
            }
        }
    });
}

/// The decodable prefix of `pending`, holding back a character cut by
/// the read boundary unless the stream ended.
fn take_text(pending: &mut Vec<u8>, end: bool) -> String {
    let mut cut = pending.len();
    if !end {
        // a lead byte within the last three whose sequence is unfinished
        for back in 1..=pending.len().min(3) {
            let byte = pending[pending.len() - back];
            if byte & 0xc0 == 0x80 {
                continue;
            }
            let width = match byte {
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => 1,
            };
            if width > back {
                cut = pending.len() - back;
            }
            break;
        }
    }
    let text = String::from_utf8_lossy(&pending[..cut]).into_owned();
    pending.drain(..cut);
    text
}

fn read_keys(mut tty: std::fs::File, sender: EventSender) {
    let mut decoder = Decoder::default();
    let mut bytes = [0u8; READ_BYTES];
    let mut decoded = Vec::new();
    loop {
        let read = match tty.read(&mut bytes) {
            Ok(0) => return,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                record_with(
                    EventKind::Error,
                    || serde_json::json!({"source":"terminal_read","message":error.to_string()}),
                );
                return;
            }
        };
        decoder.feed(&bytes[..read], read == bytes.len(), &mut decoded);
        for event in decoded.drain(..) {
            let event = match event {
                Decoded::Key(key) => AppEvent::Input(Input::Key(key)),
                Decoded::Paste(text) => AppEvent::Input(Input::Paste(text)),
                Decoded::Focus(focused) => AppEvent::Focus(focused),
            };
            // FIFO input ownership is resolved by the engine, never here.
            if sender.send_blocking(event).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stdin_chunks_split_only_between_characters() {
        let mut pending = "ab界".as_bytes()[..4].to_vec();
        assert_eq!(super::take_text(&mut pending, false), "ab");
        pending.extend_from_slice(&"界".as_bytes()[2..]);
        assert_eq!(super::take_text(&mut pending, false), "界");
        assert!(pending.is_empty());
        let mut cut = vec![b'x', 0xe7];
        assert_eq!(
            super::take_text(&mut cut, true),
            "x\u{fffd}",
            "the end flushes"
        );
    }
}
