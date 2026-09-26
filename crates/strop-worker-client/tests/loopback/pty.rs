//! PTY loopback journeys over the real worker codec and supervisor.

use super::{token, worker};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;
use strop_worker_client::{ClientError, ExecEvent, StreamEvent};
use strop_worker_protocol::{ExitStatus, PtyGeometry};

fn geometry() -> PtyGeometry {
    PtyGeometry {
        columns: 37,
        rows: 9,
    }
}

fn sh(script: &str) -> strop_worker_protocol::ExecSpec {
    strop_worker_protocol::ExecSpec {
        program: b"/bin/sh".to_vec(),
        argv: vec![b"-c".to_vec(), script.as_bytes().to_vec()],
        cwd: b"/tmp".to_vec(),
        env: Vec::new(),
        service: true,
        pty: Some(geometry()),
    }
}

/// Drain output events until `text` accumulates `needle`, returning
/// everything received; the deadline makes a stall a failure.
fn read_until(
    session: &strop_worker_client::PtySession,
    needle: &str,
    text: &mut String,
) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        match session.recv_output_timeout(Duration::from_millis(100)) {
            Ok(StreamEvent::Chunk(chunk)) => {
                text.push_str(&String::from_utf8_lossy(&chunk.bytes));
                if text.contains(needle) {
                    return text.clone();
                }
            }
            Ok(StreamEvent::Resized) => {}
            Ok(StreamEvent::Failed(reason)) => panic!("pty stream failed: {reason}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    panic!("pty output never contained {needle:?}: {text:?}");
}

/// Wait for the attested exit status (skipping input acks).
fn wait_exit(session: &strop_worker_client::PtySession) -> ExitStatus {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        match session.events().recv_timeout(Duration::from_millis(100)) {
            Ok(ExecEvent::Exit(status)) => return status,
            Ok(ExecEvent::Input { .. }) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return ExitStatus::Lost,
        }
    }
    panic!("pty exit never arrived");
}

/// Spawn with geometry, feed input, read bounded output, truthful
/// exit classification — the whole family over real pipes.
#[test]
fn pty_spawn_feed_output_and_exit_classification() {
    let worker = worker();
    let (token, _handle) = token();
    let session = worker
            .exec_pty(
                &token,
                sh("[ -t 0 ] && [ -t 1 ] || exit 90; stty size; read value; printf 'RESULT:%s\\n' \"$value\"; exit 7"),
            )
            .unwrap();
    let mut text = String::new();
    assert!(read_until(&session, "9 37", &mut text).contains("9 37"));
    let mut session = session;
    session.feed(b"hello\n").unwrap();
    // The delivered-input acknowledgment arrives in order.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut acked = false;
    while std::time::Instant::now() < deadline && !acked {
        if let Ok(ExecEvent::Input { sequence: 0 }) =
            session.events().recv_timeout(Duration::from_millis(100))
        {
            acked = true;
        }
    }
    assert!(acked, "input chunk was never acknowledged");
    assert!(read_until(&session, "RESULT:hello", &mut text).contains("RESULT:hello"));
    assert_eq!(wait_exit(&session), ExitStatus::Exit(7));
    worker.shutdown().unwrap();
}

/// The resize reply is applied geometry, and the Resized marker
/// arrives on the output stream in wire order: bytes before it are
/// the old geometry's, bytes after it the new one's.
#[test]
fn pty_resize_boundary_arrives_in_wire_order() {
    let worker = worker();
    let (token, _handle) = token();
    let session = worker
        .exec_pty(&token, sh("stty size; read value; stty size; exit 0"))
        .unwrap();
    let mut text = String::new();
    assert!(read_until(&session, "9 37", &mut text).contains("9 37"));
    session
        .resize(PtyGeometry {
            columns: 100,
            rows: 30,
        })
        .unwrap();
    // The marker must be the next non-chunk event after the resize.
    let mut session = session;
    session.feed(b"go\n").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut marked = false;
    let mut after = String::new();
    while std::time::Instant::now() < deadline {
        match session.recv_output_timeout(Duration::from_millis(100)) {
            Ok(StreamEvent::Resized) => marked = true,
            Ok(StreamEvent::Chunk(chunk)) => {
                if marked {
                    after.push_str(&String::from_utf8_lossy(&chunk.bytes));
                    if after.contains("30 100") {
                        break;
                    }
                }
            }
            Ok(StreamEvent::Failed(reason)) => panic!("pty stream failed: {reason}"),
            _ => {}
        }
    }
    assert!(marked, "the resize boundary never arrived");
    assert!(
        after.contains("30 100"),
        "post-boundary output is not the new geometry: {after:?}"
    );
    assert_eq!(wait_exit(&session), ExitStatus::Exit(0));
    worker.shutdown().unwrap();
}

