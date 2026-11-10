//! The input threads: tty bytes through the [`Decoder`], and SIGWINCH into
//! a size report. Both block on the OS only — never the UI thread — and park
//! against backpressure instead of dropping input (0056 AR06).
//!
//! Input comes from `/dev/tty`, not stdin, so `cmd | strop -` keeps a
//! keyboard while stdin carries the document.
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
