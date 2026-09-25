//! 0058 WK07 end-to-end over real OpenSSH: consent-gated deployment of
//! the matching worker through strop-worker-deploy's state machine over
//! a dedicated SFTP connection, then the `--worker-stdio` transport over
//! the ssh exec channel — deploy → handshake → read/write/notify parity
//! against a real sshd on localhost. A restricted (SFTP-only) host keeps
//! its honest read-only SFTP behavior with a typed refusal for anything
//! more, and an interrupted deploy leaves no partial activation.
//!
//! Like remote_ssh.rs, there is no network: `sshd -i` speaks the real
//! SSH protocol over a ProxyCommand pipe. Gated by
//! STROP_REQUIRE_SSH_TESTS=1 (the compose test stage runs sshd).
#![cfg(unix)]

use std::io::Read as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{LazyLock, Mutex, MutexGuard};

use strop_remote::bootstrap::{self, BootstrapError, EndpointFacts};
use strop_remote::deploy_provider::SftpDeployProvider;
use strop_remote::worker_transport;
use strop_worker_client::Worker;
use strop_worker_deploy::deploy::{
    deploy, ArtifactSupply, Consent, DeployOrigin, DeployOutcome, DeployRefusal, DeployRequest,
};
use strop_worker_deploy::manifest::ReleaseCatalog;
use strop_worker_deploy::provider::DeployProvider;
use strop_worker_deploy::MAX_WORKER_BYTES;
use strop_workspace::operation::{OperationIntent, OperationKind};
use strop_workspace::{RemoteEndpoint, ResourceLocation};

/// Tests in this binary share one sshd fixture and one remote cache;
/// deployments mutate both, so they run strictly serially.
static SERIAL: Mutex<()> = Mutex::new(());
static FIXTURE: LazyLock<Fixture> = LazyLock::new(Fixture::new);

fn required() -> bool {
    std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() == Some(std::ffi::OsStr::new("1"))
}

fn serial() -> Option<MutexGuard<'static, ()>> {
    if !required() {
        return None;
    }
    Some(SERIAL.lock().unwrap_or_else(|error| error.into_inner()))
}

