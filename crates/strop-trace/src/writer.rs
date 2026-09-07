//! Only this worker writes to disk. It flushes each available batch, keeps
//! the capture inside its bounds, and ALWAYS finishes the file with an
//! explicit terminal `TraceEnd` record — a capped or failed capture says
//! so instead of looking complete.
use crate::{EventKind, Failure, Limits, Record, SCHEMA_VERSION, TERMINAL_RESERVE};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

pub(super) fn run(file: File, receiver: Receiver<Record>, failure: Arc<Failure>, limits: Limits) {
    if let Err(error) = drain(BufWriter::new(file), receiver, &failure, limits) {
        failure.set(|| format!("writer I/O failed: {error}"));
    }
}

fn line(out: &mut impl Write, seq: u64, record: &Record) -> io::Result<()> {
    write!(
        out,
        "{{\"schema_version\":{SCHEMA_VERSION},\"seq\":{seq},\"elapsed_us\":{},\"event\":",
        record.elapsed_us
    )?;
    serde_json::to_writer(&mut *out, &record.kind)?;
    out.write_all(b",\"fields\":")?;
    out.write_all(&record.fields)?;
    out.write_all(b"}\n")
}

fn drain(
    mut out: impl Write,
    receiver: Receiver<Record>,
    failure: &Failure,
    limits: Limits,
) -> io::Result<()> {
    let (mut seq, mut bytes, mut elapsed) = (0u64, 0usize, 0u128);
    let mut capped = false;
    'admission: while let Ok(first) = receiver.recv() {
        let mut batch = std::iter::once(first).chain(receiver.try_iter().take(63));
        for record in &mut batch {
            let mut encoded = Vec::with_capacity(record.fields.len() + 128);
            line(&mut encoded, seq + 1, &record)?;
            if seq >= limits.events
                || encoded.len()
                    > limits
                        .bytes
                        .saturating_sub(TERMINAL_RESERVE)
                        .saturating_sub(bytes)
            {
                capped = true;
                failure.set(|| "capture limit reached".into());
                break 'admission;
            }
            out.write_all(&encoded)?;
            bytes += encoded.len();
            seq += 1;
            elapsed = record.elapsed_us;
        }
        out.flush()?;
    }
    // The terminal marker is reserved budget: whatever happened above, the
    // file must end by saying whether the capture is complete.
    let complete = !capped && failure.message.lock().is_none();
    let reason = if capped {
        "capture_limit"
    } else if complete {
        "complete"
    } else {
        "capture_failure"
    };
    let terminal = Record {
        kind: EventKind::TraceEnd,
        elapsed_us: elapsed,
        fields: serde_json::to_vec(&serde_json::json!({"complete": complete, "reason": reason}))?,
    };
    let mut encoded = Vec::new();
    line(&mut encoded, seq + 1, &terminal)?;
    if encoded.len() > TERMINAL_RESERVE || encoded.len() > limits.bytes.saturating_sub(bytes) {
        return Err(io::Error::other("terminal reservation violated"));
    }
    out.write_all(&encoded)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FailingWriter;
    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn disk_failure_is_returned_not_silenced() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Record {
            kind: EventKind::Input,
            elapsed_us: 0,
            fields: b"{}".to_vec(),
        })
        .unwrap();
        drop(tx);
        let error = drain(FailingWriter, rx, &Failure::default(), Limits::default()).unwrap_err();
        assert_eq!(error.to_string(), "disk full");
    }

    #[test]
    fn terminal_marker_always_writes_within_reservation() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        let mut out = Vec::new();
        drain(&mut out, rx, &Failure::default(), Limits::default()).unwrap();
        let text = String::from_utf8(out).unwrap();
        let line = text.trim();
        assert!(line.contains("\"event\":\"trace_end\""));
        assert!(line.contains("\"complete\":true"));
        assert!(line.contains("\"reason\":\"complete\""));
        assert!(line.len() <= TERMINAL_RESERVE);
    }
}
