//! One language-server process from spawn to settlement. The caller
//! names the process as a `std::process::Command` — the server itself
//! locally, an `ssh` client running it remotely — and everything after
//! that runs here, on the client's own runtime thread: spawn, the
//! mainloop over the child's stdio, a bounded stderr tail, and teardown.
//!
//! Teardown order: the mainloop's dropped write half delivers stdin EOF,
//! the server gets [`EXIT_GRACE`] to follow it out, then it is killed.
//! The reported exit is the child's real status (code or signal), never
//! a guessed one.

use std::future::Future;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use async_lsp::router::Router;
use parking_lot::Mutex;
use tokio::io::AsyncReadExt;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::frame_limits::BoundedFrames;
use super::outbound::{FramedWriter, Outbound};
use super::spawn::ClientState;
use super::trace_io::{Direction, Observed};
use crate::protocol::{LspEvent, ServerId};

/// Grace for the server to follow its stdin EOF out before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(3);

/// Bound on collecting the stderr tail after exit: a grandchild still
/// holding the pipe must not hold teardown.
const STDERR_DRAIN: Duration = Duration::from_secs(1);

/// Bounded stderr retained for the failure hint (the newest bytes win).
const STDERR_TAIL_CAP: usize = 8192;

/// A shell's "command not found": from a local wrapper script, or from
/// the remote shell `ssh` runs the server through.
const NOT_FOUND: i32 = 127;

/// `ssh` itself failed (connection, authentication, master) — distinct
/// from anything the server did.
const SSH_FAILED: i32 = 255;

/// Everything one server run reports with, owned by the runtime thread.
pub(super) struct Launch {
    pub id: ServerId,
    pub name: String,
    /// The configured server command, as the user wrote it.
    pub program: String,
    pub hint: String,
    pub label_workspace: String,
    /// The remote endpoint, when the process is an `ssh` client.
    pub endpoint: Option<String>,
    pub tx: Sender<LspEvent>,
    pub quitting: Arc<AtomicBool>,
    /// Set once the mainloop ends: the wire queue stops framing.
    pub closed: Arc<AtomicBool>,
    pub outbound: Arc<Outbound>,
}

impl Launch {
    fn fail(&self, reason: String) {
        let _ = self.tx.send(LspEvent::Failed {
            server: self.id,
            name: self.name.clone(),
            hint: format!("{reason} — {}", self.hint),
        });
    }

    fn cannot_run(&self, reason: &str) -> String {
        format!(
            "cannot run `{}` on {}: {reason}",
            self.program, self.label_workspace
        )
    }

    /// The exit as the user should read it. Only called when the exit
    /// was not requested.
    fn exit_reason(
        &self,
        status: Option<ExitStatus>,
        stderr: &str,
        error: Option<String>,
    ) -> String {
        if status.and_then(|status| status.code()) == Some(NOT_FOUND) {
            let reason = if stderr.is_empty() {
                "command not found"
            } else {
                stderr
            };
            return self.cannot_run(reason);
        }
        let status = match status {
            Some(status) if self.endpoint.is_some() && status.code() == Some(SSH_FAILED) => {
                format!("ssh failed, {status}")
            }
            Some(status) => status.to_string(),
            None => "exit status unavailable".to_string(),
        };
        let mut detail = format!(" ({status})");
        if let Some(error) = error {
            detail.push_str(&format!(": {error}"));
        }
        if !stderr.is_empty() {
            detail.push_str(&format!("; stderr: {stderr}"));
        }
        format!("{} exited unexpectedly{detail}", self.name)
    }

    /// The connection is over before or after it ran: nothing frames
    /// onto it again.
    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.outbound.close();
    }
}