fn fixture() -> &'static Fixture {
    &FIXTURE
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn successful(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

struct Fixture {
    directory: tempfile::TempDir,
    /// The deployable worker artifact: the real binary, stripped under
    /// the stage's byte bound.
    artifact: PathBuf,
    artifact_sha256: String,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("bin")).unwrap();
        std::fs::create_dir(root.join("home")).unwrap();
        std::fs::create_dir(root.join("cache")).unwrap();
        for name in ["host", "client"] {
            successful(
                Command::new("ssh-keygen")
                    .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                    .arg(root.join(name)),
            );
        }
        let public = std::fs::read_to_string(root.join("host.pub")).unwrap();
        std::fs::write(root.join("known_hosts"), format!("fixture {public}")).unwrap();
        std::fs::copy(root.join("client.pub"), root.join("authorized_keys")).unwrap();
        let username = String::from_utf8(successful(Command::new("id").arg("-un")).stdout).unwrap();
        let base = format!(
            "HostKey {}\nAuthorizedKeysFile {}\nStrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin yes\nLogLevel ERROR\nSubsystem sftp internal-sftp\n",
            root.join("host").display(),
            root.join("authorized_keys").display()
        );
        // sshd scrubs the client process's environment; SetEnv is the
        // config-owned channel that pins the remote cache base inside
        // the fixture for both sshd configurations.
        let set_env = format!("SetEnv XDG_CACHE_HOME={}\n", quoted(&root.join("cache")));
        let server = root.join("sshd_config");
        std::fs::write(&server, format!("{base}{set_env}")).unwrap();
        let sftp_only = root.join("sshd_sftp_only_config");
        std::fs::write(
            &sftp_only,
            format!("{base}{set_env}ForceCommand internal-sftp\n"),
        )
        .unwrap();
        // First matching block wins: the restricted alias gets its own
        // sshd (ForceCommand internal-sftp), every other alias the
        // full shell + subsystem sshd.
        let config = root.join("ssh_config");
        std::fs::write(&config, format!(
            "Host sftponly\n ProxyCommand /usr/sbin/sshd -i -e -f {}\nHost *\n IdentityFile {}\n HostName 127.0.0.1\n User {}\n IdentitiesOnly yes\n IdentityAgent none\n HostKeyAlias fixture\n UserKnownHostsFile {}\n GlobalKnownHostsFile /dev/null\n ProxyCommand /usr/sbin/sshd -i -e -f {}\n StrictHostKeyChecking no\n",
            quoted(&sftp_only),
            root.join("client").display(),
            username.trim(),
            root.join("known_hosts").display(),
            quoted(&server)
        )).unwrap();
        // The wrapper selects the private config AND pins the remote
        // cache base: the ssh client's environment is inherited by the
        // ProxyCommand sshd and hence by every remote shell, so the
        // audited discovery line resolves $HOME/$XDG_CACHE_HOME inside
        // the fixture — never the developer's real cache.
        let wrapper = root.join("bin/ssh");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexport HOME={}\nexport XDG_CACHE_HOME={}\nexec /usr/bin/ssh -F {} \"$@\"\n",
                quoted(&root.join("home")),
                quoted(&root.join("cache")),
                quoted(&config)
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![root.join("bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        // Process-global by design: the provider and bootstrap spawn
        // `ssh` by name, and this binary's tests all share the fixture.
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());

        let artifact = root.join("strop-worker");
        std::fs::copy(env!("CARGO_BIN_EXE_strop"), &artifact).unwrap();
        // Debug binaries exceed the stage bound with symbols; strip a
        // copy (the deployed bytes' digest is computed after stripping).
        let stripped = Command::new("strip").arg(&artifact).status();
        match stripped {
            Ok(status) if status.success() => {}
            _ => eprintln!("strip unavailable; deploying the unstripped binary"),
        }
        let bytes = std::fs::metadata(&artifact).unwrap().len();
        assert!(
            bytes <= MAX_WORKER_BYTES,
            "worker artifact is {bytes} bytes, over the {MAX_WORKER_BYTES}-byte bound"
        );
        let artifact_sha256 = sha256(&std::fs::read(&artifact).unwrap());
        Self {
            directory,
            artifact,
            artifact_sha256,
        }
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn endpoint(alias: &str) -> RemoteEndpoint {
    RemoteEndpoint::parse(&format!("ssh://{alias}")).unwrap()
}

/// A live token: the handle must be held for the whole test (dropping
/// it cancels).
fn token() -> (
    strop_core::worker::CancelToken,
    strop_core::worker::CancelHandle,
) {
    strop_core::worker::CancelToken::standalone()
}

/// The catalog of this build's own release for the same-binary supply:
/// the artifact entry's digest is computed over the exact bytes the
/// test uploads (the caller-side verification the deploy contract
/// requires of its supply).
fn catalog(target: &str, sha256: &str, bytes: u64) -> ReleaseCatalog {
    let version = env!("CARGO_PKG_VERSION");
    let body = format!(
        r#"{{
  "schema": 1,
  "product": "strop",
  "version": "{version}",
  "tag": "v{version}",
  "published_at": "2026-09-24T00:00:00Z",
  "artifacts": [
    {{
      "target": "{target}",
      "name": "strop-{version}-{target}.tar.gz",
      "sha256": "{sha256}",
      "bytes": {bytes},
      "url": "https://example.invalid/v{version}/strop-{version}-{target}.tar.gz"
    }}
  ],
  "worker": {{
    "protocol": 1,
    "min_editor": "0.35.0",
    "targets": ["{target}"]
  }}
}}"#
    );
    ReleaseCatalog::parse(body.as_bytes()).expect("test catalog parses")
}

fn request(
    target: &str,
    consent: Consent,
    supply: ArtifactSupply,
    sha256: &str,
    bytes: u64,
) -> DeployRequest {
    DeployRequest {
        catalog: catalog(target, sha256, bytes),
        editor_version: env!("CARGO_PKG_VERSION").to_string(),
        consent,
        supply,
        online: false,
    }
}

fn discover(alias: &RemoteEndpoint, token: &strop_core::worker::CancelToken) -> EndpointFacts {
    bootstrap::discover(alias, token).expect("discovery resolves over the exec channel")
}

