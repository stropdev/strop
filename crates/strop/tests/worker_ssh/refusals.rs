use super::*;

#[test]
fn first_deploy_is_consent_gated_then_quietly_reused() {
    let Some(host_fixture) = serial() else { return };
    // A distinct context (same sshd, new alias) is a virgin endpoint:
    // no receipt authorizes it yet.
    let fixture = &host_fixture.fixture;
    let host = endpoint("virgin");
    let (token, _handle) = token();
    let facts = discover(&host, &token);
    let provider2 = provider(&host, &facts, &token);
    let target = facts.target.as_str();
    // A distinct content address (trailing bytes do not disturb ELF
    // execution): the cache legitimately reuses an already-present
    // object without fresh consent, so the consent proof needs an
    // object that is absent.
    let virgin_artifact = fixture.root().join("strop-worker-virgin");
    let mut bytes = std::fs::read(&fixture.artifact).unwrap();
    bytes.extend_from_slice(b"\0\0");
    std::fs::write(&virgin_artifact, &bytes).unwrap();
    let sha = sha256(&bytes);
    let size = bytes.len() as u64;
    let supply = || ArtifactSupply::LocalBinary {
        path: virgin_artifact.clone(),
    };

    let report = deploy(
        &provider2,
        &request(target, Consent::Absent, supply(), &sha, size),
    );
    match &report.outcome {
        DeployOutcome::Refused(DeployRefusal::ConsentRequired {
            context,
            version,
            target: refused_target,
            destination,
            ..
        }) => {
            assert_eq!(context, "ssh://virgin");
            assert_eq!(version, env!("CARGO_PKG_VERSION"));
            assert_eq!(refused_target, target);
            // The precise destination travels in the refusal (0058 §5).
            assert!(destination.contains("strop-worker/objects/"));
        }
        other => panic!("a virgin endpoint must require consent, got {other:?}"),
    }

    let (_provider, ready) = deploy_worker_with(
        &host,
        &facts,
        Consent::Granted {
            action: "consent-test".into(),
        },
        &token,
        &virgin_artifact,
        &sha,
        size,
    );
    assert_eq!(ready.origin, DeployOrigin::Uploaded);

    // Previously authorized deployment quietly reuses the verified
    // cache — no fresh consent, no upload.
    let provider = provider(&host, &facts, &token);
    let report = deploy(
        &provider,
        &request(target, Consent::Absent, supply(), &sha, size),
    );
    match report.outcome {
        DeployOutcome::Probed(ready) => assert_eq!(ready.origin, DeployOrigin::Reused),
        other => panic!("an authorized endpoint quietly reuses, got {other:?}"),
    }
}

#[test]
fn interrupted_deploy_leaves_no_partial_activation() {
    let Some(host_fixture) = serial() else { return };
    let fixture = &host_fixture.fixture;
    let host = endpoint("blocked");
    let (token, _handle) = token();
    let facts = discover(&host, &token);
    let provider = provider(&host, &facts, &token);
    let target = facts.target.as_str();

    // A distinct "worker" (the real binary plus one trailing byte: ELF
    // execution is unaffected, the content address differs) so this
    // test's cache entries never alias another test's.
    let altered = fixture.root().join("strop-worker-altered");
    let mut bytes = std::fs::read(&fixture.artifact).unwrap();
    bytes.push(0);
    std::fs::write(&altered, &bytes).unwrap();
    let sha = sha256(&bytes);
    let size = bytes.len() as u64;
    let supply = || ArtifactSupply::LocalBinary {
        path: altered.clone(),
    };
    let consent = || Consent::Granted {
        action: "interruption-test".into(),
    };

    // Resolve (and thereby create) the cache layout, then loosen the
    // staging directory's privacy: the machine must refuse at cache
    // resolution, before a single upload byte.
    let layout = strop_worker_deploy::cache::resolve(&provider).expect("cache resolves");
    provider.set_mode(&layout.staging_dir(), 0o555).unwrap();
    let report = deploy(&provider, &request(target, consent(), supply(), &sha, size));
    match &report.outcome {
        DeployOutcome::Refused(DeployRefusal::Cache(
            strop_worker_deploy::cache::CacheError::NotPrivate { .. },
        )) => {}
        other => panic!("a loosened staging component must refuse, got {other:?}"),
    }
    // No partial activation: no object, no receipt, no staging residue.
    let object = layout.object(&sha);
    assert!(provider.lstat(&object).unwrap().is_none());
    assert!(provider.list(&layout.staging_dir()).unwrap().is_empty());
    assert!(provider.lstat(&layout.receipt(&sha)).unwrap().is_none());

    // Restore privacy, then seed a positively identified CORRUPT object
    // at the content address: the machine retires exactly that entry
    // and redeploys to Ready — never a partial activation retained.
    provider.set_mode(&layout.staging_dir(), 0o700).unwrap();
    provider.write(&object, b"corrupt").unwrap();
    let (_provider, ready) =
        deploy_worker_with(&host, &facts, consent(), &token, &altered, &sha, size);
    assert_eq!(ready.origin, DeployOrigin::Uploaded);
    assert_eq!(ready.object.sha256, sha);
}

#[test]
fn sftp_only_host_keeps_read_only_with_a_typed_refusal() {
    let Some(host_fixture) = serial() else { return };
    let fixture = &host_fixture.fixture;
    let host = endpoint("sftponly");
    let (token, _handle) = token();
    // The restricted account refuses the exec channel: discovery is the
    // typed refusal, and nothing was uploaded or executed.
    match bootstrap::discover(&host, &token) {
        Err(BootstrapError::Transport(detail)) => {
            assert!(!detail.is_empty(), "the refusal names the cause");
            assert!(
                !detail.contains("resolve hostname"),
                "the refusal is not a DNS artifact: {detail}"
            );
        }
        other => panic!("an SFTP-only host refuses discovery, got {other:?}"),
    }
    // No admission in this private fixture may create any worker receipt.
    let receipts = fixture.root().join("cache/strop-worker/receipts");
    if receipts.exists() {
        assert!(
            std::fs::read_dir(receipts).unwrap().next().is_none(),
            "a restricted SSH host created a worker receipt"
        );
    }

    // The read-only SFTP path behaves exactly as it always has.
    let client = strop_remote::RemoteClient::new();
    let scope = tempfile::tempdir_in(fixture.root()).unwrap();
    std::fs::write(scope.path().join("readable.txt"), "read only\n").unwrap();
    let location =
        strop_workspace::RemoteFile::from_path(host.clone(), scope.path().to_path_buf()).unwrap();
    let listed = client.list(&location.into(), &token).unwrap();
    assert!(listed
        .entries
        .iter()
        .any(|entry| entry.file.path() == scope.path().join("readable.txt")));
    let snapshot = client
        .read(
            &strop_workspace::RemoteFile::from_path(
                host.clone(),
                scope.path().join("readable.txt"),
            )
            .unwrap()
            .into(),
            strop_remote::ReadSelection::Full,
            &token,
        )
        .unwrap();
    let text = snapshot.buffer.text().to_string();
    assert_eq!(text, "read only\n");
    client.disconnect(&host).unwrap();
}
