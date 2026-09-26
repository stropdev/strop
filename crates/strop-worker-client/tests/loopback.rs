//! Client loopback against the real in-process serve loop (WK04): every
//! test crosses the actual codec over pipes — same handlers, same frames,
//! no child process. Process spawn/kill semantics live in the strop
//! binary's integration tests.

use std::io::Read as _;

use strop_worker_client::{ClientError, Transport, Worker};
use strop_workspace::operation::{
    CopyVersion, OperationIntent, OperationKind, StepOutcome, StepReceipt,
};
use strop_workspace::ResourceLocation;

fn worker() -> Worker {
    Worker::connect_with(|| {
        let (client_read, worker_write) = std::io::pipe()?;
        let (worker_read, client_write) = std::io::pipe()?;
        std::thread::spawn(move || {
            if let Err(error) = strop_worker::serve::run(worker_read, worker_write) {
                eprintln!("test worker serve failed: {error}");
            }
        });
        Ok(Transport {
            reader: Box::new(client_read),
            writer: Box::new(client_write),
            child: None,
            stderr: None,
        })
    })
}

/// A live token: the standalone handle cancels on drop, so tests hold it.
fn token() -> (
    strop_core::worker::CancelToken,
    strop_core::worker::CancelHandle,
) {
    strop_core::worker::CancelToken::standalone()
}

#[test]
fn handshake_grants_a_session_and_capabilities() {
    let worker = worker();
    let capabilities = worker.capabilities().unwrap();
    assert!(capabilities.read && capabilities.write && capabilities.list);
    let session = worker.session().unwrap();
    assert_ne!(session.incarnation, 0);
    let limits = worker.limits().unwrap();
    assert!(limits.max_frame_bytes > 0 && limits.max_pending_requests > 0);
    worker.shutdown().unwrap();
    assert!(worker.session().is_none());
}

#[test]
fn observe_list_and_read_round_trip() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("note.txt");
    std::fs::write(&file, "hello worker\n").unwrap();
    let worker = worker();
    let (token, _handle) = token();

    let observations = worker
        .observe(&token, vec![ResourceLocation::local(file.clone())])
        .unwrap();
    let observation = observations[0].value.as_ref().unwrap();
    assert_eq!(observation.kind, strop_workspace::EntryKind::File);
    assert_eq!(observation.size, Some(13));

    let missing = directory.path().join("absent.txt");
    let observations = worker
        .observe(&token, vec![ResourceLocation::local(missing)])
        .unwrap();
    assert!(observations[0].value.is_none());

    let snapshot = worker
        .list(
            &token,
            ResourceLocation::local(directory.path().to_path_buf()),
        )
        .unwrap();
    assert!(snapshot
        .entries
        .iter()
        .any(|entry| entry.name.as_path() == std::path::Path::new("note.txt")));

    // A ranged read announces and delivers exactly the range — a
    // short-stream error there would mean a torn read, never a range
    // smaller than the file.
    let mut ranged = worker
        .read(&token, ResourceLocation::local(file.clone()), 6, Some(6))
        .unwrap();
    let mut range_bytes = Vec::new();
    ranged.read_to_end(&mut range_bytes).unwrap();
    assert_eq!(range_bytes, b"worker");
    // The range's final data chunk is its only terminal marker; the
    // connection must still admit a subsequent control request.
    worker.health(&token).unwrap();

    let mut payload = worker
        .read(&token, ResourceLocation::local(file), 0, None)
        .unwrap();
    let mut bytes = Vec::new();
    payload.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"hello worker\n");
    worker.shutdown().unwrap();
}

