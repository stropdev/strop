//! Inbound frame routing for one admitted worker incarnation. This thread
//! never runs an editor input/render turn; its stream queues remain bounded.

use std::io::Read;
use std::sync::mpsc::{channel, sync_channel, TrySendError};
use std::sync::Arc;

use strop_worker_protocol::codec::{self, Incoming, StreamChunk};
use strop_worker_protocol::frame::{self, FrameDecoder};
use strop_worker_protocol::{ClientMessage, Event, ExecId, RequestId, WorkerMessage};

use super::{Attachments, Conn, ExecEvent, Pending, Reply, StreamEvent, StreamSlot};

/// Per-stream inbound chunk budget (WK11): 64 chunks ≈ 4 MiB at the
/// worker's 64 KiB read chunk. A consumer slower than that is flow
/// pressure, not a buffer to grow: the stream is abandoned honestly (the
/// payload errors; a read's cancellation reaches the worker) instead of
/// the reader thread buffering unboundedly or blocking control replies
/// behind one stalled consumer.
const STREAM_INBOUND_CHUNKS: usize = 64;

/// The reader thread: decode frames, route results/events/chunks, and on
/// EOF or corruption terminate the connection so every waiter fails typed.
pub(super) fn read_loop(conn: Arc<Conn>, reader: &mut impl Read) {
    let mut decoder = FrameDecoder::default();
    loop {
        let body = match frame::read_frame(reader, &mut decoder) {
            Ok(Some(body)) => body,
            Ok(None) => {
                conn.terminate("worker closed the protocol stream".to_owned());
                return;
            }
            Err(error) => {
                conn.terminate(format!("protocol stream failed: {error}"));
                return;
            }
        };
        match codec::decode_body::<WorkerMessage>(&body) {
            Ok(Incoming::Envelope(message)) => route_envelope(&conn, message),
            Ok(Incoming::Chunk(chunk)) => route_chunk(&conn, chunk),
            Err(error) => {
                conn.terminate(format!("undecodable worker frame: {error}"));
                return;
            }
        }
        if !conn.alive() {
            return;
        }
    }
}

fn route_envelope(conn: &Arc<Conn>, message: WorkerMessage) {
    match message {
        WorkerMessage::Welcome { .. } => {
            if let Some(sender) = conn.handshake.sender.lock().take() {
                let _ = sender.send(Ok(message));
            }
        }
        WorkerMessage::Result { id, outcome } => {
            let pending = conn.pending.lock().remove(&id);
            match pending {
                Some(Pending {
                    sender,
                    streaming: false,
                    pty_resize,
                    ..
                }) => {
                    // A resize's `done` is the ordered geometry boundary
                    // (WK12): the marker lands on the exec's output
                    // stream in wire order, before the reply wakes the
                    // caller, so the parser never races it.
                    if let Some(exec) = pty_resize {
                        if matches!(outcome, strop_worker_protocol::ResultOutcome::Done) {
                            push_resized(conn, exec);
                        }
                    }
                    let _ = sender.send(Reply::Outcome {
                        outcome,
                        attachments: Attachments::default(),
                    });
                }
                Some(Pending {
                    sender,
                    streaming: true,
                    ..
                }) => {
                    let mut attachments = Attachments::default();
                    match &outcome {
                        strop_worker_protocol::ResultOutcome::ReadOpened { stream, .. } => {
                            let (tx, rx) = sync_channel(STREAM_INBOUND_CHUNKS);
                            conn.streams.lock().insert(
                                *stream,
                                StreamSlot::Live {
                                    sender: tx,
                                    request: Some(id),
                                },
                            );
                            attachments.streams.push((*stream, rx));
                        }
                        strop_worker_protocol::ResultOutcome::ExecStarted {
                            exec,
                            stdout,
                            stderr,
                            ..
                        } => {
                            for stream in [stdout, stderr] {
                                let (tx, rx) = sync_channel(STREAM_INBOUND_CHUNKS);
                                conn.streams.lock().insert(
                                    *stream,
                                    StreamSlot::Live {
                                        sender: tx,
                                        request: None,
                                    },
                                );
                                attachments.streams.push((*stream, rx));
                            }
                            conn.exec_streams.lock().insert(*exec, *stdout);
                            let (tx, rx) = channel();
                            conn.execs.lock().insert(*exec, tx);
                            attachments.exits.push((*exec, rx));
                        }
                        _ => {}
                    }
                    let _ = sender.send(Reply::Outcome {
                        outcome,
                        attachments,
                    });
                }
                None => {}
            }
        }
        WorkerMessage::Event { event } => {
            match event {
                Event::ExecExit { exec, status } => {
                    conn.exec_streams.lock().remove(&exec);
                    if let Some(sender) = conn.execs.lock().remove(&exec) {
                        let _ = sender.send(ExecEvent::Exit(status));
                        return;
                    }
                }
                Event::ExecInput { exec, sequence } => {
                    // Delivered-input accounting: routed to the exec's
                    // owner; without one it is advisory and dropped.
                    if let Some(sender) = conn.execs.lock().get(&exec) {
                        let _ = sender.send(ExecEvent::Input { sequence });
                        return;
                    }
                }
                _ => {}
            }
            if let Some(sender) = conn.events.lock().as_ref() {
                let _ = sender.send(event);
            }
        }
        WorkerMessage::Error { id, error } => {
            // Before Welcome even an erroneously tagged request error
            // is a handshake refusal, never a pending reply to discard.
            if let Some(sender) = conn.handshake.sender.lock().take() {
                let _ = sender.send(Ok(WorkerMessage::Error { id, error }));
            } else {
                match id {
                    Some(id) => {
                        if let Some(pending) = conn.pending.lock().remove(&id) {
                            let _ = pending.sender.send(Reply::Protocol(error));
                        }
                    }
                    None => conn.terminate(format!("worker reported a session failure: {error}")),
                }
            }
        }
        WorkerMessage::Bye { reason } => {
            if let Some(sender) = conn.handshake.sender.lock().take() {
                let _ = sender.send(Ok(WorkerMessage::Bye { reason }));
            } else {
                if let Some(waiter) = conn.bye.lock().take() {
                    let _ = waiter.send(reason);
                }
                conn.terminate(format!("worker exited ({reason:?})"));
            }
        }
    }
}

