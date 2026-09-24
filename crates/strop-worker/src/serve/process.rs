//! Exec request serving (0058 WK04): admitted finite commands and leased
//! services run under [`crate::exec`]'s fixed supervisor semantics — the
//! worker owns the process, pipes and wait state; child stdout/stderr
//! leave as bounded stream chunks, never as control bytes, and the exit
//! arrives as a typed [`Event::ExecExit`].
//!
//! Scope honesty: environment overlays and PTYs are not admitted by this
//! serving slice (typed refusals, never a guessed inherit); WK10/WK12 own
//! those consumers. Cancellation is the request's own token during launch
//! and the exec's lease afterwards — half-close is EOF, never revocation.

use std::io::{Read, Write};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::thread;

use strop_core::worker::{CancelHandle, CancelReason, CancelToken};
use strop_worker_protocol::message::Capability;
use strop_worker_protocol::{
    Event, ExecId, ExecSpec, ExitStatus, Refusal, RequestId, ResultOutcome, StreamChunk, StreamId,
};
use strop_workspace::operation::{FsFailure, FsFailureKind};

use super::{failure, Inbound, SessionState};

/// One live exec's lease state. The cancel handle revokes the supervised
/// group (TERM/grace/KILL/reap, in that order); the stdin stream id names
/// the relay channel in the session's inbound table.
pub(crate) struct ExecEntry {
    pub cancel: CancelHandle,
    stdin_stream: Option<StreamId>,
}

/// The relayed-stdin queue bound per exec; a child that stops draining is
/// revoked rather than stalling protocol control behind its data.
const STDIN_QUEUE: usize = 256;
/// stdout/stderr pump read size; chunks stay well under the wire ceiling.
const PUMP_CHUNK: usize = 32 * 1024;

/// Which exec owns one stdin stream, for flow-pressure revocation.
pub(crate) fn exec_for_stream<W: Write>(
    shared: &Arc<SessionState<W>>,
    stream: StreamId,
) -> Option<ExecId> {
    shared
        .execs
        .lock()
        .iter()
        .find_map(|(id, entry)| (entry.stdin_stream == Some(stream)).then_some(*id))
}

/// Revoke one exec's lease without a wire reply (teardown, backpressure).
pub(crate) fn revoke<W: Write>(shared: &Arc<SessionState<W>>, exec: ExecId) {
    if let Some(entry) = shared.execs.lock().remove(&exec) {
        entry.cancel.cancel(CancelReason::Dismissed);
    }
}

#[cfg(unix)]
fn exec_failure(error: crate::exec::ExecError) -> FsFailure {
    use crate::exec::ExecError;
    let kind = match error {
        ExecError::Invalid { .. } => FsFailureKind::InvalidPath,
        ExecError::Cancelled => FsFailureKind::Cancelled,
        ExecError::Unavailable => FsFailureKind::Unsupported,
        ExecError::Spawn { .. } | ExecError::Launch { .. } | ExecError::Supervisor { .. } => {
            FsFailureKind::Io
        }
    };
    failure(kind, error.to_string())
}

