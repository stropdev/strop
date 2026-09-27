//! 0058 WK07 end-to-end over real OpenSSH: consent-gated deployment of
//! the matching worker through strop-worker-deploy's state machine over
//! a dedicated SFTP connection, then the `--worker-stdio` transport over
//! the ssh exec channel — deploy → handshake → read/write/notify parity
//! against a real sshd on localhost. A restricted (SFTP-only) host keeps
//! its honest read-only SFTP behavior with a typed refusal for anything
//! more, and an interrupted deploy leaves no partial activation.
//!
//! Loopback TCP carries the real SSH protocol to two scoped OpenSSH daemons
//! (ordinary and SFTP-only). Gated by STROP_REQUIRE_SSH_TESTS=1.
#![cfg(unix)]
#[path = "worker_ssh/journeys.rs"]
mod journeys;
#[path = "worker_ssh/perf.rs"]
mod perf;
#[path = "worker_ssh/refusals.rs"]
mod refusals;

use std::io::Read as _;
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

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

/// Every test owns its private sshd listeners, cache and PATH wrapper.
/// The lock keeps process-global PATH scoped until the daemons are reaped.
static SERIAL: Mutex<()> = Mutex::new(());

struct ScopedFixture {
    fixture: Fixture,
    _serial: MutexGuard<'static, ()>,
}

fn required() -> bool {
    std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() == Some(std::ffi::OsStr::new("1"))
}

fn serial() -> Option<ScopedFixture> {
    if !required() {
        return None;
    }
    let serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    Some(ScopedFixture {
        fixture: Fixture::new(),
        _serial: serial,
    })
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

struct Sshd {
    child: Option<Child>,
    port: u16,
}

impl Sshd {
    fn start(root: &Path, config: &Path, name: &str) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let log = root.join(format!("{name}.sshd.log"));
        let stderr = std::fs::File::create(&log).unwrap();
        let child = Command::new("/usr/sbin/sshd")
            .args(["-D", "-e", "-f"])
            .arg(config)
            .args(["-p", &port.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .spawn()
            .unwrap();
        let mut daemon = Self {
            child: Some(child),
            port,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return daemon;
            }
            if daemon.child.as_mut().unwrap().try_wait().unwrap().is_some()
                || Instant::now() >= deadline
            {
                panic!(
                    "scoped {name} sshd did not bind loopback: {}",
                    std::fs::read_to_string(&log).unwrap_or_default()
                );
            }
            std::thread::yield_now();
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    /// The deployable worker artifact: the real binary, stripped under
    /// the stage's byte bound.
    artifact: PathBuf,
    artifact_sha256: String,
    sshd: Sshd,
    sftp_sshd: Sshd,
    original_path: std::ffi::OsString,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        let root = directory.path();
        let original_path = std::env::var_os("PATH").unwrap();
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
            "ListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\nStrictModes no\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin yes\nLogLevel ERROR\nSubsystem sftp internal-sftp\n",
            root.join("host").display(),
            root.join("authorized_keys").display()
        );
        // sshd scrubs the client process's environment; SetEnv is the
        // config-owned channel that pins the remote cache base inside
        // the fixture for both sshd configurations.
        let set_env = format!("SetEnv XDG_CACHE_HOME={}\n", quoted(&root.join("cache")));
        let server_config = root.join("sshd_config");
        std::fs::write(&server_config, format!("{base}{set_env}")).unwrap();
        let sftp_config = root.join("sshd_sftp_only_config");
        std::fs::write(
            &sftp_config,
            format!("{base}{set_env}ForceCommand internal-sftp\n"),
        )
        .unwrap();
        // A real peer socket reaches macOS BSM audit admission; `sshd -i`
        // on a ProxyCommand pipe supplies UNKNOWN and fails before auth.
        let sshd = Sshd::start(root, &server_config, "full");
        let sftp_sshd = Sshd::start(root, &sftp_config, "restricted");
        let config = root.join("ssh_config");
        std::fs::write(&config, format!(
            "Host sftponly\n HostName 127.0.0.1\n Port {}\nHost *\n IdentityFile {}\n HostName 127.0.0.1\n Port {}\n User {}\n IdentitiesOnly yes\n IdentityAgent none\n HostKeyAlias fixture\n UserKnownHostsFile {}\n GlobalKnownHostsFile /dev/null\n StrictHostKeyChecking no\n",
            sftp_sshd.port,
            root.join("client").display(),
            sshd.port,
            username.trim(),
            root.join("known_hosts").display(),
        )).unwrap();
        // The wrapper selects the private config and cache base; server
        // SetEnv pins the remote side without touching the real $HOME.
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
        paths.extend(std::env::split_paths(&original_path));
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
            sshd,
            sftp_sshd,
            original_path,
        }
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.sshd.stop();
        self.sftp_sshd.stop();
        std::env::set_var("PATH", &self.original_path);
    }
}