/// Bounded output, never frame-lossy: while the consumer keeps
/// draining, a flooding child stalls against the bounded lane and
/// every byte still arrives, ordered, `last`-terminated, with the
/// exit classified.
#[test]
fn pty_output_backpressure_is_bounded_and_lossless() {
    let worker = worker();
    let (token, _handle) = token();
    let session = worker.exec_pty(&token, sh("seq 1 100000; exit 0")).unwrap();
    let mut collected = Vec::new();
    let mut ended = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline && !ended {
        match session.recv_output_timeout(Duration::from_millis(200)) {
            Ok(StreamEvent::Chunk(chunk)) => {
                collected.extend_from_slice(&chunk.bytes);
                ended = chunk.last;
            }
            Ok(StreamEvent::Resized) => {}
            Ok(StreamEvent::Failed(reason)) => panic!("pty stream failed: {reason}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let text = String::from_utf8_lossy(&collected);
    assert!(ended, "the output stream never ended: {}", text.len());
    // The stream is the child's exact sequence: verify it complete
    // and ordered (loss would desynchronize it).
    let mut previous = 0_u64;
    let mut count = 0_u64;
    for line in text.lines() {
        let value: u64 = match line.trim().parse() {
            Ok(value) => value,
            Err(_) => continue,
        };
        assert_eq!(
            value,
            previous + 1,
            "output was reordered or lost at {value} after {previous}"
        );
        previous = value;
        count += 1;
    }
    assert_eq!(count, 100000, "output was lossy: {count} lines");
    assert_eq!(wait_exit(&session), ExitStatus::Exit(0));
    worker.shutdown().unwrap();
}

/// A paused terminal consumer cannot exhaust a finite receive buffer
/// and lose bytes. Worker control remains responsive while its PTY
/// output producer waits for consumed-chunk credits.
#[test]
fn pty_stalled_consumer_resumes_without_losing_vt_bytes() {
    let worker = worker();
    let (pty_token, _handle) = token();
    let (sent, arrived) = std::sync::mpsc::sync_channel(64);
    worker.set_stream_notifier(move |stream| {
        let _ = sent.try_send(stream);
    });
    let session = worker
        .exec_pty(&pty_token, sh("seq 1 300000; exit 0"))
        .unwrap();
    let mut credited = 0;
    while credited < strop_worker_protocol::STREAM_WINDOW_CHUNKS {
        let stream = arrived
            .recv_timeout(Duration::from_secs(10))
            .expect("PTY must exhaust its initial window without a consumer");
        if stream == session.output_stream() {
            credited += 1;
        }
    }
    let (health_token, _health_owner) = token();
    worker.health(&health_token).unwrap();
    let mut collected = Vec::new();
    let mut ended = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline && !ended {
        match session.recv_output_timeout(Duration::from_millis(200)) {
            Ok(StreamEvent::Chunk(chunk)) => {
                collected.extend_from_slice(&chunk.bytes);
                ended = chunk.last;
            }
            Ok(StreamEvent::Resized) => {}
            Ok(StreamEvent::Failed(reason)) => panic!("PTY output failed: {reason}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    assert!(ended, "stalled PTY output failed to resume");
    let text = String::from_utf8(collected).unwrap();
    let mut count = 0_usize;
    for line in text.lines() {
        count += 1;
        assert_eq!(line.trim().parse::<usize>().unwrap(), count);
    }
    assert_eq!(count, 300000);
    assert_eq!(wait_exit(&session), ExitStatus::Exit(0));
    worker.shutdown().unwrap();
}

/// A revoked PTY carries no attested exit: terminate revokes the
/// supervised group and the classification is honestly Lost, with
/// the output stream still ending.
#[test]
fn pty_terminate_revokes_without_a_guessed_exit() {
    let worker = worker();
    let (token, _handle) = token();
    let session = worker
        .exec_pty(&token, sh("printf 'UP\\n'; read value; exit 0"))
        .unwrap();
    let mut text = String::new();
    assert!(read_until(&session, "UP", &mut text).contains("UP"));
    session.terminate().unwrap();
    assert_eq!(wait_exit(&session), ExitStatus::Lost);
    worker.shutdown().unwrap();
}

/// A dead session's input, resize and revoke cannot target the new
/// worker merely because its first exec reuses the same numeric id.
#[test]
fn pty_controls_remain_bound_to_the_admitting_incarnation() {
    let worker = worker();
    let (old_token, _old_handle) = token();
    let mut old = worker.exec_pty(&old_token, sh("cat")).unwrap();
    let reused_id = old.id();
    worker.shutdown().unwrap();

    let (new_token, _new_handle) = token();
    let mut fresh = worker.exec_pty(&new_token, sh("cat")).unwrap();
    assert_eq!(fresh.id(), reused_id, "the test must exercise id reuse");
    assert!(matches!(
        old.feed(b"WRONG\n"),
        Err(ClientError::WorkerLost(_))
    ));
    assert!(matches!(
        old.resize(PtyGeometry {
            columns: 80,
            rows: 24
        }),
        Err(ClientError::WorkerLost(_))
    ));
    assert!(matches!(old.terminate(), Err(ClientError::WorkerLost(_))));
    fresh.feed(b"RIGHT\n").unwrap();
    let output = read_until(&fresh, "RIGHT", &mut String::new());
    assert!(
        !output.contains("WRONG"),
        "stale input reached the new exec"
    );
    fresh.terminate().unwrap();
    worker.shutdown().unwrap();
}
