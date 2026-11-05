use super::DevcontainerError;
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};
use strop_core::process::OwnedProcess;
use strop_core::worker::{CancelToken, FailureKind};

const IO_CHUNK: usize = 16 * 1024;
const STDERR_TAIL: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// Raw process output at the adapter boundary. Product logs must pass this
/// through the job redactor; this type intentionally has no Debug/Display.
pub struct ProcessLog {
    pub stream: LogStream,
    pub bytes: Vec<u8>,
}

/// Nominal process identity: executable plus argv, never a shell command.
#[derive(Debug, Clone)]
pub struct ProcessRequest {
    pub operation: &'static str,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub deadline: Duration,
    pub stdout_limit: usize,
    pub log_stdout: bool,
}

/// Only stdout needed for the structured result and a bounded stderr tail are
/// retained. Full logs stream through the callback instead.
pub struct ProcessOutput {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr_tail: Vec<u8>,
}

#[derive(Debug)]
pub enum ProcessFailure {
    Spawn(String),
    Io(String),
    Cancelled,
    TimedOut,
    OutputTooLarge,
}

/// The sole fakeable boundary. Tests fake this process adapter, never product
/// discovery, review, configuration, ownership or lifecycle behavior.
pub trait CliProcess: Send + Sync + 'static {
    fn run(
        &self,
        request: &ProcessRequest,
        token: &CancelToken,
        log: &mut dyn FnMut(ProcessLog),
    ) -> Result<ProcessOutput, ProcessFailure>;
}

