//! Worker-owned PTY spawn, ordered input/resize and merged VT output.

use super::*;
use std::os::fd::AsRawFd;

/// Admit and launch one PTY exec (0058 WK12): the target owns a fresh
/// pseudo-terminal as its controlling terminal with the admitted
/// geometry. Its merged output is the `stdout` stream (the `stderr`
/// stream opens and immediately ends — a PTY has no separate stderr);
/// input chunks ride the ordered control queue with a delivered
/// [`Event::ExecInput`] acknowledgment per chunk; resizes are applied
/// in that same order and their reply is the geometry boundary.
pub(super) fn exec_pty(
    shared: &Arc<SessionState>,
    request: RequestId,
    token: &CancelToken,
    spec: ExecSpec,
    geometry: strop_worker_protocol::PtyGeometry,
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
    if geometry.columns == 0 || geometry.rows == 0 {
        return reply(ResultOutcome::Refused {
            refusal: Refusal::Limit {
                message: "PTY geometry must be non-zero".into(),
            },
        });
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
    let env: Vec<(Vec<u8>, Vec<u8>)> = spec
        .env
        .iter()
        .map(|var| (var.name.clone(), var.value.clone()))
        .collect();
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
    let (lease, handle) = CancelToken::standalone();
    let size = crate::exec::PtySize {
        columns: geometry.columns,
        rows: geometry.rows,
    };
    let pty = match crate::exec::launch_pty(&admitted, size, &lease) {
        Ok(pty) => pty,
        Err(error) => {
            shared.scheduler.release_exec();
            return reply(ResultOutcome::Failed {
                failure: exec_failure(error),
            });
        }
    };
    let mut master_in = match pty.master().try_clone() {
        Ok(master) => master,
        Err(error) => {
            shared.scheduler.release_exec();
            return reply(ResultOutcome::Failed {
                failure: super::super::io_failure(error),
            });
        }
    };
    let master_out = match pty.master().try_clone() {
        Ok(master) => master,
        Err(error) => {
            shared.scheduler.release_exec();
            return reply(ResultOutcome::Failed {
                failure: super::super::io_failure(error),
            });
        }
    };
    let owner = Arc::new(ExecLease::new(handle));
    let exec = shared.mint_exec();
    let stdin = shared.mint_stream();
    let stdout = shared.mint_stream();
    let stderr = shared.mint_stream();
    let (sender, receiver) = sync_channel::<PtyControl>(STDIN_QUEUE);
    shared.inbound.lock().insert(
        stdin,
        Inbound::PtyStdin {
            sender: sender.clone(),
            next_sequence: 0,
            closed: false,
        },
    );
    shared.execs.lock().insert(
        exec,
        ExecEntry {
            cancel: Arc::clone(&owner),
            stdin_stream: Some(stdin),
            pty: Some(PtyEntry { control: sender }),
        },
    );
    // The reply lands before any pump chunk can be written.
    reply(ResultOutcome::ExecStarted {
        exec,
        stdin: Some(stdin),
        stdout,
        stderr,
    });
    // A PTY merges stderr into its one output: the stderr stream opens
    // and ends, truthfully empty.
    shared.end_stream(stderr, 0);
    // Input + resize in admission order. A delivered chunk is
    // acknowledged; a resize reply is the ordered geometry boundary.
    // Backpressure lives on the bounded queue, never on the read loop.
    {
        let worker = Arc::clone(shared);
        thread::spawn(move || {
            while let Ok(control) = receiver.recv() {
                match control {
                    PtyControl::Input(chunk) => {
                        if write_all(&mut master_in, &chunk.bytes).is_err() {
                            break;
                        }
                        worker.send(strop_worker_protocol::WorkerMessage::Event {
                            event: Event::ExecInput {
                                exec,
                                sequence: chunk.sequence,
                            },
                        });
                    }
                    PtyControl::Resize { geometry, request } => {
                        let size = crate::exec::PtySize {
                            columns: geometry.columns,
                            rows: geometry.rows,
                        };
                        let outcome = match crate::exec::resize_pty(&master_in, size) {
                            Ok(()) => ResultOutcome::Done,
                            Err(error) => ResultOutcome::Failed {
                                failure: exec_failure(error),
                            },
                        };
                        worker.send(strop_worker_protocol::WorkerMessage::Result {
                            id: request,
                            outcome,
                        });
                    }
                }
                if worker.stop.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
            }
        });
    }
    let window = StreamRegistration::new(shared, stdout);
    let output_pump = pty_output(shared, stdout, master_out, lease, owner, window);
    settle(shared, exec, pty.into_running(), [Some(output_pump), None]);
}

/// EINTR-safe write of one full input chunk to the terminal.
fn write_all(master: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        match master.write(&bytes[written..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "terminal accepted no input",
                ));
            }
            Ok(count) => written += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Pump one PTY master to its output stream: bounded chunks, ordered,
/// `last` when the terminal reports end-of-output (EIO once the last
/// slave holder is gone). Backpressure lands on this producer through
/// the session data lane — the pump stops reading, the child's own
/// writes fill the PTY buffer and stall it; protocol control never waits
/// on this thread. Output is never truncated to fit a queue. A revoked
/// lease wakes a lane-blocked pump; the stream still ends with its
/// terminal chunk.
fn pty_output(
    shared: &Arc<SessionState>,
    stream: StreamId,
    mut master: std::fs::File,
    lease: CancelToken,
    owner: Arc<ExecLease>,
    window: StreamRegistration,
) -> thread::JoinHandle<()> {
    let worker = Arc::clone(shared);
    thread::spawn(move || {
        let mut buffer = vec![0_u8; PUMP_CHUNK];
        let _owner = owner;
        let mut sequence = 0_u64;
        'pump: loop {
            let emit = match window.take(&lease) {
                WindowUse::Emit => true,
                WindowUse::Discard => false,
                WindowUse::Stopped => break,
            };
            match master.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    let mut filled = count;
                    // Coalesce immediately-available short reads into the
                    // same chunk (0055's helper did the same at its
                    // packet boundary): a child flooding in tiny writes
                    // must not turn each write into a wire chunk —
                    // per-chunk costs would outrun the consumer and the
                    // bounded slot would abandon a healthy stream.
                    // Nonblocking: a lone short read ships at once, so
                    // interactive echo latency is untouched. The byte
                    // stream is identical; only the framing coalesces.
                    while filled < buffer.len() {
                        let mut descriptors = [libc::pollfd {
                            fd: master.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        }];
                        // SAFETY: the pump owns the live master and the
                        // pollfd storage.
                        let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), 1, 0) };
                        if ready <= 0 {
                            break;
                        }
                        match master.read(&mut buffer[filled..]) {
                            Ok(0) => break,
                            Ok(more) => filled += more,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(_) => break,
                        }
                    }
                    if !emit {
                        continue;
                    }
                    match worker.push_chunk(
                        StreamChunk {
                            stream,
                            sequence,
                            last: false,
                            bytes: buffer[..filled].to_vec(),
                        },
                        Some(&lease),
                    ) {
                        PushChunk::Enqueued => sequence += 1,
                        PushChunk::Cancelled => break 'pump,
                        PushChunk::Halted => return,
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                // A PTY master reports EIO once no slave is open: the
                // truthful end of output, not a fault.
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) => {
                    worker.note(format_args!("pty output pump: {error}"));
                    break;
                }
            }
            if worker.stop.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
        }
        worker.end_stream(stream, sequence);
    })
}

