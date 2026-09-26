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
#[path = "worker_ssh/journeys.rs"]
mod journeys;
#[path = "worker_ssh/perf.rs"]
mod perf;
#[path = "worker_ssh/refusals.rs"]
mod refusals;

use std::io::Read as _;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{LazyLock, Mutex, MutexGuard};

use strop_core::worker::cache_record::CACHE_LOCK_FILE;
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
        let provided = std::env::var_os("STROP_WORKER_BINARY");
        let source = provided
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_strop")));
        std::fs::copy(&source, &artifact).unwrap();
        // A supplied release artifact stays byte-identical. Only the
        // debug test binary needs stripping to satisfy the deploy bound.
        if provided.is_none() {
            let stripped = Command::new("strip").arg(&artifact).status();
            match stripped {
                Ok(status) if status.success() => {}
                _ => eprintln!("strip unavailable; deploying the unstripped binary"),
            }
        }
        let bytes = std::fs::metadata(&artifact).unwrap().len();
        assert!(
            bytes <= MAX_WORKER_BYTES,
            "worker artifact is {bytes} bytes, over the {MAX_WORKER_BYTES}-byte bound"
        );
        let artifact_sha256 = sha256_file(&artifact);
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
    use sha2::Digest as _;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> String {
    use sha2::Digest as _;
    let mut file = std::fs::File::open(path).unwrap();
    let mut digest = sha2::Sha256::new();
    let mut bytes = [0_u8; 65_536];
    loop {
        let count = file.read(&mut bytes).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    format!("{:x}", digest.finalize())
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
    let protocol = strop_worker_protocol::PROTOCOL_VERSION;
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
    "protocol": {protocol},
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
    strop_worker_deploy::deploy::ProbedDeployment,
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
    strop_worker_deploy::deploy::ProbedDeployment,
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
        DeployOutcome::Probed(ready) => (provider, ready),
        other => panic!(
            "deployment probe must be accepted, got {other:?} (trace {:?})",
            report.trace
        ),
    }
}

