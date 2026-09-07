//! Bounded capture for short configuration commands. The owning worker drains
//! both pipes concurrently and never relinquishes cancellation to a running child.
use super::OwnedProcess;
use crate::worker::{CancelToken, Failure, FailureKind};
use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

const LIMIT: u64 = 64 * 1024;
const DEADLINE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(20);

pub struct CommandOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
enum Stream {
    Stdout(io::Result<Vec<u8>>),
    Stderr(io::Result<Vec<u8>>),
}
fn read_pipe(pipe: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other(
            "configuration command exceeded 64 KiB output limit",
        ));
    }
    Ok(bytes)
}

/// At most 64 KiB per pipe and 30 seconds. No terminal input/output is inherited.
pub fn capture(command: &mut Command, token: &CancelToken) -> Result<CommandOutput, Failure> {
    std::thread::scope(|scope| {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Owned here, INSIDE scope: unwinding kills pipes before scope joins.
        let mut process = OwnedProcess::spawn(command, token)?;
        let stdout = process
            .take_stdout()
            .ok_or_else(|| Failure::new(FailureKind::Protocol, "missing stdout"))?;
        let stderr = process
            .take_stderr()
            .ok_or_else(|| Failure::new(FailureKind::Protocol, "missing stderr"))?;
        let (tx, rx) = channel();
        let out_tx = tx.clone();
        std::thread::Builder::new()
            .name("capture-stdout".into())
            .spawn_scoped(scope, move || {
                let _ = out_tx.send(Stream::Stdout(read_pipe(stdout)));
            })
            .map_err(|error| Failure::new(FailureKind::ThreadStart, error.to_string()))?;
        std::thread::Builder::new()
            .name("capture-stderr".into())
            .spawn_scoped(scope, move || {
                let _ = tx.send(Stream::Stderr(read_pipe(stderr)));
            })
            .map_err(|error| Failure::new(FailureKind::ThreadStart, error.to_string()))?;
        let deadline = Instant::now() + DEADLINE;
        let mut stdout = None;
        let mut stderr = None;
        loop {
            if token.is_cancelled() {
                return Err(Failure::new(
                    FailureKind::Unavailable,
                    "configuration command cancelled",
                ));
            }
            if Instant::now() >= deadline {
                return Err(Failure::new(
                    FailureKind::Wait,
                    "configuration command timed out after 30 seconds",
                ));
            }
            let exited = process.has_exited()?;
            if exited {
                process.terminate()?; // descendants cannot retain the pipes
                match (stdout.take(), stderr.take()) {
                    (Some(stdout), Some(stderr)) => {
                        return Ok(CommandOutput {
                            status: process.wait()?,
                            stdout,
                            stderr,
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
                Err(error) => {
                    return Err(Failure::new(FailureKind::Disconnected, error.to_string()))
                }
            };
            let (slot, result) = match event {
                Stream::Stdout(result) => (&mut stdout, result),
                Stream::Stderr(result) => (&mut stderr, result),
            };
            *slot = Some(result.map_err(|error| Failure::new(FailureKind::Io, error.to_string()))?);
        }
    })
}
