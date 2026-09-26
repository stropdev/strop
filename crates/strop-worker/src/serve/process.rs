//! Exec request serving (0058 WK04): admitted finite commands and leased
//! services run under [`crate::exec`]'s fixed supervisor semantics — the
//! worker owns the process, pipes and wait state; child stdout/stderr
//! leave as bounded stream chunks, never as control bytes, and the exit
//! arrives as a typed [`Event::ExecExit`].
//!
//! Environment overlays are admitted typed and bounded (WK10). PTYs are
//! owned here too (WK12): spawn with the admitted geometry, one merged
//! output stream (the stderr stream opens and closes empty — a PTY has
//! no separate stderr), input chunks written in stream order with a
//! delivered acknowledgment per chunk ([`Event::ExecInput`]), resize
//! applied in input order with its reply as the ordered geometry
//! boundary, and no half-close. Cancellation is the request's own
//! token during launch and the exec's lease afterwards.
//!
//! Scheduling (WK11): the exec-permit budget is the session scheduler's
//! ([`Scheduler::acquire_exec`] — the check and the count share one
//! lock), and output pumps ride the data lane with the exec lease token,
//! so revocation wakes a lane-blocked pump and every stream still ends
//! with its terminal chunk (or by session death), never silently.

use std::io::{Read, Write};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::thread;

use parking_lot::Mutex;
use strop_core::worker::{CancelHandle, CancelReason, CancelToken};
#[cfg(not(unix))]
use strop_worker_protocol::message::Capability;
use strop_worker_protocol::{
    Event, ExecId, ExecSpec, ExitStatus, Refusal, RequestId, ResultOutcome, StreamChunk, StreamId,
};
use strop_workspace::operation::{FsFailure, FsFailureKind};

#[cfg(unix)]
use super::stream_window::{StreamRegistration, WindowUse};
use super::{failure, Inbound, PushChunk, SessionState};
#[cfg(unix)]
mod pty;
#[cfg(unix)]
pub(crate) use pty::enqueue_resize;

/// The exec's cancellation authority. Output pumps hold this owner after
/// the child exits: dropping the handle when the exit event is published
/// must not revoke the last bytes before the pumps finish framing them.
pub(crate) struct ExecLease(Mutex<Option<CancelHandle>>);

impl ExecLease {
    fn new(handle: CancelHandle) -> Self {
        Self(Mutex::new(Some(handle)))
    }

    pub(crate) fn cancel(&self, reason: CancelReason) {
        if let Some(handle) = self.0.lock().take() {
            handle.cancel(reason);
        }
    }
}

/// One live exec's lease state and relayed input identity.
pub(crate) struct ExecEntry {
    pub cancel: Arc<ExecLease>,
    stdin_stream: Option<StreamId>,
    /// PTY control endpoint (WK12): input chunks and ordered resizes
    /// ride one queue so both reach the terminal in admission order.
    pty: Option<PtyEntry>,
}

pub(crate) struct PtyEntry {
    control: std::sync::mpsc::SyncSender<PtyControl>,
}

/// The ordered input side of one PTY exec: stream chunks and resize
/// requests in read-loop admission order. The input thread owns the
/// master writes and the resize ioctl, so a resize reply is the exact
/// geometry boundary for everything after it.
pub(crate) enum PtyControl {
    Input(StreamChunk),
    Resize {
        geometry: strop_worker_protocol::PtyGeometry,
        request: RequestId,
    },
}

/// The relayed-stdin queue bound per exec; a child that stops draining is
/// revoked rather than stalling protocol control behind its data.
const STDIN_QUEUE: usize = 256;
/// stdout/stderr pump read size; chunks stay well under the wire ceiling.
const PUMP_CHUNK: usize = 32 * 1024;

/// Which exec owns one stdin stream, for flow-pressure revocation.
pub(crate) fn exec_for_stream(shared: &Arc<SessionState>, stream: StreamId) -> Option<ExecId> {
    shared
        .execs
        .lock()
        .iter()
        .find_map(|(id, entry)| (entry.stdin_stream == Some(stream)).then_some(*id))
}

