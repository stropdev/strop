//! Only this worker writes to disk. It flushes each available batch and reports
//! both queue loss and I/O failure instead of leaving a plausible complete log.
use crate::{EventKind, Failure, Record, SCHEMA_VERSION};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

pub(super) fn run(file: File, receiver: Receiver<Record>, failure: Arc<Failure>) {
    if let Err(error) = drain(BufWriter::new(file), receiver, &failure) {
        failure.set(|| format!("writer I/O failed: {error}"));
    }
}

fn line(out: &mut impl Write, seq: u64, record: &Record) -> std::io::Result<()> {
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
) -> std::io::Result<()> {
    let mut seq = 0;
    let mut elapsed_us = 0;
    while let Ok(first) = receiver.recv() {
        seq += 1;
        elapsed_us = first.elapsed_us;
        line(&mut out, seq, &first)?;
        for record in receiver.try_iter().take(63) {
            seq += 1;
            elapsed_us = record.elapsed_us;
            line(&mut out, seq, &record)?;
        }
        out.flush()?;
    }
    if let Some(message) = failure.message.lock().as_ref() {
        let fields = serde_json::to_vec(
            &serde_json::json!({"source":"trace", "incomplete":true, "message":message}),
        )?;
        line(
            &mut out,
            seq + 1,
            &Record {
                kind: EventKind::Error,
                elapsed_us,
                fields,
            },
        )?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FailingWriter;
    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
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
        let error = drain(FailingWriter, rx, &Failure::default()).unwrap_err();
        assert_eq!(error.to_string(), "disk full");
    }
}