fn provider(
    alias: &RemoteEndpoint,
    facts: &EndpointFacts,
    token: &strop_core::worker::CancelToken,
) -> SftpDeployProvider {
    SftpDeployProvider::connect(alias, facts, token).expect("SFTP provider connects")
}

/// One full deployment of the fixture artifact to `alias`.
fn deploy_worker(
    alias: &RemoteEndpoint,
    facts: &EndpointFacts,
    consent: Consent,
    token: &strop_core::worker::CancelToken,
) -> (
    SftpDeployProvider,
    strop_worker_deploy::deploy::ReadyDeployment,
) {
    let size = std::fs::metadata(&fixture().artifact).unwrap().len();
    deploy_worker_with(
        alias,
        facts,
        consent,
        token,
        &fixture().artifact,
        &fixture().artifact_sha256,
        size,
    )
}

/// One full deployment of an explicit artifact to `alias`.
#[allow(clippy::too_many_arguments)]
fn deploy_worker_with(
    alias: &RemoteEndpoint,
    facts: &EndpointFacts,
    consent: Consent,
    token: &strop_core::worker::CancelToken,
    artifact: &Path,
    sha: &str,
    size: u64,
) -> (
    SftpDeployProvider,
    strop_worker_deploy::deploy::ReadyDeployment,
) {
    let provider = provider(alias, facts, token);
    let target = facts.local_binary_target().expect("same-platform fixture");
    let report = deploy(
        &provider,
        &request(
            target,
            consent,
            ArtifactSupply::LocalBinary {
                path: artifact.to_path_buf(),
            },
            sha,
            size,
        ),
    );
    match report.outcome {
        DeployOutcome::Ready(ready) => (provider, ready),
        other => panic!(
            "deployment must be ready, got {other:?} (trace {:?})",
            report.trace
        ),
    }
}

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
    let (_provider, ready) = deploy_worker(
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
    worker.shutdown().unwrap();
}

#[test]
fn first_deploy_is_consent_gated_then_quietly_reused() {
    let Some(_serial) = serial() else { return };
    // A distinct context (same sshd, new alias) is a virgin endpoint:
    // no receipt authorizes it yet.
    let _ = fixture();
    let host = endpoint("virgin");
    let (token, _handle) = token();
    let facts = discover(&host, &token);
    let provider2 = provider(&host, &facts, &token);
    let target = facts.local_binary_target().unwrap();
    // A distinct content address (trailing bytes do not disturb ELF
    // execution): the cache legitimately reuses an already-present
    // object without fresh consent, so the consent proof needs an
    // object that is absent.
    let virgin_artifact = fixture().root().join("strop-worker-virgin");
    let mut bytes = std::fs::read(&fixture().artifact).unwrap();
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
        DeployOutcome::Ready(ready) => assert_eq!(ready.origin, DeployOrigin::Reused),
        other => panic!("an authorized endpoint quietly reuses, got {other:?}"),
    }
}

#[test]
fn interrupted_deploy_leaves_no_partial_activation() {
    let Some(_serial) = serial() else { return };
    let _ = fixture();
    let host = endpoint("blocked");
    let (token, _handle) = token();
    let facts = discover(&host, &token);
    let provider = provider(&host, &facts, &token);
    let target = facts.local_binary_target().unwrap();

    // A distinct "worker" (the real binary plus one trailing byte: ELF
    // execution is unaffected, the content address differs) so this
    // test's cache entries never alias another test's.
    let altered = fixture().root().join("strop-worker-altered");
    let mut bytes = std::fs::read(&fixture().artifact).unwrap();
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
    let Some(_serial) = serial() else { return };
    let _ = fixture();
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
    // No receipt may name the restricted context (receipts for other
    // contexts from this binary's other tests are legitimate).
    let receipts = fixture().root().join("cache/strop-worker/receipts");
    if receipts.exists() {
        for entry in std::fs::read_dir(receipts).unwrap() {
            let body = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            assert!(
                !body.contains("ssh://sftponly"),
                "a receipt appeared for the restricted host: {body}"
            );
        }
    }

    // The read-only SFTP path behaves exactly as it always has.
    let client = strop_remote::RemoteClient::new();
    let scope = tempfile::tempdir_in(fixture().root()).unwrap();
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
