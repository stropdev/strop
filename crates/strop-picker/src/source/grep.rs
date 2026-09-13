//! The grep source: one supervised `rg` child. The supervisor owns the
//! process from spawn to reap — kill/wait happen here, never on the
//! editor thread; stdout and stderr each drain on their own worker so
//! a chatty failure can never block a pipe; and every path (bad flags,
//! missing binary, dead reader, cancellation, panic) ends in exactly
//! one terminal event.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::channel;

use strop_core::worker::{self, CancelReason, FailureKind, Outcome};

use super::query::parse_json_match;
use super::PickerMsg;
use super::SourceSnapshot;

/// One reader-thread event for the supervisor.
#[derive(Debug)]
enum ReadEvent {
    Stdout(Outcome<()>),
    Stderr(Outcome<String>),
    /// The owner dropped or cancelled the worker.
    Cancel,
}

/// The supervisor's reducer: apply one reader event; `Some(outcome)`
/// ends the supervision loop. Pure, so every terminal decision is
/// testable without a real rg process.
fn step(
    event: ReadEvent,
    stdout_done: &mut bool,
    stderr_text: &mut Option<String>,
) -> Option<Outcome<()>> {
    match event {
        ReadEvent::Stdout(Outcome::Success(())) => {
            *stdout_done = true;
            None
        }
        ReadEvent::Stderr(Outcome::Success(text)) => {
            *stderr_text = Some(text);
            None
        }
        ReadEvent::Stdout(Outcome::Failed { failure, .. })
        | ReadEvent::Stderr(Outcome::Failed { failure, .. }) => Some(Outcome::Failed {
            failure,
            partial: None,
        }),
        ReadEvent::Cancel
        | ReadEvent::Stdout(Outcome::Cancelled(_))
        | ReadEvent::Stderr(Outcome::Cancelled(_)) => {
            Some(Outcome::Cancelled(CancelReason::OwnerClosed))
        }
    }
}

/// rg's post-reap verdict: exit 0 and exit-1/no-matches succeed;
/// bad flags, signals and everything else fail visibly (with rg's
/// own stderr text when it has any).
fn verdict(status: ExitStatus, stderr: &str) -> Outcome<()> {
    if status.success() || status.code() == Some(1) {
        return Outcome::Success(());
    }
    let detail = stderr.trim().strip_prefix("rg: ").unwrap_or(stderr.trim());
    Outcome::failed(
        FailureKind::Exit,
        if detail.is_empty() {
            format!("rg exited with {status}")
        } else {
            format!("rg: {detail}")
        },
    )
}

