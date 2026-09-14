//! Streaming sources. Workers run on the shared terminal worker
//! (`strop_core::worker`): every launch — success, failure, panic or
//! cancellation — settles exactly once with a terminal `Finished`
//! (0001 §5.6: input never waits on a source; R9: no silent empty
//! successes, no stream that never ends).

mod grep;
mod query;
mod remote;
mod snapshots;
mod worker;
pub use worker::SourceWorker;

mod flow;
use flow::StreamSender;
pub use flow::{ItemBatch, SourceSink};
pub use snapshots::SourceSnapshot;

use std::path::PathBuf;

use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};

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

pub mod catalog;
pub mod selection;
pub mod symbols;

pub use selection::{display_path, SelectionPolicy, RG_PROTECTED_ARGS};

/// Walk the working directory through the shared selection authority
/// (0051 §3): one FileSelectionPlan filters candidates for every
/// surface; dotfiles show by default; `.git` is never a result. A walk
/// error terminates visibly after the batches already streamed — never
/// as a silent empty success.
fn run_files(
    cwd: PathBuf,
    query: std::sync::Arc<crate::query::SearchQuery>,
    policy: selection::SelectionPolicy,
    tx: StreamSender,
    cancel: CancelToken,
) -> Outcome<()> {
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
    let mut delivery_error = None;
    let cancelled = || cancel.is_cancelled();
    let result = selection::walk(&cwd, &plan, policy, &cancelled, |rel| {
        batch.push(Item {
            badge: None,
            text: display_path(&rel).into_owned(),
            payload: Payload::File(rel),
        });
        if batch.len() >= 512 {
            return match tx.batch(std::mem::take(&mut batch), &cancel) {
                Ok(()) => true,
                Err(error) => {
                    delivery_error = Some(error);
                    false
                }
            };
        }
        true
    });
    if let Some(error) = delivery_error {
        return error.outcome();
    }
    if let Err(error) = result {
        if !batch.is_empty() {
            if let Err(error) = tx.batch(std::mem::take(&mut batch), &cancel) {
                return error.outcome();
            }
        }
        return Outcome::failed(FailureKind::Io, error);
    }
    if let Err(error) = tx.batch(batch, &cancel) {
        return error.outcome();
    }
    Outcome::Success(())
}

/// Every declaration in the opened scope (0063 §2), syntax-fallback
/// tier: one walk through the shared selection authority, one bounded
/// symbol index, one row per declaration. Coverage is honest — the
/// index's bounds surface as a visible warning, never as silence.
fn run_workspace_symbols(
    cwd: PathBuf,
    query: std::sync::Arc<crate::query::SearchQuery>,
    policy: selection::SelectionPolicy,
    tx: StreamSender,
    cancel: CancelToken,
) -> Outcome<()> {
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
    let cancelled = || cancel.is_cancelled();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut path_bytes = 0usize;
    let mut path_limit = false;
    let walk = selection::walk(&cwd, &plan, policy, &cancelled, |rel| {
        path_bytes = path_bytes.saturating_add(rel.as_os_str().as_encoded_bytes().len());
        if paths.len() >= selection::PATH_LIMIT || path_bytes > selection::PATH_BYTES {
            path_limit = true;
            return false;
        }
        paths.push(rel);
        true
    });
    if path_limit {
        return Outcome::failed(
            FailureKind::Unavailable,
            "workspace symbols selection exceeds 100000 paths or 16 MiB; narrow the scope",
        );
    }
    if let Err(error) = walk {
        return Outcome::failed(FailureKind::Io, error);
    }
    if cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    let index = symbols::SymbolIndex::build(&cwd, &paths, &cancelled);
    if cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    let root = strop_workspace::ResourceLocation::local(cwd);
    let mut batch = Vec::with_capacity(512);
    let mut delivery_error = None;
    // Deterministic rows: path order, then declaration order in file.
    let mut entries: Vec<_> = index.iter().collect();
    entries.sort_by_key(|(path, _)| *path);
    for (path, declarations) in entries {
        for declaration in declarations {
            if cancelled() {
                return Outcome::Cancelled(CancelReason::OwnerClosed);
            }
            let relative = PathBuf::from(path);
            batch.push(Item {
                badge: Some(declaration.kind.chip().to_string()),
                text: format!(
                    "{}  {} · :{}",
                    declaration.name,
                    display_path(&relative),
                    declaration.line
                ),
                payload: Payload::Grep {
                    location: strop_workspace::ResourceLocation {
                        filesystem: root.filesystem.clone(),
                        path: root.path.join(&relative),
                    },
                    line: declaration.line,
                    col: declaration.col,
                    match_len: declaration.name.len(),
                    line_text: "".into(),
                },
            });
        }
        if batch.len() >= 512 {
            if let Err(error) = tx.batch(std::mem::take(&mut batch), &cancel) {
                delivery_error = Some(error);
                break;
            }
        }
    }
    if let Some(error) = delivery_error {
        return error.outcome();
    }
    if let Err(error) = tx.batch(batch, &cancel) {
        return error.outcome();
    }
    if index.coverage_gap() {
        let _ = tx.control(PickerMsg::Warning(
            "workspace symbols: syntax tier truncated at its bounds; narrow the scope for full coverage".into(),
        ));
    }
    Outcome::Success(())
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
        let worker = super::SourceWorker::new().unwrap();
        let _request = worker.files(
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

    #[test]
    fn workspace_symbols_worker_lists_declarations_with_chips() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "struct St;\nfn wrap() {\n    let x = 1;\n}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "fn not rust\n").unwrap();
        let (tx, rx) = channel();
        let worker = super::SourceWorker::new().unwrap();
        let _request = worker.workspace_symbols(
            dir.path().to_path_buf(),
            std::sync::Arc::new(crate::query::SearchQuery::default()),
            super::selection::SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
            tx,
        );
        let mut rows: Vec<(Option<String>, String)> = Vec::new();
        let mut done = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(super::PickerMsg::Items(batch)) => rows.extend(
                    batch
                        .iter()
                        .map(|item| (item.badge.clone(), item.text.clone())),
                ),
                Ok(super::PickerMsg::Warning(_)) => {}
                Ok(super::PickerMsg::Finished(super::Outcome::Success(()))) => done = true,
                Ok(other) => panic!("unexpected terminal: {other:?}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(worker);
        assert!(done, "source settled with a terminal event");
        // Symbol-row convention: "name  container · :line" with the
        // kind chip in the badge column (0047 §1 / 0063 §2).
        assert!(rows.contains(&(Some("struct".into()), "St  lib.rs · :1".into())));
        assert!(rows.contains(&(Some("fn".into()), "wrap  lib.rs · :2".into())));
        assert!(
            !rows.iter().any(|(_, text)| text.contains("notes.txt")),
            "non-source files contribute no rows"
        );
    }
}