/// Production runner using strop's owned process groups and caller deadline.
/// Both pipes are drained concurrently and both are streamed to the callback.
pub struct SupervisedCliProcess;
impl CliProcess for SupervisedCliProcess {
    fn run(
        &self,
        request: &ProcessRequest,
        token: &CancelToken,
        log: &mut dyn FnMut(ProcessLog),
    ) -> Result<ProcessOutput, ProcessFailure> {
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        std::thread::scope(|scope| {
            let mut process = OwnedProcess::spawn(&mut command, token).map_err(|failure| {
                if failure.kind == FailureKind::Spawn {
                    ProcessFailure::Spawn(failure.message)
                } else if token.is_cancelled() {
                    ProcessFailure::Cancelled
                } else {
                    ProcessFailure::Io(failure.message)
                }
            })?;
            let stdout = process
                .take_stdout()
                .ok_or_else(|| ProcessFailure::Io("missing child stdout".into()))?;
            let stderr = process
                .take_stderr()
                .ok_or_else(|| ProcessFailure::Io("missing child stderr".into()))?;
            let (sender, receiver) = sync_channel::<PipeEvent>(8);
            spawn_reader(scope, stdout, LogStream::Stdout, sender.clone())?;
            spawn_reader(scope, stderr, LogStream::Stderr, sender)?;
            let deadline = Instant::now() + request.deadline;
            let mut stdout = Vec::new();
            let mut stderr_tail = Vec::new();
            let mut open_pipes: u8 = 2;
            let mut terminated = false;
            loop {
                if token.is_cancelled() {
                    return Err(ProcessFailure::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Err(ProcessFailure::TimedOut);
                }
                let exited = process
                    .has_exited()
                    .map_err(|failure| ProcessFailure::Io(failure.message))?;
                if exited && !terminated {
                    // No descendant may retain a pipe after the direct child exits.
                    process
                        .terminate()
                        .map_err(|failure| ProcessFailure::Io(failure.message))?;
                    terminated = true;
                }
                if exited && open_pipes == 0 {
                    let status = process
                        .wait()
                        .map_err(|failure| ProcessFailure::Io(failure.message))?;
                    return Ok(ProcessOutput {
                        code: status.code(),
                        stdout,
                        stderr_tail,
                    });
                }
                match receiver.recv_timeout(Duration::from_millis(20)) {
                    Ok(PipeEvent::Chunk(stream, bytes)) => {
                        if stream == LogStream::Stdout {
                            if stdout.len().saturating_add(bytes.len()) > request.stdout_limit {
                                return Err(ProcessFailure::OutputTooLarge);
                            }
                            stdout.extend_from_slice(&bytes);
                            if request.log_stdout {
                                log(ProcessLog { stream, bytes });
                            }
                        } else {
                            stderr_tail.extend_from_slice(&bytes);
                            if stderr_tail.len() > STDERR_TAIL {
                                let overflow = stderr_tail.len() - STDERR_TAIL;
                                stderr_tail.drain(..overflow);
                            }
                            log(ProcessLog { stream, bytes });
                        }
                    }
                    Ok(PipeEvent::Closed(result)) => {
                        result.map_err(ProcessFailure::Io)?;
                        open_pipes = open_pipes.saturating_sub(1);
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) if !exited => {
                        std::thread::park_timeout(Duration::from_millis(20));
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(ProcessFailure::Io(
                            "process pipe observers disconnected".into(),
                        ));
                    }
                }
            }
        })
    }
}

enum PipeEvent {
    Chunk(LogStream, Vec<u8>),
    Closed(Result<(), String>),
}

fn spawn_reader<'scope, 'env>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    mut pipe: impl Read + Send + 'env,
    stream: LogStream,
    sender: SyncSender<PipeEvent>,
) -> Result<(), ProcessFailure> {
    std::thread::Builder::new()
        .name(match stream {
            LogStream::Stdout => "devcontainer-cli-stdout".to_string(),
            LogStream::Stderr => "devcontainer-cli-stderr".to_string(),
        })
        .spawn_scoped(scope, move || {
            let result = (|| {
                let mut buffer = vec![0u8; IO_CHUNK];
                loop {
                    match pipe.read(&mut buffer) {
                        Ok(0) => return Ok(()),
                        Ok(count) => {
                            if sender
                                .send(PipeEvent::Chunk(stream, buffer[..count].to_vec()))
                                .is_err()
                            {
                                return Ok(());
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(error) => return Err(error.to_string()),
                    }
                }
            })();
            let _ = sender.send(PipeEvent::Closed(result));
        })
        .map(|_| ())
        .map_err(|error| ProcessFailure::Io(format!("start pipe observer: {error}")))
}

/// Incremental redactor retaining enough tail to catch secrets split across
/// arbitrary process-read boundaries. It is used for logs and diagnostics only;
/// structured stdout is parsed before any redacted display.
pub(crate) struct StreamRedactor {
    secrets: Vec<Vec<u8>>,
    pending: Vec<u8>,
}
impl StreamRedactor {
    pub fn new(mut secrets: Vec<String>) -> Self {
        secrets.retain(|secret| !secret.is_empty());
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        Self {
            secrets: secrets.into_iter().map(String::into_bytes).collect(),
            pending: Vec::new(),
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(bytes);
        replace_secrets(&mut self.pending, &self.secrets);
        // Retain only a suffix that can actually become a secret. Redacting
        // before this split catches every complete secret without splitting a
        // possible prefix across emitted and retained buffers.
        let pending = &self.pending;
        let retain = self
            .secrets
            .iter()
            .flat_map(|secret| {
                let maximum = secret.len().saturating_sub(1).min(pending.len());
                (1..=maximum).filter(|length| pending.ends_with(&secret[..*length]))
            })
            .max()
            .unwrap_or(0);
        let mut output = std::mem::take(&mut self.pending);
        self.pending = output.split_off(output.len() - retain);
        output
    }
    pub fn finish(&mut self) -> Vec<u8> {
        let mut output = std::mem::take(&mut self.pending);
        replace_secrets(&mut output, &self.secrets);
        output
    }
    pub fn redact_complete(&self, text: &str) -> String {
        let mut bytes = text.as_bytes().to_vec();
        replace_secrets(&mut bytes, &self.secrets);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

fn replace_secrets(bytes: &mut Vec<u8>, secrets: &[Vec<u8>]) {
    for secret in secrets {
        let mut start = 0;
        while start + secret.len() <= bytes.len() {
            let Some(relative) = bytes[start..]
                .windows(secret.len())
                .position(|window| window == secret)
            else {
                break;
            };
            let found = start + relative;
            bytes.splice(found..found + secret.len(), b"<redacted>".iter().copied());
            start = found + "<redacted>".len();
        }
    }
}

pub(crate) fn process_error(
    request: &ProcessRequest,
    failure: ProcessFailure,
    redactor: &StreamRedactor,
) -> DevcontainerError {
    match failure {
        ProcessFailure::Cancelled => DevcontainerError::Cancelled,
        ProcessFailure::TimedOut => DevcontainerError::Timeout {
            operation: request.operation.into(),
            deadline: request.deadline,
        },
        ProcessFailure::OutputTooLarge => DevcontainerError::OutputTooLarge {
            operation: request.operation.into(),
            limit: request.stdout_limit,
        },
        ProcessFailure::Spawn(detail) | ProcessFailure::Io(detail) => DevcontainerError::Process {
            detail: redactor.redact_complete(&detail),
        },
    }
}