/// A finite read may exceed the inbound queue many times over without
/// being mistaken for a stalled consumer. While it waits for credits,
/// the worker's control reader must still answer Health.
#[test]
fn large_read_returns_exact_bytes_and_does_not_starve_control() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("large.bin");
    let original = vec![b'x'; 17 * 1024 * 1024];
    std::fs::write(&file, &original).unwrap();
    let worker = worker();
    let (read_token, _handle) = token();
    let mut payload = worker
        .read(&read_token, ResourceLocation::local(file.clone()), 0, None)
        .unwrap();

    let (sent, received) = std::sync::mpsc::channel();
    let peer = worker.clone();
    let checking = std::thread::spawn(move || {
        let (token, _handle) = token();
        sent.send(peer.health(&token)).unwrap();
    });
    received
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("control must make progress while the read window is full")
        .unwrap();
    checking.join().unwrap();

    let mut bytes = Vec::new();
    payload.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, original);
    drop(payload);

    // Dropping an unread payload cancels its owner: the worker never
    // waits forever for credits to a consumer that no longer exists.
    let abandoned = worker
        .read(&read_token, ResourceLocation::local(file), 0, None)
        .unwrap();
    drop(abandoned);
    let (health, _health_handle) = token();
    worker.health(&health).unwrap();
    worker.shutdown().unwrap();
}

#[test]
fn prepare_apply_and_verify_a_create() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("created.txt");
    let worker = worker();
    let (token, _handle) = token();
    let intents = vec![OperationIntent {
        kind: OperationKind::CreateFile,
        source: None,
        destination: Some(ResourceLocation::local(target.clone())),
        copy_version: CopyVersion::Stored,
        expected_content: None,
        store: None,
    }];
    let (steps, refused) = worker.prepare(&token, intents, None).unwrap();
    assert!(refused.is_empty());
    assert_eq!(steps.len(), 1);
    let receipts = worker.apply(&token, steps, None).unwrap();
    assert!(
        receipts
            .iter()
            .all(|receipt| matches!(receipt.outcome, StepOutcome::Committed { .. })),
        "create commits through the worker: {:?}",
        receipts
    );
    assert!(target.exists());
    let verified = worker.verify(&token, receipts[0].clone()).unwrap();
    let _ = verified;
    worker.shutdown().unwrap();
}

/// WK09: the Store intent (protected document save) round-trips the real
/// codec — baseline conflict, committed receipt and witness-less verify.
#[test]
fn store_save_round_trip_with_conflict_and_verify() {
    use sha2::Digest as _;
    use strop_workspace::operation::{FsFailureKind, StorePolicy, VerifiedOutcome};
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("note.txt");
    std::fs::write(&target, "before\n").unwrap();
    let baseline = {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(&target).unwrap();
        strop_workspace::FileTime {
            seconds: metadata.mtime(),
            nanos: metadata.mtime_nsec() as u32,
        }
    };
    let worker = worker();
    let (token, _handle) = token();
    let store = |force: bool, content: &[u8]| OperationIntent {
        kind: OperationKind::Store,
        source: None,
        destination: Some(ResourceLocation::local(target.clone())),
        copy_version: CopyVersion::Stored,
        expected_content: Some(sha2::Sha256::digest(content).into()),
        store: Some(StorePolicy {
            baseline: Some(baseline),
            baseline_object: None,
            baseline_attributes: None,
            force,
            expect_absent: false,
            displayed: None,
        }),
    };
    // A stale baseline refuses typed, without touching the file.
    let moved = strop_workspace::FileTime {
        seconds: 42,
        nanos: 0,
    };
    let stale = OperationIntent {
        store: Some(StorePolicy {
            baseline: Some(moved),
            ..store(false, b"x").store.unwrap()
        }),
        ..store(false, b"x")
    };
    let (steps, refused) = worker.prepare(&token, vec![stale], None).unwrap();
    assert!(steps.is_empty());
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].failure.kind, FsFailureKind::Conflict);
    assert_eq!(std::fs::read(&target).unwrap(), b"before\n");
    // The true baseline commits through the upload stream.
    let (steps, refused) = worker
        .prepare(&token, vec![store(false, b"after\n")], None)
        .unwrap();
    assert!(refused.is_empty() && steps.len() == 1, "{refused:?}");
    let receipts = worker.apply(&token, steps, Some(b"after\n")).unwrap();
    assert!(
        matches!(receipts[0].outcome, StepOutcome::Committed { .. }),
        "store commits through the worker: {:?}",
        receipts
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"after\n");
    // A receipt lost before its witness verifies by the intended digest.
    let lost = StepReceipt {
        step: 0,
        operation: receipts[0].operation.clone(),
        outcome: StepOutcome::Unconfirmed {
            detail: "simulated lost acknowledgment".into(),
            observed_destination: None,
            recovery: None,
            publication: None,
        },
    };
    let verified = worker.verify(&token, lost.clone()).unwrap();
    assert!(
        matches!(verified, VerifiedOutcome::Committed(_)),
        "{verified:?}"
    );
    #[cfg(target_os = "linux")]
    {
        let namespace = worker.namespace().unwrap();
        worker.shutdown().unwrap();
        assert!(
            matches!(
                worker.verify(&token, lost.clone()),
                Err(ClientError::Domain(_))
            ),
            "a new session cannot reuse the old prepared capability"
        );
        let verified = worker
            .verify_recovered(&token, lost.clone(), namespace.clone())
            .unwrap();
        assert!(matches!(verified, VerifiedOutcome::Committed(_)));
        assert!(
            matches!(
                worker.verify_recovered(&token, receipts[0].clone(), namespace.clone()),
                Err(ClientError::Domain(_))
            ),
            "an acknowledged commit must not enter recovered verification"
        );
        let mut foreign = namespace;
        foreign.identity.push_str("-foreign-mount");
        assert!(matches!(
            worker.verify_recovered(&token, lost, foreign),
            Err(ClientError::Domain(_))
        ));
    }
    worker.shutdown().unwrap();
}