#[test]
fn scoped_loopback_ssh_daemons_are_reaped_after_each_fixture() {
    let Some(scope) = serial() else { return };
    let pids = [
        scope.fixture.sshd.child.as_ref().unwrap().id(),
        scope.fixture.sftp_sshd.child.as_ref().unwrap().id(),
    ];
    drop(scope);
    for pid in pids {
        // SAFETY: signal 0 only observes the PID; the owning Child was
        // killed and waited by this fixture's Drop before this check.
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) };
        assert_eq!(alive, -1, "this fixture left an sshd child alive");
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH),
            "the exact child PID must have been reaped"
        );
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
    fixture: &Fixture,
    alias: &RemoteEndpoint,
    facts: &EndpointFacts,
    consent: Consent,
    token: &strop_core::worker::CancelToken,
) -> (
    SftpDeployProvider,
    strop_worker_deploy::deploy::ProbedDeployment,
) {
    let size = std::fs::metadata(&fixture.artifact).unwrap().len();
    deploy_worker_with(
        alias,
        facts,
        consent,
        token,
        &fixture.artifact,
        &fixture.artifact_sha256,
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
    let target = facts.target.as_str();
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
    let Some(_host_fixture) = serial() else {
        return;
    };
    let alias = endpoint("fixture");
    let (setup_token, _owner) = token();
    let facts = bootstrap::discover(&alias, &setup_token).unwrap();
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
    let Some(host_fixture) = serial() else { return };
    let fixture = &host_fixture.fixture;
    let host = endpoint("fixture");
    let (token, _handle) = token();
    let facts = discover(&host, &token);
    let (_provider, ready) = deploy_worker(
        fixture,
        &host,
        &facts,
        Consent::Granted {
            action: "ssh-save-parity-test".into(),
        },
        &token,
    );
    let worker: Worker = worker_transport::worker(&host, &ready.object.path, &facts.target);
    let scope = tempfile::tempdir_in(fixture.root()).unwrap();
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
    let (steps, refused, session) = worker
        .prepare(
            &token,
            vec![store(&created, None, false, false, b"created\n")],
            None,
        )
        .unwrap();
    assert!(refused.is_empty() && steps.len() == 1, "{refused:?}");
    let receipts = worker
        .apply(&token, session, steps, Some(b"created\n"))
        .unwrap();
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
    let (steps, refused, _session) = worker
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
    let (steps, refused, session) = worker
        .prepare(
            &token,
            vec![store(&file, Some(baseline), true, false, b"mine\n")],
            None,
        )
        .unwrap();
    assert!(refused.is_empty() && steps.len() == 1, "{refused:?}");
    let receipts = worker
        .apply(&token, session, steps, Some(b"mine\n"))
        .unwrap();
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
    let (steps, refused, session) = worker
        .prepare(
            &token,
            vec![store(&file, Some(baseline), false, false, b"yours")],
            None,
        )
        .unwrap();
    assert!(refused.is_empty() && steps.len() == 1, "{refused:?}");
    let operation = steps.into_iter().next().unwrap();
    let receipts = worker
        .apply(&token, session, vec![operation.clone()], Some(b"yours"))
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
    let (steps, refused, _session) = worker
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
    let Some(host_fixture) = serial() else { return };
    let fixture = &host_fixture.fixture;
    let host = endpoint("fixture");
    let (token, _handle) = token();
    let facts = discover(&host, &token);
    let (_provider, ready) = deploy_worker(
        fixture,
        &host,
        &facts,
        Consent::Granted {
            action: "ssh-read-parity-test".into(),
        },
        &token,
    );
    let worker: Worker = worker_transport::worker(&host, &ready.object.path, &facts.target);
    let scope = tempfile::tempdir_in(fixture.root()).unwrap();
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
