//! Client loopback against the real in-process serve loop (WK04): every
//! test crosses the actual codec over pipes — same handlers, same frames,
//! no child process. Process spawn/kill semantics live in the strop
//! binary's integration tests.

use std::io::Read as _;

use strop_worker_client::{ClientError, Transport, Worker};
use strop_workspace::operation::{CopyVersion, OperationIntent, OperationKind, StepOutcome};
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

    let mut payload = worker
        .read(&token, ResourceLocation::local(file), 0, None)
        .unwrap();
    let mut bytes = Vec::new();
    payload.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"hello worker\n");
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
    let mut handle = worker
        .exec(
            &token,
            strop_worker_protocol::ExecSpec {
                program: b"/bin/echo".to_vec(),
                argv: vec![b"worker-says-hi".to_vec()],
                cwd: b"/".to_vec(),
                env: Vec::new(),
                service: false,
                pty: None,
            },
        )
        .unwrap();
    let mut output = String::new();
    handle.stdout().read_to_string(&mut output).unwrap();
    assert_eq!(output, "worker-says-hi\n");
    assert_eq!(handle.wait_exit(), ExitStatus::Exit(0));
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