/// Revoke one exec's lease without a wire reply (teardown, backpressure).
pub(crate) fn revoke(shared: &Arc<SessionState>, exec: ExecId) {
    if let Some(entry) = shared.execs.lock().remove(&exec) {
        shared.scheduler.release_exec();
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
/// on their own threads and publish through the session scheduler.
#[cfg(unix)]
pub(crate) fn exec(
    shared: &Arc<SessionState>,
    request: RequestId,
    token: &CancelToken,
    spec: ExecSpec,
) {
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;
    let reply = |outcome| {
        shared.send(strop_worker_protocol::WorkerMessage::Result {
            id: request,
            outcome,
        })
    };
    if token.is_cancelled() {
        return reply(ResultOutcome::Failed {
            failure: failure(FsFailureKind::Cancelled, "exec cancelled before launch"),
        });
    }
    // Environment overlays are admitted typed and bounded (WK10): the
    // supervisor's `with_env_overlay` refuses a malformed or overlong
    // table rather than guessing an inherit/skip.
    let env: Vec<(Vec<u8>, Vec<u8>)> = spec
        .env
        .iter()
        .map(|var| (var.name.clone(), var.value.clone()))
        .collect();
    if let Some(geometry) = spec.pty {
        return pty::exec_pty(shared, request, token, spec, geometry);
    }
    if let Err(refusal) = shared.scheduler.acquire_exec() {
        return reply(ResultOutcome::Refused { refusal });
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
            shared.scheduler.release_exec();
            return reply(ResultOutcome::Failed {
                failure: exec_failure(error),
            });
        }
    };
    if !env.is_empty() {
        admitted = match admitted.with_env_overlay(env) {
            Ok(admitted) => admitted,
            Err(error) => {
                shared.scheduler.release_exec();
                return reply(ResultOutcome::Failed {
                    failure: exec_failure(error),
                });
            }
        };
    }
    if spec.service {
        admitted = admitted.relayed_stdin();
    }
    // The exec's own lease token: it outlives this request's token, which
    // is retired when the reply lands. The pumps hold clones so a lease
    // revocation wakes a lane-blocked pump (WK11).
    let (lease, handle) = CancelToken::standalone();
    let running = match crate::exec::launch(&admitted, &lease) {
        Ok(running) => running,
        Err(error) => {
            shared.scheduler.release_exec();
            return reply(ResultOutcome::Failed {
                failure: exec_failure(error),
            });
        }
    };
    let mut running = running;
    let owner = Arc::new(ExecLease::new(handle));
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
            cancel: Arc::clone(&owner),
            stdin_stream: stdin,
            pty: None,
        },
    );
    let stdout = shared.mint_stream();
    let stderr = shared.mint_stream();
    let stdout_window = StreamRegistration::new(shared, stdout);
    let stderr_window = StreamRegistration::new(shared, stderr);
    // The reply lands before any pump chunk can be written.
    reply(ResultOutcome::ExecStarted {
        exec,
        stdin,
        stdout,
        stderr,
    });
    let stdout_pump = pump(
        shared,
        stdout,
        running.take_stdout(),
        lease.clone(),
        Arc::clone(&owner),
        stdout_window,
    );
    let stderr_pump = pump(
        shared,
        stderr,
        running.take_stderr(),
        lease,
        owner,
        stderr_window,
    );
    settle(shared, exec, running, [stdout_pump, stderr_pump]);
}

/// One admitted exec's settlement: report the attested terminal status
/// only after both output pumps finish and the writer actually flushes
/// their terminal chunks. A control-lane exit must never overtake its
/// own data-lane output.
#[cfg(unix)]
fn settle(
    shared: &Arc<SessionState>,
    exec: ExecId,
    running: crate::exec::Running,
    pumps: [Option<thread::JoinHandle<()>>; 2],
) {
    let worker = Arc::clone(shared);
    thread::spawn(move || {
        let status = match running.wait() {
            Ok(crate::exec::Settlement::Recorded(crate::exec::StatusRecord::Exited(code))) => {
                ExitStatus::Exit(code as i32)
            }
            Ok(crate::exec::Settlement::Recorded(crate::exec::StatusRecord::Signaled(signal))) => {
                ExitStatus::Signal(signal as i32)
            }
            // Revoked carries no attested exit; launch failures never
            // enter wait. Neither is silently mapped to success.
            Ok(crate::exec::Settlement::Revoked) | Err(_) => ExitStatus::Lost,
        };
        let mut outputs_ok = true;
        for pump in pumps.into_iter().flatten() {
            outputs_ok &= pump.join().is_ok();
        }
        if !outputs_ok {
            worker.halt();
            return;
        }
        let entry = { worker.execs.lock().remove(&exec) };
        if let Some(entry) = entry {
            if let Some(stream) = entry.stdin_stream {
                worker.inbound.lock().remove(&stream);
            }
            worker.scheduler.release_exec();
        }
        if worker.scheduler.flush_data(&worker.stop) {
            worker.send(strop_worker_protocol::WorkerMessage::Event {
                event: Event::ExecExit { exec, status },
            });
        }
    });
}

