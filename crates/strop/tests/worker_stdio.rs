//! `strop --worker-stdio` against the real release binary (WK04): the
//! client spawns the matching executable, readiness is the handshake,
//! and a `kill -9` mid-session is a typed failure with no corruption and
//! no fallback — the next request spawns a fresh incarnation, and
//! authority prepared by the dead worker fails closed there.

use std::io::Read as _;
use std::path::Path;

use strop_worker_client::{ClientError, Worker};
use strop_workspace::operation::{CopyVersion, OperationIntent, OperationKind};
use strop_workspace::ResourceLocation;

fn worker() -> Worker {
    Worker::spawn_program(env!("CARGO_BIN_EXE_strop"))
}

fn token() -> (
    strop_core::worker::CancelToken,
    strop_core::worker::CancelHandle,
) {
    strop_core::worker::CancelToken::standalone()
}

#[test]
fn handshake_and_filesystem_round_trip_through_the_real_binary() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("note.txt");
    std::fs::write(&file, "through the worker\n").unwrap();
    let worker = worker();
    let (token, _handle) = token();

    let capabilities = worker.capabilities().unwrap();
    assert!(capabilities.read && capabilities.write && capabilities.list);

    let observations = worker
        .observe(&token, vec![ResourceLocation::local(file.clone())])
        .unwrap();
    assert_eq!(
        observations[0].value.as_ref().unwrap().kind,
        strop_workspace::EntryKind::File
    );

    let snapshot = worker
        .list(
            &token,
            ResourceLocation::local(directory.path().to_path_buf()),
        )
        .unwrap();
    assert!(snapshot
        .entries
        .iter()
        .any(|entry| entry.name.as_path() == Path::new("note.txt")));

    let mut payload = worker
        .read(&token, ResourceLocation::local(file), 0, None)
        .unwrap();
    let mut bytes = Vec::new();
    payload.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"through the worker\n");

    worker.shutdown().unwrap();
}

#[test]
fn a_non_worker_binary_fails_the_handshake_typed() {
    // /bin/true is a valid executable that is not a worker: readiness is
    // the handshake, never a successful spawn, and never a fallback.
    let worker = Worker::spawn_program("/bin/true");
    let (token, _handle) = token();
    match worker.health(&token) {
        Err(ClientError::Handshake(_)) | Err(ClientError::WorkerLost(_)) => {}
        other => panic!("expected a typed handshake failure, got {other:?}"),
    }
}

#[test]
fn kill_minus_nine_is_a_typed_failure_with_no_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("created.txt");
    let worker = worker();
    let (token, _handle) = token();

    // Prepare against incarnation A.
    let intents = vec![OperationIntent {
        kind: OperationKind::CreateFile,
        source: None,
        destination: Some(ResourceLocation::local(target.clone())),
        copy_version: CopyVersion::Stored,
        expected_content: None,
    }];
    let (steps, refused) = worker.prepare(&token, intents, None).unwrap();
    assert!(refused.is_empty());

    // SIGKILL the worker mid-session.
    let pid = worker.worker_pid().unwrap();
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());

    // The apply fails typed: incarnation A's prepared authority is dead,
    // and there is no silent in-process fallback.
    let error = worker.apply(&token, steps, None).unwrap_err();
    assert!(
        matches!(error, ClientError::WorkerLost(_)),
        "killed worker fails typed, got {error:?}"
    );
    assert!(!target.exists(), "no effect escaped the dead worker");

    // The next request spawns a fresh incarnation with fresh authority.
    let first = worker.session();
    worker.health(&token).unwrap();
    let second = worker.session().unwrap();
    assert!(
        first.map(|s| s.incarnation) != Some(second.incarnation),
        "a fresh incarnation replaces the killed worker"
    );
    let intents = vec![OperationIntent {
        kind: OperationKind::CreateFile,
        source: None,
        destination: Some(ResourceLocation::local(target.clone())),
        copy_version: CopyVersion::Stored,
        expected_content: None,
    }];
    let (steps, refused) = worker.prepare(&token, intents, None).unwrap();
    assert!(refused.is_empty());
    let receipts = worker.apply(&token, steps, None).unwrap();
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt.outcome.is_committed()),
        "the fresh worker commits: {receipts:?}"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "");
    worker.shutdown().unwrap();
}

#[test]
fn stale_prepared_authority_fails_closed_after_a_kill() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("stale.txt");
    let worker = worker();
    let (token, _handle) = token();
    let intents = || {
        vec![OperationIntent {
            kind: OperationKind::CreateFile,
            source: None,
            destination: Some(ResourceLocation::local(target.clone())),
            copy_version: CopyVersion::Stored,
            expected_content: None,
        }]
    };
    let (steps, refused) = worker.prepare(&token, intents(), None).unwrap();
    assert!(refused.is_empty());

    let pid = worker.worker_pid().unwrap();
    assert!(std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap()
        .success());
    // Let the reader thread observe the death before the next request.
    while worker.session().is_some() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // Respawned incarnation B: applying A's prepared steps must fail —
    // the kernel's incarnation binding rejects them, typed, and no file
    // is created.
    let receipts_or_error = worker.apply(&token, steps, None);
    if let Ok(receipts) = receipts_or_error {
        assert!(
            receipts
                .iter()
                .all(|receipt| !receipt.outcome.is_committed()),
            "stale steps never commit: {receipts:?}"
        );
    }
    assert!(!target.exists());
    worker.shutdown().unwrap();
}