/// Admit and launch one exec, then own its streams and settlement. The
/// `ExecStarted` reply names every stream; pumps and the supervisor run
/// on their own threads and publish through the shared writer.
#[cfg(unix)]
pub(crate) fn exec<W: Write + Send + 'static>(
    shared: &Arc<SessionState<W>>,
    request: RequestId,
    token: &CancelToken,
    spec: ExecSpec,
) {
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;
    let reply = |outcome| {
        shared.send(&strop_worker_protocol::WorkerMessage::Result {
            id: request,
            outcome,
        })
    };
    if token.is_cancelled() {
        return reply(ResultOutcome::Failed {
            failure: failure(FsFailureKind::Cancelled, "exec cancelled before launch"),
        });
    }
    if !spec.env.is_empty() {
        return reply(ResultOutcome::Refused {
            refusal: Refusal::Limit {
                message: "environment overlays are not admitted by this worker slice".into(),
            },
        });
    }
    if spec.pty.is_some() {
        return reply(ResultOutcome::Refused {
            refusal: Refusal::Capability {
                capability: Capability::Pty,
            },
        });
    }
    if shared.execs.lock().len() >= shared.limits.max_exec_processes {
        return reply(ResultOutcome::Refused {
            refusal: Refusal::Busy {
                message: "exec process bound reached".into(),
            },
        });
    }
    let program = OsStr::from_bytes(&spec.program);
    let args = spec
        .argv
        .iter()
        .map(|arg| OsString::from(OsStr::from_bytes(arg)));
    let cwd = std::path::PathBuf::from(OsString::from(OsStr::from_bytes(&spec.cwd)));
    let mut admitted = match crate::exec::ExecSpec::new(program, args, &cwd) {
        Ok(admitted) => admitted,
        Err(error) => {
            return reply(ResultOutcome::Failed {
                failure: exec_failure(error),
            });
        }
    };
    if spec.service {
        admitted = admitted.relayed_stdin();
    }
    // The exec's own lease token: it outlives this request's token, which
    // is retired when the reply lands.
    let (lease, handle) = CancelToken::standalone();
    let running = match crate::exec::launch(&admitted, &lease) {
        Ok(running) => running,
        Err(error) => {
            return reply(ResultOutcome::Failed {
                failure: exec_failure(error),
            });
        }
    };
    let mut running = running;
    let exec = shared.mint_exec();
    let stdin = if spec.service {
        let stream = shared.mint_stream();
        let (sender, receiver) = sync_channel::<StreamChunk>(STDIN_QUEUE);
        let mut child_stdin = running.take_stdin();
        let worker = Arc::clone(shared);
        thread::spawn(move || {
            while let Ok(chunk) = receiver.recv() {
                let Some(writer) = child_stdin.as_mut() else {
                    break;
                };
                if writer.write_all(&chunk.bytes).is_err() {
                    break;
                }
                if chunk.last {
                    break;
                }
            }
            // Channel closed or `last`: EOF to the child, not revocation.
            drop(child_stdin);
            let _ = worker;
        });
        shared.inbound.lock().insert(
            stream,
            Inbound::ExecStdin {
                sender,
                next_sequence: 0,
            },
        );
        Some(stream)
    } else {
        None
    };
    shared.execs.lock().insert(
        exec,
        ExecEntry {
            cancel: handle,
            stdin_stream: stdin,
        },
    );
    let stdout = shared.mint_stream();
    let stderr = shared.mint_stream();
    // The reply lands before any pump chunk can be written.
    reply(ResultOutcome::ExecStarted {
        exec,
        stdin,
        stdout,
        stderr,
    });
    pump(shared, stdout, running.take_stdout());
    pump(shared, stderr, running.take_stderr());
    let worker = Arc::clone(shared);
    thread::spawn(move || {
        let status = match running.wait() {
            Ok(crate::exec::Settlement::Recorded(crate::exec::StatusRecord::Exited(code))) => {
                ExitStatus::Exit(code as i32)
            }
            Ok(crate::exec::Settlement::Recorded(crate::exec::StatusRecord::Signaled(signal))) => {
                ExitStatus::Signal(signal as i32)
            }
            // Revoked carries no attested exit; a launch that failed its
            // handshake never reaches wait. Neither is silently mapped to
            // a code.
            Ok(crate::exec::Settlement::Revoked)
            | Ok(crate::exec::Settlement::Recorded(crate::exec::StatusRecord::LaunchFailed(_)))
            | Err(_) => ExitStatus::Lost,
        };
        worker.execs.lock().remove(&exec);
        worker.send(&strop_worker_protocol::WorkerMessage::Event {
            event: Event::ExecExit { exec, status },
        });
    });
}

#[cfg(not(unix))]
pub(crate) fn exec<W: Write + Send + 'static>(
    _shared: &Arc<SessionState<W>>,
    _request: RequestId,
    _token: &CancelToken,
    spec: ExecSpec,
) -> ResultOutcome {
    ResultOutcome::Refused {
        refusal: Refusal::Capability {
            capability: if spec.service {
                Capability::ExecService
            } else {
                Capability::ExecFinite
            },
        },
    }
}

/// Pump one child output pipe to its stream: bounded chunks, ordered, and
/// a `last` chunk at EOF. A dead session stops the pump; the supervisor
/// thread still owns settlement.
#[cfg(unix)]
fn pump<W: Write + Send + 'static>(
    shared: &Arc<SessionState<W>>,
    stream: StreamId,
    pipe: Option<impl Read + Send + 'static>,
) {
    let Some(mut pipe) = pipe else { return };
    let worker = Arc::clone(shared);
    thread::spawn(move || {
        let mut buffer = vec![0_u8; PUMP_CHUNK];
        let mut sequence = 0_u64;
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => {
                    worker.send_chunk(&StreamChunk {
                        stream,
                        sequence,
                        last: true,
                        bytes: Vec::new(),
                    });
                    return;
                }
                Ok(count) => {
                    worker.send_chunk(&StreamChunk {
                        stream,
                        sequence,
                        last: false,
                        bytes: buffer[..count].to_vec(),
                    });
                    sequence += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return,
            }
            if worker.stop.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
        }
    });
}

/// Half-close one exec's stdin: drop the relay channel so the pump
/// delivers EOF. Distinct from revocation — the target keeps running.
pub(crate) fn half_close<W: Write>(shared: &Arc<SessionState<W>>, exec: ExecId) -> ResultOutcome {
    let stream = {
        let execs = shared.execs.lock();
        execs.get(&exec).and_then(|entry| entry.stdin_stream)
    };
    let Some(stream) = stream else {
        return unknown_exec(exec);
    };
    // Dropping the last sender closes the pump's channel; the pump then
    // drops the child's stdin.
    shared.inbound.lock().remove(&stream);
    ResultOutcome::Done
}

/// Cancel one exec: revoke its lease with the supervisor's ordering. The
/// reply is prompt; the `ExecExit` event follows the actual teardown.
pub(crate) fn cancel<W: Write>(shared: &Arc<SessionState<W>>, exec: ExecId) -> ResultOutcome {
    let Some(entry) = shared.execs.lock().remove(&exec) else {
        return unknown_exec(exec);
    };
    entry.cancel.cancel(CancelReason::Dismissed);
    ResultOutcome::Done
}

fn unknown_exec(exec: ExecId) -> ResultOutcome {
    ResultOutcome::Refused {
        refusal: Refusal::UnknownHandle {
            message: format!("exec {}", exec.0),
        },
    }
}