#[test]
fn concurrent_real_sftp_installers_accept_only_owned_private_cache_components() {
    let Some(_serial) = serial() else { return };
    let fixture = fixture();
    let alias = endpoint("fixture");
    let (setup_token, _owner) = token();
    let facts = bootstrap::discover(&alias, &setup_token).unwrap();
    let cache = fixture.directory.path().join("cache/strop-worker");
    if cache.exists() {
        // This is the test's own private fixture cache, never a user
        // or system path. All other tests in this binary hold SERIAL.
        std::fs::remove_dir_all(&cache).unwrap();
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut installers = Vec::new();
    for _ in 0..2 {
        let barrier = std::sync::Arc::clone(&barrier);
        let endpoint = alias.clone();
        let facts = facts.clone();
        installers.push(std::thread::spawn(move || {
            let (token, _owner) = token();
            let provider = provider(&endpoint, &facts, &token);
            barrier.wait();
            strop_worker_deploy::cache::resolve(&provider).unwrap()
        }));
    }
    barrier.wait();
    let roots: Vec<_> = installers
        .into_iter()
        .map(|installer| installer.join().unwrap().root().to_owned())
        .collect();
    assert_eq!(roots[0], roots[1]);
    let provider = provider(&alias, &facts, &setup_token);
    let stat = provider.lstat(&roots[0]).unwrap().unwrap();
    assert_eq!(stat.kind, strop_worker_deploy::provider::RemoteKind::Dir);
    assert_eq!(stat.mode & 0o7777, 0o700);
    assert_eq!(stat.owner, facts.principal);
}

/// WK09: protected document save (the Store intent) through the deployed
/// worker over real sshd — baseline-mtime conflict, permission
/// preservation, same-size content change and uncertainty→verify parity.
#[test]
fn store_save_parity_over_real_sshd() {
    use sha2::Digest as _;
    use strop_workspace::operation::{FsFailureKind, StepOutcome, StorePolicy, VerifiedOutcome};
    let Some(_serial) = serial() else { return };
    let _ = fixture();
    let host = endpoint("fixture");
    let (token, _handle) = token();
    let facts = discover(&host, &token);
    let (_provider, ready) = deploy_worker(
        &host,
        &facts,
        Consent::Granted {
            action: "ssh-save-parity-test".into(),
        },
        &token,
    );
    let worker: Worker = worker_transport::worker(
        &host,
        &ready.object.path,
        facts.local_binary_target().unwrap(),
    );
    let scope = tempfile::tempdir_in(fixture().root()).unwrap();
    let file = scope.path().join("note.txt");
    std::fs::write(&file, "before\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mtime = |path: &Path| -> strop_workspace::FileTime {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(path).unwrap();
        strop_workspace::FileTime {
            seconds: metadata.mtime(),
            nanos: metadata.mtime_nsec() as u32,
        }
    };
    let store = |path: &Path,
                 baseline: Option<strop_workspace::FileTime>,
                 force: bool,
                 expect_absent: bool,
                 content: &[u8]| {
        OperationIntent {
            kind: OperationKind::Store,
            source: None,
            destination: Some(ResourceLocation::local(path.to_path_buf())),
            copy_version: strop_workspace::operation::CopyVersion::Stored,
            expected_content: Some(sha2::Sha256::digest(content).into()),
            store: Some(StorePolicy {
                baseline,
                baseline_object: None,
                baseline_attributes: None,
                force,
                expect_absent,
                displayed: None,
            }),
        }
    };

    // Save-as create: an absent destination with an absent baseline.
    let created = scope.path().join("created.txt");
    let (steps, refused) = worker
        .prepare(
            &token,
            vec![store(&created, None, false, false, b"created\n")],
            None,
        )
        .unwrap();
    assert!(refused.is_empty() && steps.len() == 1, "{refused:?}");
    let receipts = worker.apply(&token, steps, Some(b"created\n")).unwrap();
    assert!(receipts[0].outcome.is_committed(), "{receipts:?}");
    assert_eq!(std::fs::read(&created).unwrap(), b"created\n");

    // Conflict: the baseline moved under the save (another writer's
    // mtime). The refusal is typed and the occupant is preserved.
    let baseline = mtime(&file);
    std::fs::write(&file, "theirs\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(42)),
        )
        .unwrap();
    let (steps, refused) = worker
        .prepare(
            &token,
            vec![store(&file, Some(baseline), false, false, b"mine\n")],
            None,
        )
        .unwrap();
    assert!(steps.is_empty());
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].failure.kind, FsFailureKind::Conflict);
    assert!(
        refused[0].failure.detail.contains("file changed on disk"),
        "{:?}",
        refused[0].failure
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"theirs\n");

    // Forced save-as over an occupied name commits, preserving the
    // occupant's permissions on the replaced file.
    let (steps, refused) = worker
        .prepare(
            &token,
            vec![store(&file, Some(baseline), true, false, b"mine\n")],
            None,
        )
        .unwrap();
    assert!(refused.is_empty() && steps.len() == 1, "{refused:?}");
    let receipts = worker.apply(&token, steps, Some(b"mine\n")).unwrap();
    assert!(receipts[0].outcome.is_committed(), "{receipts:?}");
    assert_eq!(std::fs::read(&file).unwrap(), b"mine\n");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o7777,
        0o640,
        "the save preserved the file's permissions"
    );

    // Same-size content change: the committed witness's digest separates
    // it from the overwritten bytes, and a receipt lost before its
    // witness arrived still verifies Committed by the intended digest.
    let baseline = mtime(&file);
    let (steps, refused) = worker
        .prepare(
            &token,
            vec![store(&file, Some(baseline), false, false, b"yours")],
            None,
        )
        .unwrap();
    assert!(refused.is_empty() && steps.len() == 1, "{refused:?}");
    let operation = steps.into_iter().next().unwrap();
    let receipts = worker
        .apply(&token, vec![operation.clone()], Some(b"yours"))
        .unwrap();
    assert!(receipts[0].outcome.is_committed(), "{receipts:?}");
    assert_eq!(std::fs::read(&file).unwrap(), b"yours");
    let lost = strop_workspace::operation::StepReceipt {
        step: 0,
        operation,
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
        "the landed same-size write verifies committed: {verified:?}"
    );
    // A receipt whose apply never ran verifies Unchanged, and a foreign
    // state is Unknown — never a guessed reconciliation.
    std::fs::write(&file, "zzzzz\n").unwrap();
    let baseline = mtime(&file);
    let (steps, refused) = worker
        .prepare(
            &token,
            vec![store(&file, Some(baseline), false, false, b"fresh\n")],
            None,
        )
        .unwrap();
    assert!(refused.is_empty() && steps.len() == 1, "{refused:?}");
    let never_applied = strop_workspace::operation::StepReceipt {
        step: 0,
        operation: steps.into_iter().next().unwrap(),
        outcome: StepOutcome::Unconfirmed {
            detail: "transport lost before apply".into(),
            observed_destination: None,
            recovery: None,
            publication: None,
        },
    };
    let verified = worker.verify(&token, never_applied.clone()).unwrap();
    assert!(
        matches!(verified, VerifiedOutcome::Unchanged),
        "the write that never ran verifies unchanged: {verified:?}"
    );
    std::fs::write(&file, "foreign").unwrap();
    let verified = worker.verify(&token, never_applied).unwrap();
    assert!(
        matches!(verified, VerifiedOutcome::Unknown { .. }),
        "a foreign state is never reconciled as ours: {verified:?}"
    );
    worker.shutdown().unwrap();
}

/// WK09: read/list parity through the deployed worker for native
/// (non-UTF8) name bytes and ranged read windows.
#[test]
fn read_list_native_bytes_and_windows_over_real_sshd() {
    let Some(_serial) = serial() else { return };
    let _ = fixture();
    let host = endpoint("fixture");
    let (token, _handle) = token();
    let facts = discover(&host, &token);
    let (_provider, ready) = deploy_worker(
        &host,
        &facts,
        Consent::Granted {
            action: "ssh-read-parity-test".into(),
        },
        &token,
    );
    let worker: Worker = worker_transport::worker(
        &host,
        &ready.object.path,
        facts.local_binary_target().unwrap(),
    );
    let scope = tempfile::tempdir_in(fixture().root()).unwrap();
    // A native byte name that is not valid UTF-8 round-trips exactly.
    use std::os::unix::ffi::OsStrExt;
    let raw = std::ffi::OsStr::from_bytes(b"native-\xFF-name.txt");
    let path = scope.path().join(raw);
    std::fs::write(&path, b"0123456789abcdef").unwrap();
    let snapshot = worker
        .list(&token, ResourceLocation::local(scope.path().to_path_buf()))
        .unwrap();
    assert!(
        snapshot
            .entries
            .iter()
            .any(|entry| entry.name.as_path().as_os_str().as_bytes() == b"native-\xFF-name.txt"),
        "the listing carries the exact native name bytes"
    );
    // A ranged read delivers exactly the window's bytes.
    let mut payload = worker
        .read(&token, ResourceLocation::local(path.clone()), 4, Some(8))
        .unwrap();
    let mut bytes = Vec::new();
    payload.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"456789ab", "the window is byte-exact");
    // A window past EOF announces only what remains.
    let mut payload = worker
        .read(&token, ResourceLocation::local(path), 12, Some(64))
        .unwrap();
    let mut bytes = Vec::new();
    payload.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"cdef");
    worker.shutdown().unwrap();
}
