//! `strop --worker-stdio` against the real release binary (WK04): the
//! client spawns the matching executable, readiness is the handshake,
//! and a `kill -9` mid-session is a typed failure with no corruption and
//! no fallback — the next request spawns a fresh incarnation, and
//! authority prepared by the dead worker fails closed there.

use std::io::Read as _;
use std::path::Path;

use strop_worker_client::{ClientError, StderrCapture, Transport, Worker};
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

#[cfg(target_os = "linux")]
const CACHE_TEST_CONTEXT: &str = "local-test-cache";

#[cfg(target_os = "linux")]
fn cached_worker_binary() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    use sha2::{Digest, Sha256};
    use strop_core::worker::cache_record::{
        CacheReceipt, CACHE_DIR_NAME, LEASES_DIR, OBJECTS_DIR, RECEIPTS_DIR, RECEIPT_SCHEMA,
    };

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join(CACHE_DIR_NAME);
    let objects = root.join(OBJECTS_DIR);
    let leases = root.join(LEASES_DIR);
    let receipts = root.join(RECEIPTS_DIR);
    for path in [&root, &objects, &leases, &receipts] {
        std::fs::create_dir(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let binary = env!("CARGO_BIN_EXE_strop");
    let mut source = std::fs::File::open(binary).unwrap();
    let mut hash = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let count = source.read(&mut chunk).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&chunk[..count]);
    }
    let sha = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let object = objects.join(&sha);
    std::fs::copy(binary, &object).unwrap();
    std::fs::set_permissions(&object, std::fs::Permissions::from_mode(0o500)).unwrap();
    // SAFETY: std has no effective-uid getter; geteuid takes no pointers.
    let uid = unsafe { libc::geteuid() };
    let receipt = CacheReceipt {
        schema: RECEIPT_SCHEMA,
        context: CACHE_TEST_CONTEXT.into(),
        principal: uid.to_string(),
        version: env!("CARGO_PKG_VERSION").into(),
        target: strop_worker_protocol::TARGET_TRIPLE.into(),
        object_sha256: sha.clone(),
        object_bytes: std::fs::metadata(&object).unwrap().len(),
        tarball_sha256: "c".repeat(64),
    };
    let receipt_path = receipts.join(format!("{sha}.json"));
    std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    std::fs::set_permissions(&receipt_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    (directory, object, root)
}