fn route_chunk(conn: &Arc<Conn>, chunk: StreamChunk) {
    /// How the chunk resolved under the inbound budget.
    enum Verdict {
        Routed,
        /// The consumer exceeded the inbound budget or vanished; the
        /// owning request (when known) is cancelled so the worker stops
        /// producing instead of streaming into a tombstone.
        Abandoned(Option<RequestId>),
        Corrupt(String),
    }
    let stream = chunk.stream;
    let verdict = {
        let mut streams = conn.streams.lock();
        let last = chunk.last;
        match streams.get_mut(&stream) {
            Some(StreamSlot::Live { sender, request }) => {
                let request = *request;
                match sender.try_send(StreamEvent::Chunk(chunk)) {
                    Ok(()) => {
                        if last {
                            streams.remove(&stream);
                        }
                        Verdict::Routed
                    }
                    // Full: the consumer is slower than the inbound
                    // budget. Disconnected: the consumer dropped the
                    // payload. Either way the stream is abandoned
                    // honestly — the slot tombstones until the terminal
                    // chunk so routing stays exact, and dropping the
                    // sender fails the payload's next read (never silent
                    // truncation).
                    Err(TrySendError::Full(_)) => {
                        streams.insert(stream, StreamSlot::Abandoned);
                        Verdict::Abandoned(request)
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        streams.insert(stream, StreamSlot::Abandoned);
                        Verdict::Routed
                    }
                }
            }
            Some(StreamSlot::Abandoned) => {
                if last {
                    streams.remove(&stream);
                }
                Verdict::Routed
            }
            None => Verdict::Corrupt(format!("chunk on unknown stream {}", stream.0)),
        }
    };
    match verdict {
        Verdict::Routed => {
            if let Some(notifier) = conn.stream_notifier.lock().as_ref() {
                notifier(stream);
            }
        }
        Verdict::Abandoned(Some(request)) => conn.write_quiet(&ClientMessage::Cancel {
            session: conn.session(),
            id: request,
        }),
        Verdict::Abandoned(None) => {}
        // A chunk for an unknown stream: the worker violated the
        // session. This is corruption, not data.
        Verdict::Corrupt(message) => conn.terminate(message),
    }
}

/// Push the ordered resize boundary onto an exec's output stream
/// (WK12). The marker obeys the same inbound budget as chunks: a
/// consumer 64 chunks behind has its stream tombstoned honestly (its
/// receiver closes) rather than the reader buffering for it.
fn push_resized(conn: &Arc<Conn>, exec: ExecId) {
    let stream = conn.exec_streams.lock().get(&exec).copied();
    let Some(stream) = stream else { return };
    let mut streams = conn.streams.lock();
    let Some(slot) = streams.get_mut(&stream) else {
        return;
    };
    let StreamSlot::Live { sender, .. } = slot else {
        return;
    };
    if sender.try_send(StreamEvent::Resized).is_err() {
        streams.insert(stream, StreamSlot::Abandoned);
    }
}
