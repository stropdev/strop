//! Streaming sources. Workers run on the shared terminal worker
//! (`strop_core::worker`): every launch — success, failure, panic or
//! cancellation — settles exactly once with a terminal `Finished`
//! (0001 §5.6: input never waits on a source; R9: no silent empty
//! successes, no stream that never ends).

mod grep;
mod query;

pub use grep::GrepWorker;

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use strop_core::worker::{self, CancelHandle, CancelReason, FailureKind, Outcome};

use crate::{Item, Payload};

/// Messages workers post to the event loop.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PickerMsg {
    Items(Vec<Item>),
    /// A successful source can still issue a useful warning (rg's
    /// stderr on exit 0): it lands in the picker, not the void.
    Warning(String),
    /// Terminal: exactly one per request, on every path — success,
    /// failure (keeping whatever items already streamed), panic or
    /// cancellation. Streaming stops here.
    Finished(Outcome<()>),
}

/// Walk the working directory (respects .gitignore via `ignore`),
/// streaming paths in chunks. A walk error terminates visibly after
/// the batches already streamed — never as a silent empty success.
pub fn spawn_files(cwd: PathBuf, tx: Sender<PickerMsg>) -> CancelHandle {
    let terminal = tx.clone();
    worker::spawn(
        "picker-files",
        move |outcome| {
            let _ = terminal.send(PickerMsg::Finished(outcome));
        },
        move |cancel| {
            let mut batch = Vec::with_capacity(512);
            for result in ignore::WalkBuilder::new(&cwd).hidden(true).build() {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::OwnerClosed);
                }
                let entry = match result {
                    Ok(entry) => entry,
                    Err(error) => {
                        // keep the useful partial stream, then fail loudly
                        if !batch.is_empty() {
                            let _ = tx.send(PickerMsg::Items(std::mem::take(&mut batch)));
                        }
                        return Outcome::failed(FailureKind::Io, error.to_string());
                    }
                };
                let Ok(rel) = entry.path().strip_prefix(&cwd) else {
                    continue;
                };
                if entry.file_type().is_some_and(|t| t.is_dir()) {
                    continue;
                }
                batch.push(Item {
                    text: rel.display().to_string(),
                    payload: Payload::File(rel.to_path_buf()),
                });
                if batch.len() >= 512
                    && tx
                        .send(PickerMsg::Items(std::mem::take(&mut batch)))
                        .is_err()
                {
                    // the event loop dropped the stream: cancelled
                    return Outcome::Cancelled(CancelReason::OwnerClosed);
                }
            }
            if !batch.is_empty() && tx.send(PickerMsg::Items(batch)).is_err() {
                return Outcome::Cancelled(CancelReason::OwnerClosed);
            }
            Outcome::Success(())
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    #[test]
    fn files_worker_walks() {
        let dir = std::env::temp_dir().join("strop-picker-walk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.rs"), "").unwrap();
        let (tx, rx) = channel();
        let worker = super::spawn_files(dir.clone(), tx);
        let mut found = false;
        let mut done = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(super::PickerMsg::Items(batch)) => {
                    if batch.iter().any(|i| i.text.contains("x.rs")) {
                        found = true;
                    }
                }
                Ok(super::PickerMsg::Warning(_)) => {}
                Ok(super::PickerMsg::Finished(super::Outcome::Success(()))) => done = true,
                Ok(other) => panic!("unexpected terminal: {other:?}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(worker);
        std::fs::remove_dir_all(&dir).ok();
        assert!(found, "walked file was streamed");
        assert!(done, "walk settled with a terminal event");
    }
}