#[cfg(target_os = "linux")]
fn spawn_cached_worker(object: &Path) -> std::io::Result<Transport> {
    use std::process::{Command, Stdio};

    let mut child = Command::new(object)
        .arg("--worker-stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let writer = child.stdin.take().unwrap();
    let reader = child.stdout.take().unwrap();
    let stderr = child.stderr.take().map(StderrCapture::spawn);
    Ok(Transport {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child: Some(child),
        stderr,
    })
}

#[cfg(target_os = "linux")]
fn seed_cache_object(root: &Path, bytes: &[u8], context: &str, version: &str) -> String {
    use std::os::unix::fs::PermissionsExt;

    use sha2::{Digest, Sha256};
    use strop_core::worker::cache_record::{
        CacheReceipt, OBJECTS_DIR, RECEIPTS_DIR, RECEIPT_SCHEMA,
    };

    let sha = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let object = root.join(OBJECTS_DIR).join(&sha);
    std::fs::write(&object, bytes).unwrap();
    std::fs::set_permissions(&object, std::fs::Permissions::from_mode(0o500)).unwrap();
    // SAFETY: std has no effective-uid getter; geteuid takes no pointers.
    let uid = unsafe { libc::geteuid() };
    let receipt = CacheReceipt {
        schema: RECEIPT_SCHEMA,
        context: context.into(),
        principal: uid.to_string(),
        version: version.into(),
        target: strop_worker_protocol::TARGET_TRIPLE.into(),
        object_sha256: sha.clone(),
        object_bytes: bytes.len() as u64,
        tarball_sha256: "b".repeat(64),
    };
    let receipt_path = root.join(RECEIPTS_DIR).join(format!("{sha}.json"));
    std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    std::fs::set_permissions(&receipt_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    sha
}

#[cfg(target_os = "linux")]
fn seed_cache_lease(root: &Path, lease: u64, sha: &str) {
    use std::os::unix::fs::PermissionsExt;

    use strop_core::worker::cache_record::{LeaseRecord, LEASES_DIR};

    let path = root.join(LEASES_DIR).join(format!("{lease}.json"));
    let record = LeaseRecord {
        lease,
        object_sha256: sha.into(),
    };
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// A content-addressed object removed after exec but before Hello is
/// not an installed artifact. The kernel may still be executing its
/// inode; that must not grant a session with no matching cache lease.
#[cfg(target_os = "linux")]
#[test]
fn deleted_cache_object_cannot_welcome_an_unleased_worker() {
    let (_directory, object, _root) = cached_worker_binary();

    let worker = Worker::connect_with(move || {
        let transport = spawn_cached_worker(&object)?;
        // The process has started but is waiting for Hello. Unlinking
        // now is deterministic; it still runs through the real inode.
        std::fs::remove_file(&object)?;
        Ok(transport)
    });
    let refusal = worker.capabilities();
    assert!(
        matches!(
            &refusal,
            Err(ClientError::Protocol(
                strop_worker_protocol::ProtocolError::Unexpected { .. }
            ))
        ),
        "a deleted cache object must refuse the handshake typed, got {refusal:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn cached_worker_waits_for_exclusive_lease_snapshot_before_welcome() {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    use strop_core::worker::cache_record::{LeaseRecord, CACHE_LOCK_FILE, LEASES_DIR};

    let (_directory, object, root) = cached_worker_binary();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(root.join(CACHE_LOCK_FILE))
        .unwrap();
    held.lock().unwrap();
    let (spawned_tx, spawned_rx) = channel();
    let (ready_tx, ready_rx) = channel();
    let serving_object = object.clone();
    let worker = Worker::connect_with(move || {
        let transport = spawn_cached_worker(&serving_object)?;
        spawned_tx.send(()).unwrap();
        Ok(transport)
    });
    let caller = std::thread::spawn(move || {
        let result = worker.capabilities();
        ready_tx.send(result).unwrap();
        worker
    });
    spawned_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        matches!(
            ready_rx.recv_timeout(Duration::from_millis(250)),
            Err(RecvTimeoutError::Timeout)
        ),
        "a cached worker cannot Welcome while another process owns the cache lock"
    );
    drop(held);
    let capabilities = ready_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert!(capabilities.read && capabilities.write);
    let worker = caller.join().unwrap();
    let session = worker.session().unwrap();
    let bytes = std::fs::read(
        root.join(LEASES_DIR)
            .join(format!("{}.json", session.lease.0)),
    )
    .unwrap();
    let record: LeaseRecord = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(record.lease, session.lease.0);
    assert_eq!(
        record.object_sha256,
        object.file_name().unwrap().to_str().unwrap()
    );
    worker.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn cache_gc_retires_only_unleased_selected_context_artifacts() {
    use strop_core::worker::cache_record::{OBJECTS_DIR, RECEIPTS_DIR};

    let (_directory, object, root) = cached_worker_binary();
    let unused = seed_cache_object(&root, b"old worker", CACHE_TEST_CONTEXT, "0.34.0");
    let foreign = seed_cache_object(&root, b"other context worker", "foreign-context", "0.34.0");
    std::fs::write(root.join(OBJECTS_DIR).join("README"), b"foreign entry").unwrap();
    let (token, _handle) = token();
    let worker = Worker::spawn_program(&object);

    let outcome = worker.collect_cache(&token, CACHE_TEST_CONTEXT).unwrap();
    assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
    assert_eq!(outcome.report.removed_objects, vec![unused.clone()]);
    assert_eq!(outcome.report.removed_receipts, 1);
    assert_eq!(outcome.report.kept_objects, 1);
    assert!(!root.join(OBJECTS_DIR).join(&unused).exists());
    assert!(!root
        .join(RECEIPTS_DIR)
        .join(format!("{unused}.json"))
        .exists());
    assert!(object.exists());
    assert!(root.join(OBJECTS_DIR).join(&foreign).exists());
    assert!(root.join(OBJECTS_DIR).join("README").exists());
    worker.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn cache_gc_rejects_a_foreign_context_even_with_its_receipt() {
    use strop_core::worker::cache_record::OBJECTS_DIR;
    use strop_workspace::operation::FsFailureKind;

    let (_directory, object, root) = cached_worker_binary();
    let foreign = seed_cache_object(&root, b"foreign context", "foreign-context", "0.34.0");
    let (token, _handle) = token();
    let worker = Worker::spawn_program(&object);

    let outcome = worker.collect_cache(&token, "foreign-context").unwrap();
    assert_eq!(outcome.failure.unwrap().kind, FsFailureKind::Permission);
    assert!(outcome.report.removed_objects.is_empty());
    assert!(root.join(OBJECTS_DIR).join(foreign).exists());
    assert!(object.exists());
    worker.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn cache_gc_keeps_foreign_live_lease_until_its_record_is_released() {
    use strop_core::worker::cache_record::{LEASES_DIR, OBJECTS_DIR};

    let (_directory, object, root) = cached_worker_binary();
    let pinned = seed_cache_object(&root, b"another live client", CACHE_TEST_CONTEXT, "0.34.0");
    seed_cache_lease(&root, 42, &pinned);
    let (token, _handle) = token();
    let worker = Worker::spawn_program(&object);

    let first = worker.collect_cache(&token, CACHE_TEST_CONTEXT).unwrap();
    assert!(first.failure.is_none(), "{:?}", first.failure);
    assert!(first.report.removed_objects.is_empty());
    assert_eq!(first.report.kept_objects, 2);
    assert!(root.join(OBJECTS_DIR).join(&pinned).exists());
    std::fs::remove_file(root.join(LEASES_DIR).join("42.json")).unwrap();

    let second = worker.collect_cache(&token, CACHE_TEST_CONTEXT).unwrap();
    assert!(second.failure.is_none(), "{:?}", second.failure);
    assert_eq!(second.report.removed_objects, vec![pinned.clone()]);
    assert!(!root.join(OBJECTS_DIR).join(pinned).exists());
    worker.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn cache_gc_preserves_another_actual_worker_process_and_build() {
    use std::process::Command;

    use strop_core::worker::cache_record::{LEASES_DIR, OBJECTS_DIR};

    let (_directory, object, root) = cached_worker_binary();
    let second_build = tempfile::tempdir().unwrap();
    let stripped = second_build.path().join("other-worker-build");
    assert!(Command::new("strip")
        .arg(env!("CARGO_BIN_EXE_strop"))
        .arg("-o")
        .arg(&stripped)
        .status()
        .unwrap()
        .success());
    let other_sha = seed_cache_object(
        &root,
        &std::fs::read(stripped).unwrap(),
        CACHE_TEST_CONTEXT,
        env!("CARGO_PKG_VERSION"),
    );
    let other_path = root.join(OBJECTS_DIR).join(&other_sha);
    assert_ne!(
        other_path, object,
        "the two executables have distinct addresses"
    );
    let first = Worker::spawn_program(&object);
    let second = Worker::spawn_program(&other_path);
    first.capabilities().unwrap();
    second.capabilities().unwrap();
    let lease = second.session().unwrap().lease.0;
    assert!(root.join(LEASES_DIR).join(format!("{lease}.json")).exists());

    let (token, _handle) = token();
    let outcome = first.collect_cache(&token, CACHE_TEST_CONTEXT).unwrap();
    assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
    assert!(outcome.report.removed_objects.is_empty());
    assert_eq!(outcome.report.kept_objects, 2);
    assert!(object.exists());
    assert!(other_path.exists());
    first.shutdown().unwrap();
    second.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn cache_gc_refuses_corrupt_lease_before_any_retirement() {
    use strop_core::worker::cache_record::{LEASES_DIR, OBJECTS_DIR};
    use strop_workspace::operation::FsFailureKind;

    let (_directory, object, root) = cached_worker_binary();
    let candidate = seed_cache_object(&root, b"candidate worker", CACHE_TEST_CONTEXT, "0.34.0");
    seed_cache_lease(&root, 42, &candidate);
    std::fs::write(root.join(LEASES_DIR).join("42.json"), b"not json").unwrap();
    let (token, _handle) = token();
    let worker = Worker::spawn_program(&object);

    let outcome = worker.collect_cache(&token, CACHE_TEST_CONTEXT).unwrap();
    assert_eq!(outcome.failure.unwrap().kind, FsFailureKind::Protocol);
    assert!(outcome.report.removed_objects.is_empty());
    assert!(root.join(OBJECTS_DIR).join(candidate).exists());
    worker.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn cache_gc_holds_the_worker_lease_lock_through_retirement() {
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;

    use strop_core::worker::cache_record::{CACHE_LOCK_FILE, OBJECTS_DIR};

    let (_directory, object, root) = cached_worker_binary();
    let candidate = seed_cache_object(&root, b"exclusive retirement", CACHE_TEST_CONTEXT, "0.34.0");
    let (token, _handle) = token();
    let worker = Worker::spawn_program(&object);
    worker.capabilities().unwrap();
    let held = std::fs::File::open(root.join(CACHE_LOCK_FILE)).unwrap();
    held.lock().unwrap();
    let (started_tx, started_rx) = channel();
    let (result_tx, result_rx) = channel();
    let collector = worker.clone();
    let request_token = token.clone();
    let request = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        result_tx
            .send(collector.collect_cache(&request_token, CACHE_TEST_CONTEXT))
            .unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        matches!(
            result_rx.recv_timeout(Duration::from_millis(250)),
            Err(RecvTimeoutError::Timeout)
        ),
        "the cache pass must not unlink while another process holds the lock"
    );
    assert!(root.join(OBJECTS_DIR).join(&candidate).exists());
    drop(held);
    let outcome = result_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
    assert_eq!(outcome.report.removed_objects, vec![candidate.clone()]);
    assert!(!root.join(OBJECTS_DIR).join(candidate).exists());
    request.join().unwrap();
    worker.shutdown().unwrap();
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
        store: None,
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
        store: None,
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
            store: None,
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