#[test]
fn cancellation_is_typed_before_send() {
    let worker = worker();
    let (token, handle) = strop_core::worker::CancelToken::standalone();
    handle.cancel(strop_core::worker::CancelReason::Dismissed);
    let error = worker.health(&token).unwrap_err();
    assert!(matches!(error, ClientError::Cancelled));
    worker.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn notify_subscription_delivers_hints_and_retires() {
    let directory = tempfile::tempdir().unwrap();
    let worker = worker();
    let (token, _handle) = token();
    let (tx, rx) = std::sync::mpsc::channel();
    worker.set_event_sink(tx);
    let (subscription, coverage) = worker
        .subscribe(
            &token,
            ResourceLocation::local(directory.path().to_path_buf()),
            false,
        )
        .unwrap();
    assert_eq!(coverage, strop_worker_protocol::NotifyCoverage::Native);
    // The reconcile boundary arrives first after every install.
    let first = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(matches!(
        first,
        strop_worker_protocol::Event::ReconcileBoundary { .. }
    ));
    std::fs::write(directory.path().join("watched.txt"), b"x").unwrap();
    let mut saw_hint = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !saw_hint && std::time::Instant::now() < deadline {
        if let Ok(strop_worker_protocol::Event::Notify { hints, .. }) =
            rx.recv_timeout(std::time::Duration::from_secs(1))
        {
            saw_hint = hints.iter().any(|hint| hint.path == b"watched.txt");
        }
    }
    assert!(saw_hint, "the write produced a notify hint");
    worker.unsubscribe(&token, subscription).unwrap();
    let error = worker.unsubscribe(&token, subscription).unwrap_err();
    assert!(matches!(error, ClientError::Refused(_)));
    worker.shutdown().unwrap();
}

#[cfg(unix)]
#[test]
fn finite_exec_streams_output_and_exits() {
    use strop_worker_protocol::ExitStatus;
    let worker = worker();
    let (token, _handle) = token();
    // A short-lived child can exit before either output pump starts.
    // Every admitted exec must still deliver its bytes and terminal
    // chunk before the next request reuses the caller's token.
    for run in 0..64 {
        let message = format!("worker-says-hi-{run}");
        let mut handle = worker
            .exec(
                &token,
                strop_worker_protocol::ExecSpec {
                    program: b"/bin/echo".to_vec(),
                    argv: vec![message.clone().into_bytes()],
                    cwd: b"/".to_vec(),
                    env: Vec::new(),
                    service: false,
                    pty: None,
                },
            )
            .unwrap();
        let mut output = String::new();
        handle.stdout().read_to_string(&mut output).unwrap();
        assert_eq!(output, format!("{message}\n"));
        assert_eq!(handle.wait_exit(), ExitStatus::Exit(0));
    }
    worker.shutdown().unwrap();
}

#[cfg(unix)]
#[test]
fn child_printing_framed_worker_control_remains_stdout_data() {
    use strop_worker_protocol::codec;
    use strop_worker_protocol::{ExitStatus, RequestId, ResultOutcome, WorkerMessage};

    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("forged-wire.bin");
    let mut forged = Vec::new();
    codec::write_envelope(
        &mut forged,
        &WorkerMessage::Result {
            id: RequestId(7),
            outcome: ResultOutcome::Healthy,
        },
    )
    .unwrap();
    std::fs::write(&file, &forged).unwrap();
    let worker = worker();
    let (exec_token, _owner) = token();
    let mut handle = worker
        .exec(
            &exec_token,
            strop_worker_protocol::ExecSpec {
                program: b"/bin/cat".to_vec(),
                argv: vec![file.as_os_str().as_encoded_bytes().to_vec()],
                cwd: b"/".to_vec(),
                env: Vec::new(),
                service: false,
                pty: None,
            },
        )
        .unwrap();
    let mut stdout = Vec::new();
    handle.stdout().read_to_end(&mut stdout).unwrap();
    assert_eq!(
        stdout, forged,
        "child control-shaped bytes stay inert payload"
    );
    assert_eq!(handle.wait_exit(), ExitStatus::Exit(0));
    let (health_token, _health_owner) = token();
    worker.health(&health_token).unwrap();
    worker.shutdown().unwrap();
}

#[cfg(unix)]
#[test]
fn cancelled_quiet_exec_unblocks_its_reader_and_revokes_exact_lease() {
    use strop_worker_protocol::ExitStatus;
    let worker = worker();
    let (request_token, request_owner) = token();
    let handle = worker
        .exec(
            &request_token,
            strop_worker_protocol::ExecSpec {
                program: b"/bin/sh".to_vec(),
                argv: vec![b"-c".to_vec(), b"sleep 30".to_vec()],
                cwd: b"/".to_vec(),
                env: Vec::new(),
                service: false,
                pty: None,
            },
        )
        .unwrap();
    let (exec, stdin, mut stdout, _stderr, exit) = handle.into_parts();
    let control = stdin.control(worker.clone(), exec);
    let reading = std::thread::spawn(move || {
        let mut byte = [0; 1];
        stdout.read(&mut byte).unwrap_err().kind()
    });
    drop(request_owner);
    assert_eq!(reading.join().unwrap(), std::io::ErrorKind::Interrupted);
    let (cancel, _owner) = token();
    control.cancel(&cancel).unwrap();
    assert_eq!(exit.wait(), ExitStatus::Lost);
    worker.shutdown().unwrap();
}

#[cfg(unix)]
#[test]
fn large_exec_output_backpressures_off_control_without_losing_bytes() {
    use strop_worker_protocol::ExitStatus;
    let worker = worker();
    let (request_token, _owner) = token();
    let mut handle = worker
        .exec(
            &request_token,
            strop_worker_protocol::ExecSpec {
                program: b"/bin/sh".to_vec(),
                argv: vec![b"-c".to_vec(), b"head -c 8388608 /dev/zero".to_vec()],
                cwd: b"/".to_vec(),
                env: Vec::new(),
                service: false,
                pty: None,
            },
        )
        .unwrap();
    let (health, _health_owner) = token();
    worker.health(&health).unwrap();
    let mut total = 0usize;
    let mut chunk = [0u8; 1024];
    loop {
        let count = handle.stdout().read(&mut chunk).unwrap();
        if count == 0 {
            break;
        }
        assert!(chunk[..count].iter().all(|byte| *byte == 0));
        total += count;
        std::thread::yield_now();
    }
    assert_eq!(total, 8 * 1024 * 1024);
    assert_eq!(handle.wait_exit(), ExitStatus::Exit(0));
    worker.shutdown().unwrap();
}

#[cfg(unix)]
#[test]
fn dropping_exec_output_drains_child_to_completion() {
    use std::os::unix::ffi::OsStrExt;
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("settled");
    let worker = worker();
    let (request_token, _owner) = token();
    let handle = worker
        .exec(
            &request_token,
            strop_worker_protocol::ExecSpec {
                program: b"/bin/sh".to_vec(),
                argv: vec![
                    b"-c".to_vec(),
                    b"head -c 8388608 /dev/zero && printf done > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"".to_vec(),
                    b"sh".to_vec(),
                    marker.as_os_str().as_bytes().to_vec(),
                ],
                cwd: directory.path().as_os_str().as_bytes().to_vec(),
                env: Vec::new(),
                service: false,
                pty: None,
            },
        )
        .unwrap();
    drop(handle);
    let (health, _health_owner) = token();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.exists() && std::time::Instant::now() < deadline {
        worker.health(&health).unwrap();
    }
    assert_eq!(std::fs::read(&marker).unwrap(), b"done");
    worker.shutdown().unwrap();
}

#[test]
fn shutdown_then_next_request_spawns_a_fresh_incarnation() {
    let worker = worker();
    let (token, _handle) = token();
    worker.health(&token).unwrap();
    let first = worker.session().unwrap();
    worker.shutdown().unwrap();
    worker.health(&token).unwrap();
    let second = worker.session().unwrap();
    assert_ne!(first, second, "a retired lease never resurrects");
    worker.shutdown().unwrap();
}

/// A deployed worker's handshake binds to the *endpoint's* target, not
/// the client's platform (0058 WK08): a lease admitted for a foreign
/// triple accepts exactly that triple and refuses any other — the same
/// check deployment's activation relies on, at the lease boundary.
#[test]
fn deployed_worker_binds_the_endpoint_target() {
    use strop_worker_protocol::codec::{self, Incoming};
    use strop_worker_protocol::frame::{self, FrameDecoder};
    use strop_worker_protocol::{
        Capabilities, ClientMessage, EndpointInfo, LeaseId, Limits, NamespaceIdentity,
        NotifyCoverage, Session, WorkerMessage, PROTOCOL_VERSION,
    };

    /// A foreign worker: speaks the real codec, reports a target triple
    /// this client was never built for.
    fn foreign_serve(mut reader: impl std::io::Read, mut writer: impl std::io::Write) {
        let mut decoder = FrameDecoder::default();
        let body = frame::read_frame(&mut reader, &mut decoder)
            .unwrap()
            .unwrap();
        let Incoming::Envelope(ClientMessage::Hello { .. }) = codec::decode_body(&body).unwrap()
        else {
            panic!("a foreign worker still expects hello first");
        };
        codec::write_envelope(
            &mut writer,
            &WorkerMessage::Welcome {
                protocol: PROTOCOL_VERSION,
                worker: EndpointInfo {
                    name: "strop".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    build: None,
                    target: "riscv64-unknown-linux-musl".into(),
                },
                session: Session {
                    incarnation: 7,
                    lease: LeaseId(11),
                },
                namespace: NamespaceIdentity {
                    identity: "test".into(),
                    principal: None,
                },
                limits: Limits {
                    max_frame_bytes: 1 << 20,
                    max_chunk_bytes: 1 << 16,
                    max_pending_requests: 8,
                    max_batch_steps: 8,
                    max_listing_entries: 1024,
                    max_subscriptions: 4,
                    max_streams: 8,
                    max_exec_processes: 4,
                    max_concurrent_reads: 4,
                    control_reserve: 2,
                    max_queued_data_chunks: 8,
                },
                capabilities: Capabilities {
                    observe: true,
                    list: true,
                    read: true,
                    write: true,
                    trash: false,
                    notify: NotifyCoverage::OnDemand,
                    exec_finite: true,
                    exec_service: true,
                    pty: false,
                },
            },
        )
        .unwrap();
        // Stay alive: answer requests with Healthy, so the admitted
        // lease can make a real round trip after the handshake.
        while let Ok(Some(body)) = frame::read_frame(&mut reader, &mut decoder) {
            let Incoming::Envelope(ClientMessage::Request { id, .. }) =
                codec::decode_body(&body).unwrap()
            else {
                continue;
            };
            codec::write_envelope(
                &mut writer,
                &WorkerMessage::Result {
                    id,
                    outcome: strop_worker_protocol::ResultOutcome::Healthy,
                },
            )
            .unwrap();
        }
    }
    fn foreign_transport() -> std::io::Result<Transport> {
        let (client_read, worker_write) = std::io::pipe()?;
        let (worker_read, client_write) = std::io::pipe()?;
        std::thread::spawn(move || foreign_serve(worker_read, worker_write));
        Ok(Transport {
            reader: Box::new(client_read),
            writer: Box::new(client_write),
            child: None,
            stderr: None,
        })
    }

    // Admitted for the endpoint's own triple: the lease answers.
    let deployed = Worker::connect_deployed("riscv64-unknown-linux-musl", foreign_transport);
    let session = deployed.session();
    assert!(session.is_none(), "lazy until the first request");
    let (token, _handle) = token();
    // Health needs no filesystem; the handshake alone exercises the
    // binding, and the foreign worker answers health checks.
    let error = deployed.health(&token);
    assert!(error.is_ok(), "endpoint-target lease admitted: {error:?}");

    // The default (this build's own target) refuses the same worker: a
    // typed mismatch, never a downgrade.
    let local = Worker::connect_with(foreign_transport);
    let error = local.health(&token).unwrap_err();
    assert!(
        matches!(
            error,
            ClientError::Mismatch {
                field: "target",
                ..
            }
        ),
        "foreign target refused: {error:?}"
    );
}

#[cfg(unix)]
#[test]
fn exec_env_overlay_crosses_the_codec() {
    use strop_worker_protocol::{request::EnvVar, ExitStatus};
    let worker = worker();
    let (token, _handle) = token();
    let mut handle = worker
        .exec(
            &token,
            strop_worker_protocol::ExecSpec {
                program: b"/bin/sh".to_vec(),
                argv: vec![
                    b"-c".to_vec(),
                    b"printf %s \"$STROP_LOOP\"; printf :; printf %s \"$HOME\" | wc -c".to_vec(),
                ],
                cwd: b"/".to_vec(),
                env: vec![EnvVar {
                    name: b"STROP_LOOP".to_vec(),
                    value: vec![0x6f, 0x76, 0x65, 0x72, 0x6c, 0x61, 0x79],
                }],
                service: false,
                pty: None,
            },
        )
        .unwrap();
    let mut output = String::new();
    handle.stdout().read_to_string(&mut output).unwrap();
    // The overlay applies; the inherited environment (HOME) survives.
    assert!(output.starts_with("overlay:"), "output: {output}");
    assert!(output[8..].trim().parse::<usize>().unwrap() > 0);
    assert_eq!(handle.wait_exit(), ExitStatus::Exit(0));
    worker.shutdown().unwrap();
}

#[cfg(unix)]
#[test]
fn exec_env_overlay_malformed_is_a_typed_failure() {
    use strop_worker_protocol::request::EnvVar;
    let worker = worker();
    let (token, _handle) = token();
    let error = worker
        .exec(
            &token,
            strop_worker_protocol::ExecSpec {
                program: b"/bin/true".to_vec(),
                argv: vec![],
                cwd: b"/".to_vec(),
                env: vec![EnvVar {
                    name: b"BAD=NAME".to_vec(),
                    value: b"v".to_vec(),
                }],
                service: false,
                pty: None,
            },
        )
        .expect_err("malformed overlay must be refused");
    assert!(
        matches!(error, ClientError::Domain(_)),
        "typed domain failure, not silence: {error}"
    );
    worker.shutdown().unwrap();
}

#[cfg(unix)]
#[path = "loopback/pty.rs"]
mod pty;
