//! Streaming sources. Workers run on the shared terminal worker
//! (`strop_core::worker`): every launch — success, failure, panic or
//! cancellation — settles exactly once with a terminal `Finished`
//! (0001 §5.6: input never waits on a source; R9: no silent empty
//! successes, no stream that never ends).

mod grep;
mod query;

mod flow;
pub use flow::ItemBatch;
use flow::StreamSender;
pub use grep::{GrepWorker, SourceSnapshot};

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use strop_core::worker::{self, CancelHandle, CancelReason, FailureKind, Outcome};

use crate::{Item, Payload};

/// Messages workers post to the event loop.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PickerMsg {
    Items(ItemBatch),
    /// A successful source can still issue a useful warning (rg's
    /// stderr on exit 0): it lands in the picker, not the void.
    Warning(String),
    QueryError(crate::query::QueryDiagnostic),
    /// Terminal: exactly one per request, on every path — success,
    /// failure (keeping whatever items already streamed), panic or
    /// cancellation. Streaming stops here.
    Finished(Outcome<()>),
}

pub mod selection;

pub use selection::{display_path, SelectionPolicy, RG_PROTECTED_ARGS};

/// Walk the working directory through the shared selection authority
/// (0051 §3): one FileSelectionPlan filters candidates for every
/// surface; dotfiles show by default; `.git` is never a result. A walk
/// error terminates visibly after the batches already streamed — never
/// as a silent empty success.
pub fn spawn_files(
    cwd: PathBuf,
    query: std::sync::Arc<crate::query::SearchQuery>,
    policy: selection::SelectionPolicy,
    tx: Sender<PickerMsg>,
) -> CancelHandle {
    let tx = StreamSender::from(tx);
    let terminal = tx.clone();
    worker::spawn(
        "picker-files",
        move |outcome| {
            let _ = terminal.control(PickerMsg::Finished(outcome));
        },
        move |cancel| {
            if cancel.is_cancelled() {
                return Outcome::Cancelled(CancelReason::OwnerClosed);
            }
            let plan = match crate::query::FileSelectionPlan::compile(&query) {
                Ok(plan) => plan,
                Err(diagnostic) => {
                    let message = diagnostic.message.clone();
                    let _ = tx.control(PickerMsg::QueryError(diagnostic));
                    return Outcome::failed(FailureKind::Protocol, message);
                }
            };
            if query.exact_file_expression {
                if let Err(diagnostic) = crate::query::ContentPlan::compile(&query) {
                    let message = diagnostic.message.clone();
                    let _ = tx.control(PickerMsg::QueryError(diagnostic));
                    return Outcome::failed(FailureKind::Protocol, message);
                }
            }
            let mut batch = Vec::with_capacity(512);
            let cancelled = || cancel.is_cancelled();
            let result = selection::walk(&cwd, &plan, policy, &cancelled, |rel| {
                batch.push(Item {
                    badge: None,
                    text: display_path(&rel).into_owned(),
                    payload: Payload::File(rel),
                });
                if batch.len() >= 512 {
                    return tx.batch(std::mem::take(&mut batch), &cancel);
                }
                true
            });
            if let Err(error) = result {
                if !batch.is_empty() {
                    tx.batch(std::mem::take(&mut batch), &cancel);
                }
                return Outcome::failed(FailureKind::Io, error);
            }
            if !batch.is_empty() && !tx.batch(batch, &cancel) {
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
        let worker = super::spawn_files(
            dir.clone(),
            std::sync::Arc::new(crate::query::SearchQuery::default()),
            super::selection::SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
            tx,
        );
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