/// The child is reaped exactly here or killed on drop — it can never
/// outlive its supervisor.
struct OwnedChild {
    child: Child,
    reaped: bool,
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Execute one local source request on the editor-owned source worker.
pub(super) fn run(
    query: std::sync::Arc<crate::query::SearchQuery>,
    policy: super::selection::SelectionPolicy,
    cwd: std::path::PathBuf,
    snapshots: Vec<SourceSnapshot>,
    tx: super::flow::StreamSender,
    cancel: strop_core::worker::CancelToken,
) -> Outcome<()> {
    let (events, rx) = channel::<ReadEvent>();
    let stop = events.clone();
    if let Err(failure) = cancel.register_cancel_resource(move || {
        let _ = stop.send(ReadEvent::Cancel);
        Ok(())
    }) {
        return Outcome::Failed {
            failure,
            partial: None,
        };
    }
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
    let content = match crate::query::ContentPlan::compile(&query) {
        Ok(Some(content)) => content,
        Ok(None) => {
            return Outcome::failed(
                FailureKind::Protocol,
                "the query has no search expression — add text or regex",
            )
        }
        Err(diagnostic) => {
            let message = diagnostic.message.clone();
            let _ = tx.control(PickerMsg::QueryError(diagnostic));
            return Outcome::failed(FailureKind::Protocol, message);
        }
    };
    if cancel.is_cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    let pattern = match &content.expr {
        crate::query::ContentExpr::Literal(text) => {
            if text.is_empty() {
                return Outcome::Success(());
            }
            text.clone()
        }
        crate::query::ContentExpr::Regex(pattern) => pattern.clone(),
    };
    // eligible paths from the shared selection authority;
    // bounded argv batches (0051 §3: real bounds, no shell)
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    let mut path_bytes = 0usize;
    let mut path_limit = false;
    let cancelled = || cancel.is_cancelled();
    let walk = super::selection::walk(&cwd, &plan, policy, &cancelled, |rel| {
        path_bytes = path_bytes.saturating_add(rel.as_os_str().as_encoded_bytes().len());
        if paths.len() >= super::selection::PATH_LIMIT || path_bytes > super::selection::PATH_BYTES
        {
            path_limit = true;
            return false;
        }
        paths.push(rel);
        true
    });
    if path_limit {
        return Outcome::failed(
            FailureKind::Unavailable,
            "search selection exceeds 100000 paths or 16 MiB; narrow the scope",
        );
    }
    if let Err(error) = walk {
        return Outcome::failed(FailureKind::Io, error);
    }
    if cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    // Dirty open source text is authoritative, never a hidden disk
    // save. Matching happens on this worker, under the same plan.
    if let Err(error) = super::snapshots::emit_snapshots(
        &strop_workspace::ResourceLocation::local(cwd.clone()),
        &content,
        snapshots,
        &mut paths,
        &tx,
        &cancel,
    ) {
        return if cancel.is_cancelled() {
            Outcome::Cancelled(CancelReason::Superseded)
        } else {
            Outcome::failed(FailureKind::Protocol, error)
        };
    }
    let mut argv: Vec<String> = vec!["--no-config".into(), "--json".into(), "-e".into(), pattern];
    match content.case {
        crate::query::CaseMode::Smart => argv.push("--smart-case".into()),
        crate::query::CaseMode::Sensitive => argv.push("--case-sensitive".into()),
        crate::query::CaseMode::Ignore => argv.push("-i".into()),
    }
    if matches!(content.expr, crate::query::ContentExpr::Literal(_)) {
        argv.push("-F".into());
    }
    if policy.effective(&plan).hidden {
        argv.push("--hidden".into());
    }
    if !policy.effective(&plan).respect_ignore {
        argv.push("--no-ignore".into());
    }
    for arg in super::RG_PROTECTED_ARGS {
        argv.push((*arg).to_string());
    }
    argv.push("--".into());
    let mut batches: Vec<Vec<std::path::PathBuf>> = Vec::new();
    let mut current: Vec<std::path::PathBuf> = Vec::new();
    let mut bytes = argv.iter().map(|a| a.len() + 1).sum::<usize>();
    for path in paths {
        let path_bytes = path.as_os_str().as_encoded_bytes().len();
        if current.len() >= 4096 || bytes + path_bytes > 128 * 1024 {
            batches.push(std::mem::take(&mut current));
            bytes = argv.iter().map(|a| a.len() + 1).sum::<usize>();
        }
        bytes += path_bytes + 1;
        current.push(path);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    for batch in batches {
        if cancel.is_cancelled() {
            return Outcome::Cancelled(CancelReason::OwnerClosed);
        }
        let outcome = std::thread::scope(|scope| {
            let child = match Command::new("rg")
                .args(&argv)
                .args(&batch)
                .current_dir(&cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(child) => child,
                Err(e) => return Outcome::failed(FailureKind::Spawn, format!("rg: {e}")),
            };
            let mut process = OwnedChild {
                child,
                reaped: false,
            };
            let Some(stdout) = process.child.stdout.take() else {
                return Outcome::failed(FailureKind::Protocol, "rg: missing stdout");
            };
            let Some(mut stderr) = process.child.stderr.take() else {
                return Outcome::failed(FailureKind::Protocol, "rg: missing stderr");
            };
            let out_events = events.clone();
            let out_tx = tx.clone();
            let root = strop_workspace::ResourceLocation::local(cwd.clone());
            let out_reader = worker::spawn_scoped(
                scope,
                "rg-stdout",
                move |outcome| {
                    let _ = out_events.send(ReadEvent::Stdout(outcome));
                },
                move |token| {
                    let mut reader = BufReader::new(stdout);
                    let mut line = Vec::new();
                    loop {
                        if token.is_cancelled() {
                            return Outcome::Cancelled(CancelReason::OwnerClosed);
                        }
                        line.clear();
                        match reader
                            .by_ref()
                            .take((super::query::RECORD_LIMIT + 1) as u64)
                            .read_until(b'\n', &mut line)
                        {
                            Ok(0) => break,
                            Ok(_) => {}
                            Err(error) => {
                                return Outcome::failed(FailureKind::Io, error.to_string())
                            }
                        }
                        let items = match parse_json_match(&line, &root) {
                            Ok(items) => items,
                            Err(error) => return Outcome::failed(FailureKind::Protocol, error),
                        };
                        if let Err(error) = out_tx.batch(items, &token) {
                            return error.outcome();
                        }
                    }
                    Outcome::Success(())
                },
            );
            let err_events = events.clone();
            let err_reader = worker::spawn_scoped(
                scope,
                "rg-stderr",
                move |outcome| {
                    let _ = err_events.send(ReadEvent::Stderr(outcome));
                },
                move |_| {
                    let mut retained = Vec::new();
                    let mut chunk = [0; 8192];
                    let mut dropped = 0usize;
                    loop {
                        match stderr.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(count) => {
                                let keep =
                                    count.min((64 * 1024usize).saturating_sub(retained.len()));
                                retained.extend_from_slice(&chunk[..keep]);
                                dropped = dropped.saturating_add(count - keep);
                            }
                            Err(error) => {
                                return Outcome::failed(FailureKind::Io, error.to_string())
                            }
                        }
                    }
                    let mut text = String::from_utf8_lossy(&retained).into_owned();
                    if dropped > 0 {
                        text.push_str(&format!("\nrg stderr truncated ({dropped} bytes omitted)"));
                    }
                    Outcome::Success(text)
                },
            );
            // hold both reader handles: dropping one would cancel a
            // perfectly healthy reader
            let _readers = (out_reader, err_reader);
            let mut stdout_done = false;
            let mut stderr_text = None;
            while !stdout_done || stderr_text.is_none() {
                match rx.recv() {
                    Ok(event) => {
                        if let Some(outcome) = step(event, &mut stdout_done, &mut stderr_text) {
                            return outcome; // OwnedChild::drop kills/reaps
                        }
                    }
                    Err(e) => return Outcome::failed(FailureKind::Disconnected, e.to_string()),
                }
            }
            let status = match process.child.wait() {
                Ok(status) => {
                    process.reaped = true;
                    status
                }
                Err(e) => return Outcome::failed(FailureKind::Wait, format!("rg: {e}")),
            };
            let stderr = stderr_text.unwrap_or_default();
            let outcome = verdict(status, &stderr);
            if matches!(outcome, Outcome::Success(())) && !stderr.trim().is_empty() {
                let detail = stderr.trim().strip_prefix("rg: ").unwrap_or(stderr.trim());
                let _ = tx.control(PickerMsg::Warning(format!("rg: {detail}")));
            }
            outcome
        });
        if !matches!(outcome, Outcome::Success(())) {
            return outcome;
        }
    }
    Outcome::Success(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Sender};

    /// The old pattern-string helper through the query language (0051):
    /// bare text becomes a literal content expression.
    fn spawn_query(
        text: &str,
        cwd: &std::path::Path,
        tx: Sender<PickerMsg>,
    ) -> (super::super::SourceWorker, strop_core::worker::CancelHandle) {
        let query = crate::query::SearchQuery::parse(text);
        let worker = super::super::SourceWorker::new().unwrap();
        let request = worker.search(
            std::sync::Arc::new(query),
            super::super::selection::SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
            strop_workspace::ResourceLocation::local(cwd.to_path_buf()),
            Vec::new(),
            tx,
        );
        (worker, request)
    }

    #[test]
    fn exhausted_catalog_reports_failure_after_the_bounded_prefix() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("hits.txt"),
            "needle\n".repeat(100_001),
        )
        .unwrap();
        let (tx, rx) = channel();
        let worker = super::super::SourceWorker::new().unwrap();
        let _request = worker.search(
            std::sync::Arc::new(crate::query::SearchQuery::parse("needle")),
            super::super::selection::SelectionPolicy {
                hidden: true,
                respect_ignore: false,
            },
            strop_workspace::ResourceLocation::local(directory.path().to_path_buf()),
            Vec::new(),
            tx,
        );
        let mut received = 0;
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
                PickerMsg::Items(items) => received += items.len(),
                PickerMsg::Finished(Outcome::Failed { failure, .. }) => {
                    assert_eq!(failure.kind, FailureKind::Unavailable);
                    assert_eq!(received, 100_000);
                    break;
                }
                event => panic!("expected source rows followed by a capacity failure: {event:?}"),
            }
        }
    }

    #[test]
    fn filter_only_query_is_a_named_failure_not_a_silent_everything_search() {
        let (tx, rx) = channel();
        let worker = spawn_query("language:rust", &std::env::temp_dir(), tx);
        match rx.recv() {
            Ok(PickerMsg::Finished(Outcome::Failed { failure, .. })) => {
                assert!(failure.message.contains("no search expression"));
            }
            other => panic!("filter-only grep must name the need, got {other:?}"),
        }
        drop(worker);
    }

    #[test]
    fn an_invalid_regex_is_a_diagnostic_at_plan_time() {
        let query = crate::query::SearchQuery::parse("regex:\"x [\"");
        let error = crate::query::ContentPlan::compile(&query).unwrap_err();
        assert!(error.message.contains("invalid regex"), "{}", error.message);
    }

    #[test]
    fn reducer_settles_every_terminal_shape() {
        // a reader failure ends supervision carrying the failure
        let mut done = false;
        let mut err = None;
        let out = step(
            ReadEvent::Stdout(Outcome::failed(FailureKind::Io, "pipe broke")),
            &mut done,
            &mut err,
        );
        assert!(
            matches!(&out, Some(Outcome::Failed { failure, .. }) if failure.message == "pipe broke"),
            "reader failure is terminal: {out:?}"
        );
        // owner cancellation is terminal
        let out = step(ReadEvent::Cancel, &mut done, &mut err);
        assert!(matches!(out, Some(Outcome::Cancelled(_))));
        // a reader's own cancellation (handle dropped) is terminal too
        let out = step(
            ReadEvent::Stderr(Outcome::Cancelled(CancelReason::OwnerClosed)),
            &mut done,
            &mut err,
        );
        assert!(matches!(out, Some(Outcome::Cancelled(_))));
        // the two success halves advance independently
        let mut done = false;
        let mut err = None;
        assert!(step(ReadEvent::Stdout(Outcome::Success(())), &mut done, &mut err).is_none());
        assert!(done, "stdout half recorded");
        assert!(step(
            ReadEvent::Stderr(Outcome::Success("warn".into())),
            &mut done,
            &mut err
        )
        .is_none());
        assert_eq!(err.as_deref(), Some("warn"), "stderr half recorded");
    }

    #[cfg(unix)]
    #[test]
    fn rg_exit_verdicts() {
        use std::os::unix::process::ExitStatusExt;
        // exit 0: clean success
        assert!(matches!(
            verdict(ExitStatus::from_raw(0), ""),
            Outcome::Success(())
        ));
        // exit 1: no matches is a successful empty result
        assert!(matches!(
            verdict(ExitStatus::from_raw(0x0100), ""),
            Outcome::Success(())
        ));
        // exit 2 with stderr: rg's own diagnosis, not a silent empty list
        let bad = verdict(ExitStatus::from_raw(0x0200), "rg: unrecognized file type");
        match bad {
            Outcome::Failed { failure, .. } => assert!(failure.message.contains("unrecognized")),
            other => panic!("bad flags must fail, got {other:?}"),
        }
        // exit 2 without stderr: the status itself
        let bare = verdict(ExitStatus::from_raw(0x0200), "");
        match bare {
            Outcome::Failed { failure, .. } => assert!(failure.message.contains("rg exited")),
            other => panic!("bare failure must carry the status, got {other:?}"),
        }
        // killed by a signal (no exit code): failure
        let killed = verdict(ExitStatus::from_raw(9), "");
        assert!(
            matches!(killed, Outcome::Failed { .. }),
            "signals are failures"
        );
    }

    #[test]
    fn grep_worker_streams_and_finishes() {
        let dir = std::env::temp_dir().join("strop-picker-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn sharpen() {}\nlet x = sharpen;\n").unwrap();
        let (tx, rx) = channel();
        let worker = spawn_query("sharpen", &dir, tx);
        let mut items = 0;
        let mut done = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(PickerMsg::Items(batch)) => items += batch.len(),
                Ok(PickerMsg::Warning(_)) => {}
                Ok(PickerMsg::Finished(Outcome::Success(()))) => done = true,
                Ok(other) => panic!("unexpected terminal: {other:?}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(worker);
        std::fs::remove_dir_all(&dir).ok();
        assert!(done, "worker never sent Finished");
        assert_eq!(items, 2);
    }

    #[test]
    #[ignore = "the old -t flag grammar is gone; rg's own failure paths are covered by spawn failures"]
    fn rg_failure_is_a_terminal_failure_not_silence_legacy() {
        let dir = std::env::temp_dir().join("strop-picker-err");
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = channel();
        let worker = spawn_query("regex:\"x [\"", &dir, tx);
        let mut failure = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while failure.is_none() && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(PickerMsg::Finished(Outcome::Failed { failure: f, .. })) => failure = Some(f),
                Ok(PickerMsg::Finished(outcome)) => {
                    panic!("bad type filter must fail, got {outcome:?}")
                }
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(worker);
        std::fs::remove_dir_all(&dir).ok();
        let failure = failure.expect("rg failure posted a terminal event");
        assert!(
            failure.message.contains("unrecognized file type"),
            "{}",
            failure.message
        );
    }
}
