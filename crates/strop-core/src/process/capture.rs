//! Bounded capture for short subprocesses. The owning worker drains
//! both pipes concurrently and never relinquishes cancellation to a
//! running child.
//!
//! Two policies generalize the original 64 KiB/30 s profile:
//!
//! - [`StdinPolicy::Held`] keeps the child's stdin writer open for the
//!   whole capture — required when the pipe is a lifetime lease (the
//!   remote supervisor treats SSH stdin close as *cancel*, so closing
//!   it early would kill a healthy command). A held stdin never
//!   delivers data; it simply does not deliver EOF.
//! - [`CapturePolicy::stderr_tail`] reserves bytes at the *end* of
//!   stderr in addition to the head, so a final status record survives
//!   a chatty worker.
//!
//! Overflowing either limit truncates (head, plus tail for stderr) and
//! reports the dropped byte count on [`CommandOutput`] rather than
//! failing: callers decide whether truncation invalidates the result.
use super::OwnedProcess;
use crate::worker::{CancelToken, Failure, FailureKind};
use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

const LIMIT: u64 = 64 * 1024;
const DEADLINE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(20);
const CHUNK: usize = 64 * 1024;

/// One captured pipe: the retained bytes and how many arrived beyond
/// the retention window.
struct Retained {
    bytes: Vec<u8>,
    dropped: u64,
}

pub struct CommandOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Stdout bytes that arrived beyond `stdout_limit` and were
    /// discarded. Zero means nothing was dropped.
    pub stdout_dropped: u64,
    /// Stderr bytes discarded between the retained head and tail.
    pub stderr_dropped: u64,
}

/// What capture does with the child's stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdinPolicy {
    /// The child reads `/dev/null` and sees EOF immediately.
    Null,
    /// The child's stdin is a pipe whose local writer is held open for
    /// the whole capture and dropped when the child has exited. The
    /// child sees an open stdin with no data — never an early EOF.
    Held,
}

/// Bounds and stdin handling for one capture.
#[derive(Debug, Clone)]
pub struct CapturePolicy {
    /// Retained head of stdout.
    pub stdout_limit: u64,
    /// Retained head of stderr.
    pub stderr_limit: u64,
    /// Additional bytes retained from the *end* of stderr, after any
    /// dropped middle. Zero disables the tail.
    pub stderr_tail: u64,
    /// Wall-clock budget for the whole exchange.
    pub deadline: Duration,
    pub stdin: StdinPolicy,
}

impl Default for CapturePolicy {
    fn default() -> Self {
        Self {
            stdout_limit: LIMIT,
            stderr_limit: LIMIT,
            stderr_tail: 0,
            deadline: DEADLINE,
            stdin: StdinPolicy::Null,
        }
    }
}

/// Why a capture did not produce output. `capture` folds these back
/// into the historical `Failure` spellings; richer callers (remote
/// exec) match on them directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    Spawn(String),
    Cancelled,
    TimedOut(Duration),
    Failure(Failure),
}

enum Stream {
    Stdout(io::Result<Retained>),
    Stderr(io::Result<Retained>),
}