#[cfg(not(unix))]
pub(crate) fn exec(
    _shared: &Arc<SessionState>,
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

#[cfg(not(unix))]
pub(crate) fn enqueue_resize(
    shared: &Arc<SessionState>,
    request: RequestId,
    _exec: ExecId,
    _geometry: strop_worker_protocol::PtyGeometry,
) {
    shared.send(strop_worker_protocol::WorkerMessage::Result {
        id: request,
        outcome: ResultOutcome::Refused {
            refusal: Refusal::Capability {
                capability: Capability::Pty,
            },
        },
    });
}

/// Pump one child output pipe to its stream: bounded chunks, ordered,
/// and a `last` chunk at EOF — also on revocation (the lane-blocked pump
/// wakes through the lease token) and on a read fault, so the stream
/// always ends with its terminal chunk while the session lives. The
/// supervisor thread still owns the exit settlement.
#[cfg(unix)]
fn pump(
    shared: &Arc<SessionState>,
    stream: StreamId,
    pipe: Option<impl Read + Send + 'static>,
    lease: CancelToken,
    owner: Arc<ExecLease>,
    window: StreamRegistration,
) -> Option<thread::JoinHandle<()>> {
    let Some(mut pipe) = pipe else {
        shared.end_stream(stream, 0);
        return None;
    };
    let worker = Arc::clone(shared);
    Some(thread::spawn(move || {
        let _owner = owner;
        let mut buffer = vec![0_u8; PUMP_CHUNK];
        let mut sequence = 0_u64;
        loop {
            let deliver = match window.take(&lease) {
                WindowUse::Emit => true,
                WindowUse::Discard => false,
                WindowUse::Stopped => {
                    worker.end_stream(stream, sequence);
                    return;
                }
            };
            let read = loop {
                match pipe.read(&mut buffer) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    result => break result,
                }
            };
            match read {
                Ok(0) => {
                    worker.end_stream(stream, sequence);
                    return;
                }
                Ok(_) if !deliver => {}
                Ok(count) => {
                    match worker.push_chunk(
                        StreamChunk {
                            stream,
                            sequence,
                            last: false,
                            bytes: buffer[..count].to_vec(),
                        },
                        Some(&lease),
                    ) {
                        PushChunk::Enqueued => sequence += 1,
                        // Revoked under pressure: end the stream honestly
                        // — the terminal bypass never queues behind data.
                        PushChunk::Cancelled => {
                            worker.end_stream(stream, sequence);
                            return;
                        }
                        PushChunk::Halted => return,
                    }
                }
                Err(error) => {
                    worker.note(format_args!("exec pump: {error}"));
                    worker.end_stream(stream, sequence);
                    return;
                }
            }
            if worker.stop.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
        }
    }))
}

/// Half-close one exec's stdin: drop the relay channel so the pump
/// delivers EOF. Distinct from revocation — the target keeps running.
/// A PTY has no input half-close (WK12): the refusal is typed, never a
/// guessed EOF byte.
pub(crate) fn half_close(shared: &Arc<SessionState>, exec: ExecId) -> ResultOutcome {
    let (stream, is_pty) = {
        let execs = shared.execs.lock();
        match execs.get(&exec) {
            Some(entry) => (entry.stdin_stream, entry.pty.is_some()),
            None => (None, false),
        }
    };
    if is_pty {
        return ResultOutcome::Refused {
            refusal: Refusal::Limit {
                message: "PTY input has no half-close".into(),
            },
        };
    }
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
pub(crate) fn cancel(shared: &Arc<SessionState>, exec: ExecId) -> ResultOutcome {
    let Some(entry) = shared.execs.lock().remove(&exec) else {
        return unknown_exec(exec);
    };
    shared.scheduler.release_exec();
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
