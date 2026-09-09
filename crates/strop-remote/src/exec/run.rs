//! `run`: one bounded, finite remote command, end to end.
//!
//! Local side ownership: the ssh process is spawned through
//! `strop_core::process::capture_with` with [`StdinPolicy::Held`] — its
//! stdin writer is the lifetime lease and stays open for the whole
//! command, because closing SSH stdin means *cancel* to the remote
//! supervisor, never "finish up". The capture machinery owns the local
//! child in its own process group; the cancellation hook SIGKILLs that
//! group (which closes the lease kernel-side and triggers the remote
//! teardown), and the PID is revoked before it is reaped.
//!
//! Bounds: stdout keeps its first 16 MiB, stderr its first 64 KiB plus
//! the final 2 KiB (so the supervisor's nonce-marked status record
//! survives a chatty worker), and the whole exchange is bounded by the
//! command's deadline. Overflow truncates and is reported through the
//! dropped counters rather than corrupting a record stream silently.

use super::spec::Spec;
use super::supervisor;
use super::{
    CommandOutput, RemoteCommand, RemoteCommandError, RemoteExitStatus, StdinMode,
    SupervisionOutcome,
};
use crate::address::RemoteEndpoint;
use std::time::Duration;
use strop_core::process::{capture_with, CaptureError, CapturePolicy, StdinPolicy};
use strop_core::worker::CancelToken;

pub(super) const STDOUT_LIMIT: u64 = 16 * 1024 * 1024;
pub(super) const STDERR_LIMIT: u64 = 64 * 1024;
/// Reserved tail so the supervisor's final status line — written after
/// the worker's own stderr — is never the part a flood discards.
pub(super) const STDERR_TAIL: u64 = 2 * 1024;

/// Default wall-clock budget for one finite remote command.
pub(super) const DEFAULT_DEADLINE: Duration = Duration::from_secs(120);

/// Build the local ssh command plus the key that identifies this
/// session's supervisor records. Native worker-side setup captures interpreter
/// configuration here; RemoteCommand constructors remain pure and do not spawn.
pub(super) fn supervised(
    endpoint: &RemoteEndpoint,
    command: &RemoteCommand,
    mode: StdinMode,
) -> Result<(std::process::Command, super::SupervisionKey), RemoteCommandError> {
    let key = super::SupervisionKey::generate();
    let spec = Spec::encode(
        mode,
        key.nonce(),
        command.program(),
        command.args(),
        command.cwd(),
    )?;
    let python = super::python::PythonInterpreter::from_environment()?;
    let line = supervisor::command_line(&spec.encoded()?, &python);
    Ok((crate::ssh::exec_command(endpoint, &line), key))
}

/// Run one finite remote command to completion on a worker.
pub(super) fn run(
    endpoint: &RemoteEndpoint,
    command: &RemoteCommand,
    token: &CancelToken,
) -> Result<CommandOutput, RemoteCommandError> {
    run_input(endpoint, command, token, None)
}

pub(super) fn run_input(
    endpoint: &RemoteEndpoint,
    command: &RemoteCommand,
    token: &CancelToken,
    chunks: Option<&[&[u8]]>,
) -> Result<CommandOutput, RemoteCommandError> {
    let mode = if chunks.is_some() {
        StdinMode::Relayed
    } else {
        StdinMode::Finite
    };
    let (mut process, key) = supervised(endpoint, command, mode)?;
    let policy = CapturePolicy {
        stdout_limit: STDOUT_LIMIT,
        stderr_limit: STDERR_LIMIT,
        stderr_tail: STDERR_TAIL,
        deadline: command.deadline(),
        stdin: chunks.map_or(StdinPolicy::Held, StdinPolicy::HeldInput),
    };
    let output = match capture_with(&mut process, token, &policy) {
        Ok(output) => output,
        Err(CaptureError::Cancelled) => {
            return Err(RemoteCommandError::Cancelled {
                diagnostics: "cancellation observed while the command ran".into(),
            })
        }
        Err(CaptureError::TimedOut(deadline)) => {
            return Err(RemoteCommandError::Timeout {
                seconds: deadline.as_secs(),
            })
        }
        Err(CaptureError::Spawn(message)) => return Err(RemoteCommandError::Spawn { message }),
        Err(CaptureError::Failure(failure)) => {
            return Err(RemoteCommandError::Local {
                message: format!("{:?}: {}", failure.kind, failure.message),
            })
        }
    };
    classify(output, &key)
}

