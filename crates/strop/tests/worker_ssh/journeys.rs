use super::*;

#[test]
fn deploy_handshake_read_write_notify_parity_over_real_sshd() {
    let Some(_serial) = serial() else { return };
    let _ = fixture();
    let host = endpoint("fixture");
    let (token, _handle) = token();
    let facts = discover(&host, &token);
    assert_eq!(facts.principal, {
        String::from_utf8(successful(Command::new("id").arg("-u")).stdout)
            .unwrap()
            .trim()
            .to_string()
    });
    let (provider, ready) = deploy_worker(
        &host,
        &facts,
        Consent::Granted {
            action: "ssh-parity-test".into(),
        },
        &token,
    );
    assert_eq!(ready.handshake.worker.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        ready.handshake.worker.target,
        facts.local_binary_target().unwrap()
    );

    let worker: Worker = worker_transport::worker(
        &host,
        &ready.object.path,
        facts.local_binary_target().unwrap(),
    );
    let capabilities = worker.capabilities().unwrap();
    assert!(capabilities.read && capabilities.write && capabilities.list);
    let first = worker.session().expect("handshake captured the lease");
    let layout = strop_worker_deploy::cache::resolve(&provider).unwrap();
    assert!(
        provider
            .lstat(&layout.lease(first.lease.0))
            .unwrap()
            .is_some(),
        "the actual client session must have a cache lease, not just the deployment probe"
    );
    let lock_path = format!("{}/{}", layout.root(), CACHE_LOCK_FILE);
    let lock_stat = std::fs::symlink_metadata(&lock_path).unwrap();
    assert!(lock_stat.file_type().is_file());
    assert_eq!(lock_stat.mode() & 0o077, 0, "cache lock is private");
    assert_eq!(lock_stat.uid(), facts.principal.parse::<u32>().unwrap());
    let lock_inode = lock_stat.ino();

    // The actual SSH worker, not a local fake provider, retires an
    // unleased receipted object in the selected context while keeping
    // its own executable and live lease intact.
    let old_bytes = b"old unleased worker";
    let old_sha = sha256(old_bytes);
    provider.write(&layout.object(&old_sha), old_bytes).unwrap();
    provider.set_mode(&layout.object(&old_sha), 0o500).unwrap();
    let old_receipt = strop_core::worker::cache_record::CacheReceipt {
        schema: strop_core::worker::cache_record::RECEIPT_SCHEMA,
        context: provider.endpoint().context.clone(),
        principal: provider.endpoint().principal.clone(),
        version: "0.34.0".into(),
        target: provider.endpoint().target.clone(),
        object_sha256: old_sha.clone(),
        object_bytes: old_bytes.len() as u64,
        tarball_sha256: "b".repeat(64),
    };
    provider
        .write(
            &layout.receipt(&old_sha),
            &serde_json::to_vec(&old_receipt).unwrap(),
        )
        .unwrap();
    let maintenance = worker
        .collect_cache(&token, &provider.endpoint().context)
        .unwrap();
    assert!(maintenance.failure.is_none(), "{:?}", maintenance.failure);
    assert!(maintenance.report.removed_objects.contains(&old_sha));
    assert!(provider.lstat(&layout.object(&old_sha)).unwrap().is_none());
    assert!(provider.lstat(&layout.receipt(&old_sha)).unwrap().is_none());
    assert!(provider.lstat(&ready.object.path).unwrap().is_some());

    // Write parity: a frozen content stream applied by the remote
    // worker lands exactly on the remote filesystem (== this
    // filesystem through localhost sshd). Buffer-copy is the protocol's
    // content-carrying write; CreateFile alone creates empty.
    let scope = tempfile::tempdir_in(fixture().root()).unwrap();
    let source_file = scope.path().join("source.txt");
    std::fs::write(&source_file, "seed\n").unwrap();
    let written = scope.path().join("worker-written.txt");
    let intents = vec![OperationIntent {
        kind: OperationKind::Copy,
        source: Some(ResourceLocation::local(source_file.clone())),
        destination: Some(ResourceLocation::local(written.clone())),
        copy_version: strop_workspace::operation::CopyVersion::Buffer,
        expected_content: None,
        store: None,
    }];
    let (steps, refused) = worker.prepare(&token, intents, None).unwrap();
    assert!(refused.is_empty());
    assert_eq!(steps.len(), 1);
    let receipts = worker
        .apply(&token, steps, Some(b"through the ssh worker\n"))
        .unwrap();
    assert_eq!(receipts.len(), 1);
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt.outcome.is_committed()),
        "the remote write commits: {receipts:?}"
    );
    assert_eq!(
        std::fs::read(&written).unwrap(),
        b"through the ssh worker\n"
    );
    // Verify parity: the same receipt verifies against fresh evidence.
    let verified = worker.verify(&token, receipts[0].clone()).unwrap();
    assert!(matches!(
        verified,
        strop_workspace::operation::VerifiedOutcome::Committed(_)
            | strop_workspace::operation::VerifiedOutcome::Unchanged
    ));

    // Read/observe/list parity against the same files.
    let mut payload = worker
        .read(&token, ResourceLocation::local(written.clone()), 0, None)
        .unwrap();
    let mut bytes = Vec::new();
    payload.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"through the ssh worker\n");
    let observations = worker
        .observe(&token, vec![ResourceLocation::local(written.clone())])
        .unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(
        observations[0].value.as_ref().map(|value| value.kind),
        Some(strop_workspace::EntryKind::File)
    );
    let snapshot = worker
        .list(&token, ResourceLocation::local(scope.path().to_path_buf()))
        .unwrap();
    assert!(snapshot
        .entries
        .iter()
        .any(|entry| entry.name.as_path() == Path::new("worker-written.txt")));

    // Notify parity: a subscription through the ssh worker reports the
    // remote-side change as a hint on the client's sink.
    let (events, hints) = std::sync::mpsc::channel();
    worker.set_event_sink(events);
    let (subscription, coverage) = worker
        .subscribe(
            &token,
            ResourceLocation::local(scope.path().to_path_buf()),
            true,
        )
        .unwrap();
    assert_eq!(coverage, strop_worker_protocol::NotifyCoverage::Native);
    std::fs::write(scope.path().join("hinted.txt"), "watch me\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let hinted = loop {
        match hints.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(strop_worker_protocol::Event::Notify {
                subscription: seen,
                hints: batch,
                ..
            }) if seen == subscription
                && batch.iter().any(|hint| {
                    hint.path == b"hinted.txt"
                        || hint.path == b".".as_slice()
                        || hint.path.is_empty()
                }) =>
            {
                break true
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break false,
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
    };
    assert!(
        hinted,
        "the ssh worker relayed a notify hint for hinted.txt"
    );
    worker.unsubscribe(&token, subscription).unwrap();

    // Reconnect re-handshakes fresh: kill the worker, then the next
    // request spawns a new incarnation (old handles die with the old).
    let pid = worker.worker_pid().expect("ssh transport carries a child");
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        match worker.session() {
            None => break,
            _ if std::time::Instant::now() > deadline => {
                panic!("the dead incarnation never retired")
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    worker
        .observe(&token, vec![ResourceLocation::local(written.clone())])
        .unwrap();
    let second = worker.session().expect("a fresh incarnation handshook");
    assert_ne!(
        first.incarnation, second.incarnation,
        "reconnect re-handshakes a fresh incarnation"
    );
    assert!(
        provider
            .lstat(&layout.lease(second.lease.0))
            .unwrap()
            .is_some(),
        "reconnected worker must pin its own new session before serving"
    );
    assert_eq!(
        std::fs::symlink_metadata(&lock_path).unwrap().ino(),
        lock_inode,
        "reconnect must reuse the same lock inode"
    );
    worker.shutdown().unwrap();
    assert!(
        std::fs::symlink_metadata(&lock_path).is_ok(),
        "worker retirement must not unlink the shared cache lock"
    );
}