/// Keep the first `head` bytes and, when `tail` is non-zero, also the
/// last `tail` bytes; count everything dropped in between.
fn read_pipe(mut pipe: impl Read, head: u64, tail: u64) -> io::Result<Retained> {
    let mut kept = Vec::new();
    let mut tail_window: Vec<u8> = Vec::new();
    let mut dropped: u64 = 0;
    let mut chunk = vec![0u8; CHUNK];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(seen) => {
                let mut data = &chunk[..seen];
                let head_room = (head as usize).saturating_sub(kept.len());
                if head_room > 0 {
                    let take = head_room.min(data.len());
                    kept.extend_from_slice(&data[..take]);
                    data = &data[take..];
                }
                if !data.is_empty() {
                    if tail > 0 {
                        tail_window.extend_from_slice(data);
                        let over = tail_window.len().saturating_sub(tail as usize);
                        if over > 0 {
                            tail_window.drain(..over);
                            dropped += over as u64;
                        }
                    } else {
                        dropped += data.len() as u64;
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    kept.extend_from_slice(&tail_window);
    Ok(Retained {
        bytes: kept,
        dropped,
    })
}

/// At most 64 KiB per pipe, 30 seconds, stdin from `/dev/null`. No
/// terminal input/output is inherited.
pub fn capture(command: &mut Command, token: &CancelToken) -> Result<CommandOutput, Failure> {
    capture_with(command, token, &CapturePolicy::default()).map_err(|error| match error {
        CaptureError::Spawn(message) => Failure::new(FailureKind::Spawn, message),
        CaptureError::Cancelled => {
            Failure::new(FailureKind::Unavailable, "configuration command cancelled")
        }
        CaptureError::TimedOut(deadline) => Failure::new(
            FailureKind::Wait,
            format!(
                "configuration command timed out after {} seconds",
                deadline.as_secs()
            ),
        ),
        CaptureError::Failure(failure) => failure,
    })
}

/// Run one command to completion under explicit bounds. The child runs
/// in its own process group owned by [`OwnedProcess`]; cancellation
/// SIGKILLs it, the deadline kills it, and both pipes are drained
/// concurrently with bounded retention.
pub fn capture_with(
    command: &mut Command,
    token: &CancelToken,
    policy: &CapturePolicy,
) -> Result<CommandOutput, CaptureError> {
    let failure =
        |kind: FailureKind, message: String| CaptureError::Failure(Failure::new(kind, message));
    std::thread::scope(|scope| {
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        match policy.stdin {
            StdinPolicy::Null => command.stdin(Stdio::null()),
            StdinPolicy::Held => command.stdin(Stdio::piped()),
        };
        // Owned here, INSIDE scope: unwinding kills pipes before scope joins.
        let mut process = OwnedProcess::spawn(command, token).map_err(|failure| {
            if failure.kind == FailureKind::Spawn {
                CaptureError::Spawn(failure.message)
            } else {
                CaptureError::Failure(failure)
            }
        })?;
        // Held: keep the writer alive until the child has exited; its
        // drop afterwards is pure pipe hygiene.
        let _lease = if policy.stdin == StdinPolicy::Held {
            process.take_stdin()
        } else {
            None
        };
        let stdout = process
            .take_stdout()
            .ok_or_else(|| failure(FailureKind::Protocol, "missing stdout".into()))?;
        let stderr = process
            .take_stderr()
            .ok_or_else(|| failure(FailureKind::Protocol, "missing stderr".into()))?;
        let (tx, rx) = channel();
        let out_tx = tx.clone();
        let out_limit = policy.stdout_limit;
        let err_limit = policy.stderr_limit;
        let err_tail = policy.stderr_tail;
        std::thread::Builder::new()
            .name("capture-stdout".into())
            .spawn_scoped(scope, move || {
                let _ = out_tx.send(Stream::Stdout(read_pipe(stdout, out_limit, 0)));
            })
            .map_err(|error| failure(FailureKind::ThreadStart, error.to_string()))?;
        std::thread::Builder::new()
            .name("capture-stderr".into())
            .spawn_scoped(scope, move || {
                let _ = tx.send(Stream::Stderr(read_pipe(stderr, err_limit, err_tail)));
            })
            .map_err(|error| failure(FailureKind::ThreadStart, error.to_string()))?;
        let deadline = Instant::now() + policy.deadline;
        let mut stdout: Option<Retained> = None;
        let mut stderr: Option<Retained> = None;
        loop {
            if token.is_cancelled() {
                return Err(CaptureError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(CaptureError::TimedOut(policy.deadline));
            }
            let exited = process.has_exited().map_err(CaptureError::Failure)?;
            if exited {
                process.terminate().map_err(CaptureError::Failure)?; // descendants cannot retain the pipes
                match (stdout.take(), stderr.take()) {
                    (Some(stdout), Some(stderr)) => {
                        let status = process.wait().map_err(CaptureError::Failure)?;
                        return Ok(CommandOutput {
                            status,
                            stdout: stdout.bytes,
                            stderr: stderr.bytes,
                            stdout_dropped: stdout.dropped,
                            stderr_dropped: stderr.dropped,
                        });
                    }
                    (out, err) => {
                        stdout = out;
                        stderr = err;
                    }
                }
            }
            let event = match rx.recv_timeout(POLL) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) if !exited => {
                    std::thread::park_timeout(POLL);
                    continue;
                }
                Err(error) => return Err(failure(FailureKind::Disconnected, error.to_string())),
            };
            let (slot, result) = match event {
                Stream::Stdout(result) => (&mut stdout, result),
                Stream::Stderr(result) => (&mut stderr, result),
            };
            *slot = Some(result.map_err(|error| failure(FailureKind::Io, error.to_string()))?);
        }
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(script);
        command
    }

    fn policy(stdin: StdinPolicy, deadline: Duration) -> CapturePolicy {
        CapturePolicy {
            stdin,
            deadline,
            ..CapturePolicy::default()
        }
    }

    fn run_capture(
        script: &'static str,
        policy: CapturePolicy,
    ) -> Result<CommandOutput, CaptureError> {
        let (tx, rx) = channel();
        let _owner = crate::worker::spawn(
            "capture-oracle",
            move |outcome| {
                let _ = tx.send(outcome);
            },
            move |token| {
                crate::worker::Outcome::Success(capture_with(&mut sh(script), &token, &policy))
            },
        );
        match rx
            .recv_timeout(Duration::from_secs(10))
            .expect("capture settled")
        {
            crate::worker::Outcome::Success(result) => result,
            _ => panic!("capture worker failed"),
        }
    }

    #[test]
    fn small_outputs_come_back_whole() {
        let output = run_capture("echo out; echo err >&2", CapturePolicy::default()).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"out\n");
        assert_eq!(output.stderr, b"err\n");
        assert_eq!(output.stdout_dropped, 0);
        assert_eq!(output.stderr_dropped, 0);
    }

    #[test]
    fn stdout_keeps_the_head_and_counts_the_drops() {
        let bounded = CapturePolicy {
            stdout_limit: 1000,
            ..CapturePolicy::default()
        };
        let output = run_capture("yes | head -c 200000", bounded).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 1000);
        assert_eq!(output.stdout_dropped, 199000);
        assert!(output
            .stdout
            .iter()
            .all(|&byte| byte == b'y' || byte == b'\n'));
    }

    #[test]
    fn stderr_tail_keeps_the_final_record() {
        let bounded = CapturePolicy {
            stderr_limit: 1000,
            stderr_tail: 64,
            ..CapturePolicy::default()
        };
        let script = "printf 'start'; head -c 100000 /dev/zero 1>&2; printf 'END-MARK' 1>&2";
        let output = run_capture(script, bounded).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stderr_dropped, 98_944);
        assert_eq!(output.stderr.len(), 1064);
        assert!(output.stderr.ends_with(b"END-MARK"), "{:?}", output.stderr);
    }

    #[test]
    fn null_stdin_delivers_eof_immediately() {
        let output = run_capture("read line; echo done", CapturePolicy::default()).unwrap();
        assert_eq!(output.stdout, b"done\n");
    }

    #[test]
    fn held_stdin_is_open_rather_than_eof() {
        // `read` would return instantly on EOF; an open pipe with no
        // data blocks, so only the deadline can end this — proving the
        // writer was genuinely held.
        let held = policy(StdinPolicy::Held, Duration::from_secs(2));
        assert!(matches!(
            run_capture("read line; echo done", held),
            Err(CaptureError::TimedOut(_))
        ));
    }
}
