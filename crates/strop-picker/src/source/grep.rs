//! The grep source: one supervised `rg` child. The supervisor owns the
//! process from spawn to reap — kill/wait happen here, never on the
//! editor thread; stdout and stderr each drain on their own worker so
//! a chatty failure can never block a pipe; and every path (bad flags,
//! missing binary, dead reader, cancellation, panic) ends in exactly
//! one terminal event.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Sender};

use strop_core::worker::{self, CancelHandle, CancelReason, FailureKind, Outcome};

use super::query::{parse_json_match, split_query};
use super::PickerMsg;

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

/// A grep worker. Each keystroke respawns the search; dropping or
/// cancelling the worker kills the previous rg before it can flood —
/// through the supervisor, so both pipes and the process are reaped.
pub struct GrepWorker {
    stop: Sender<ReadEvent>,
    terminal: Option<CancelHandle>,
}

impl GrepWorker {
    /// Run `rg` for `query` in `cwd`, streaming JSON match events. The
    /// query splits into a pattern plus passthrough filter flags
    /// (`-t rs`, `--glob '!target/*'` — rootle-style power filters).
    /// An empty pattern succeeds without spawning rg.
    pub fn spawn(query: &str, cwd: &std::path::Path, tx: Sender<PickerMsg>) -> Self {
        let tx = super::flow::StreamSender::from(tx);
        let (pattern, filters) = split_query(query);
        let cwd = cwd.to_path_buf();
        let (events, rx) = channel::<ReadEvent>();
        let stop = events.clone();
        let terminal_tx = tx.clone();
        let terminal = worker::spawn(
            "picker-rg",
            move |outcome| {
                let _ = terminal_tx.control(PickerMsg::Finished(outcome));
            },
            move |cancel| {
                if pattern.is_empty() {
                    return Outcome::Success(());
                }
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::OwnerClosed);
                }
                let child = match Command::new("rg")
                    .args(["--json", "--smart-case"])
                    .args(&filters)
                    .arg("--")
                    .arg(&pattern)
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
                let out_reader = worker::spawn(
                    "rg-stdout",
                    move |outcome| {
                        let _ = out_events.send(ReadEvent::Stdout(outcome));
                    },
                    move |token| {
                        for line in BufReader::new(stdout).lines() {
                            if token.is_cancelled() {
                                return Outcome::Cancelled(CancelReason::OwnerClosed);
                            }
                            let line = match line {
                                Ok(line) => line,
                                Err(e) => return Outcome::failed(FailureKind::Io, e.to_string()),
                            };
                            let items = parse_json_match(&line);
                            if !out_tx.batch(items, &token) {
                                return Outcome::Cancelled(CancelReason::OwnerClosed);
                            }
                        }
                        Outcome::Success(())
                    },
                );
                let err_reader = worker::spawn(
                    "rg-stderr",
                    move |outcome| {
                        let _ = events.send(ReadEvent::Stderr(outcome));
                    },
                    move |_| {
                        let mut text = String::new();
                        match stderr.read_to_string(&mut text) {
                            Ok(_) => Outcome::Success(text),
                            Err(e) => Outcome::Failed {
                                failure: strop_core::worker::Failure::new(
                                    FailureKind::Io,
                                    e.to_string(),
                                ),
                                partial: Some(text),
                            },
                        }
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
            },
        );
        Self {
            stop,
            terminal: Some(terminal),
        }
    }

    /// Explicit cancellation: wake the supervisor (it kills and reaps
    /// the child) and revoke publication immediately with the real
    /// reason.
    pub fn cancel(mut self, reason: CancelReason) {
        let _ = self.stop.send(ReadEvent::Cancel);
        if let Some(handle) = self.terminal.take() {
            handle.cancel(reason);
        }
    }
}

impl Drop for GrepWorker {
    fn drop(&mut self) {
        let _ = self.stop.send(ReadEvent::Cancel);
        if let Some(handle) = self.terminal.take() {
            handle.cancel(CancelReason::OwnerClosed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn empty_query_settles_success_without_spawning_rg() {
        let (tx, rx) = channel();
        let worker = GrepWorker::spawn("", &std::env::temp_dir(), tx);
        match rx.recv() {
            Ok(PickerMsg::Finished(Outcome::Success(()))) => {}
            other => panic!("empty query must settle Success, got {other:?}"),
        }
        // nothing follows the terminal event; the channel then closes
        assert!(rx.recv().is_err(), "no messages after the terminal event");
        drop(worker);
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
        let worker = GrepWorker::spawn("sharpen", &dir, tx);
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
    fn rg_failure_is_a_terminal_failure_not_silence() {
        // a bad -t alias used to look like "no matches" (stderr nulled)
        let dir = std::env::temp_dir().join("strop-picker-err");
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = channel();
        let worker = GrepWorker::spawn("x -t nosuchtype", &dir, tx);
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