/// Enqueue one ordered PTY resize (WK12). Admitted on the read loop —
/// never behind a request thread — so input chunks and resizes reach
/// the terminal in wire order; the input thread's reply is the ordered
/// geometry boundary. A full control queue is flow pressure: the lease
/// is revoked rather than letting control wait behind undrained data.
pub(crate) fn enqueue_resize(
    shared: &Arc<SessionState>,
    request: RequestId,
    exec: ExecId,
    geometry: strop_worker_protocol::PtyGeometry,
) {
    let reply = |outcome| {
        shared.send(strop_worker_protocol::WorkerMessage::Result {
            id: request,
            outcome,
        })
    };
    if geometry.columns == 0 || geometry.rows == 0 {
        return reply(ResultOutcome::Refused {
            refusal: Refusal::Limit {
                message: "PTY geometry must be non-zero".into(),
            },
        });
    }
    let control = {
        let execs = shared.execs.lock();
        execs
            .get(&exec)
            .and_then(|entry| entry.pty.as_ref().map(|pty| pty.control.clone()))
    };
    let Some(control) = control else {
        return reply(unknown_exec(exec));
    };
    match control.try_send(PtyControl::Resize { geometry, request }) {
        Ok(()) => {}
        Err(std::sync::mpsc::TrySendError::Full(_)) => {
            revoke(shared, exec);
            reply(ResultOutcome::Refused {
                refusal: Refusal::Busy {
                    message: "PTY input queue is full; lease revoked".into(),
                },
            });
        }
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
            reply(ResultOutcome::Refused {
                refusal: Refusal::Closed,
            });
        }
    }
}