/// Turn one completed local capture into the typed remote result: the
/// supervisor's nonce-marked records are authoritative, ssh's exit
/// code is the fallback when supervision never reported.
pub(super) fn classify(
    mut output: strop_core::process::CommandOutput,
    key: &super::SupervisionKey,
) -> Result<CommandOutput, RemoteCommandError> {
    let ssh_code = output.status.code();
    let records = key.records(&output.stderr);
    key.remove_records(&mut output.stderr);
    // The program never started: the worker's own pre-exec diagnostic
    // is the actionable one, whatever exit code the anchor recorded.
    if let Some(SupervisionOutcome::LaunchFailure(detail)) = records
        .iter()
        .find(|record| matches!(record, SupervisionOutcome::LaunchFailure(_)))
    {
        return Err(RemoteCommandError::Launch {
            diagnostics: detail.clone(),
        });
    }
    match records.last() {
        Some(SupervisionOutcome::Exited(code)) => Ok(CommandOutput {
            status: RemoteExitStatus::Exited(*code),
            stdout: output.stdout,
            stderr: output.stderr,
            stdout_dropped: output.stdout_dropped,
            stderr_dropped: output.stderr_dropped,
            stdin_error: output.stdin_error.map(|error| error.to_string()),
        }),
        Some(SupervisionOutcome::Signaled(signal)) => Ok(CommandOutput {
            status: RemoteExitStatus::Signaled(*signal),
            stdout: output.stdout,
            stderr: output.stderr,
            stdout_dropped: output.stdout_dropped,
            stderr_dropped: output.stderr_dropped,
            stdin_error: output.stdin_error.map(|error| error.to_string()),
        }),
        Some(SupervisionOutcome::Cancelled) => Err(RemoteCommandError::Cancelled {
            diagnostics: tail(&output.stderr),
        }),
        Some(SupervisionOutcome::SupervisorError(stage)) => Err(RemoteCommandError::Supervisor {
            stage: stage.clone(),
            diagnostics: tail(&output.stderr),
        }),
        Some(SupervisionOutcome::LaunchFailure(detail)) => Err(RemoteCommandError::Launch {
            diagnostics: detail.clone(),
        }),
        None => classify_without_record(ssh_code, &output.stderr),
    }
}

/// No supervisor record reached us: the supervision layer itself never
/// came up, or died before reporting. ssh's own exit code is the only
/// evidence, and its classic meanings get actionable errors.
fn classify_without_record(
    ssh_code: Option<i32>,
    stderr: &[u8],
) -> Result<CommandOutput, RemoteCommandError> {
    let diagnostics = String::from_utf8_lossy(stderr).trim().to_string();
    match ssh_code {
        Some(127) | Some(126) => Err(RemoteCommandError::MissingPython { diagnostics }),
        Some(255) | None => Err(RemoteCommandError::Transport {
            exit: ssh_code,
            diagnostics,
        }),
        Some(code) => Err(RemoteCommandError::Supervisor {
            stage: "exited-without-report".into(),
            diagnostics: format!("supervised session ended with ssh exit {code}: {diagnostics}"),
        }),
    }
}

/// The tail of stderr, record lines first — bounded context for typed
/// failures, never an unbounded dump.
fn tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim_end();
    let mut start = text.len().saturating_sub(2048);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].trim().to_string()
}