/// Run one server process to its settlement. A spawn failure (program
/// missing) is a `Failed` event naming the command and workspace; an
/// exit the client did not ask for is a `Failed` event carrying the
/// real exit status and the stderr tail.
pub(super) async fn run(
    launch: Launch,
    command: std::process::Command,
    mainloop: async_lsp::MainLoop<Router<ClientState>>,
    stopping: tokio::sync::oneshot::Receiver<()>,
) {
    let mut command = tokio::process::Command::from(command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            launch.close();
            strop_trace::record_with(strop_trace::EventKind::Error, || {
                serde_json::json!({
                    "source":"lsp_spawn","server":launch.name,"command":launch.program,
                    "message":error.to_string(),
                })
            });
            launch.fail(launch.cannot_run(&error.to_string()));
            return;
        }
    };
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        debug_assert!(false, "stdio was configured piped above");
        launch.close();
        launch.fail(launch.cannot_run("the server's stdio is not piped"));
        return;
    };
    let tail = Arc::new(Mutex::new(Vec::new()));
    let collector = tokio::spawn(collect_tail(stderr, tail.clone()));
    let label = format!("{}@{}", launch.name, launch.label_workspace);
    strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
        let mut event = serde_json::json!({"service":"lsp","server":label,"pid":child.id()});
        if let Some(endpoint) = &launch.endpoint {
            event["remote"] = serde_json::json!(endpoint);
        }
        event
    });
    let result = {
        let mut run = std::pin::pin!(mainloop.run_buffered(
            Observed::new(BoundedFrames::new(stdout), &label, Direction::Rx).compat(),
            Observed::new(
                FramedWriter::new(stdin, launch.outbound.clone()),
                &label,
                Direction::Tx
            )
            .compat_write(),
        ));
        let mut stopping = std::pin::pin!(stopping);
        std::future::poll_fn(|context| {
            if stopping.as_mut().poll(context).is_ready() {
                return std::task::Poll::Ready(Ok(()));
            }
            run.as_mut().poll(context)
        })
        .await
        // The mainloop and its write half drop here: stdin EOF.
    };
    launch.closed.store(true, Ordering::Relaxed);
    strop_trace::record_with(strop_trace::EventKind::JobFinished, || {
        serde_json::json!({
            "service":"lsp","server":label,"error":result.as_ref().err().map(ToString::to_string),
        })
    });
    let status = settle(&mut child).await;
    if tokio::time::timeout(STDERR_DRAIN, collector).await.is_err() {
        strop_trace::record_with(
            strop_trace::EventKind::Error,
            || serde_json::json!({"source":"lsp_stderr","server":label,"message":"stderr still open after exit"}),
        );
    }
    if launch.quitting.load(Ordering::Relaxed) {
        return;
    }
    let stderr = String::from_utf8_lossy(&tail.lock()).trim().to_string();
    let error = result.err().map(|error| error.to_string());
    launch.fail(launch.exit_reason(status, &stderr, error));
}

/// Wait out the grace after stdin EOF, then kill. `None` only when the
/// status cannot be collected at all.
async fn settle(child: &mut tokio::process::Child) -> Option<ExitStatus> {
    match tokio::time::timeout(EXIT_GRACE, child.wait()).await {
        Ok(status) => status.ok(),
        Err(_) => {
            let _ = child.start_kill();
            child.wait().await.ok()
        }
    }
}

/// Keep the newest [`STDERR_TAIL_CAP`] bytes of the server's stderr.
async fn collect_tail(mut stderr: tokio::process::ChildStderr, tail: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 4096];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => push_tail(&mut tail.lock(), &chunk[..read]),
        }
    }
}

fn push_tail(tail: &mut Vec<u8>, bytes: &[u8]) {
    tail.extend_from_slice(bytes);
    if tail.len() > STDERR_TAIL_CAP {
        let excess = tail.len() - STDERR_TAIL_CAP;
        tail.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_tail_keeps_the_newest_bytes_within_the_cap() {
        let mut tail = Vec::new();
        push_tail(&mut tail, &vec![b'a'; STDERR_TAIL_CAP]);
        push_tail(&mut tail, b"last words");
        assert_eq!(tail.len(), STDERR_TAIL_CAP);
        assert!(tail.ends_with(b"last words"));
        assert_eq!(tail[0], b'a');
    }
}
