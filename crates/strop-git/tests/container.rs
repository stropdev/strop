//! Real Docker Git journey through the verified container worker.
//! Busybox intentionally has no Git: admission and the missing-tool
//! refusal must be typed inside its own namespace, never through an
//! old shell supervisor or an analogous local Git process.
//! STROP_CONTAINER_TESTS=1 is the required native lane.
#![cfg(unix)]

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use strop_containers::{engine, inspect};
use strop_core::worker::CancelToken;
use strop_git::{GitExec, GitExecError, RepoTarget};
use strop_worker_client::ClientError;
use strop_worker_deploy::container::{ContainerProvider, ShellPolicy};
use strop_worker_deploy::deploy::{deploy, ArtifactSupply, Consent, DeployOutcome, DeployRequest};
use strop_worker_deploy::provider::DeployProvider;
use strop_worker_deploy::{ReleaseCatalog, MAX_WORKER_BYTES};
use strop_workspace::{operation::FsFailureKind, ContainerId};

fn required() -> bool {
    std::env::var_os("STROP_CONTAINER_TESTS").as_deref() == Some(OsStr::new("1"))
}

fn engine_available() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// The loud skip: the test calls this first.
fn gate(test: &str) -> bool {
    if !required() {
        eprintln!("skipping {test}: STROP_CONTAINER_TESTS is not 1");
        return false;
    }
    assert!(
        engine_available(),
        "{test}: STROP_CONTAINER_TESTS=1 requires an accessible Docker engine"
    );
    true
}

/// Hand one closure a real worker-issued cancellation token (CancelToken
/// cannot be constructed outside strop-core's worker machinery).
fn with_token<T>(work: impl FnOnce(CancelToken) -> T) -> T {
    let (tokens, receiver) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel::<()>();
    let owner = strop_core::worker::spawn(
        "git-container-test",
        |_| {},
        move |token| {
            tokens.send(token).expect("test receives token");
            let _ = waiting.recv();
            strop_core::worker::Outcome::Success(())
        },
    );
    let token = receiver.recv().expect("worker issued token");
    let result = work(token);
    drop(release);
    drop(owner);
    result
}

/// A unique run tag for labels and names — no registry state, so two
/// concurrent test runs never share a container.
fn tag() -> String {
    let mut bytes = [0u8; 8];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .expect("urandom is available on unix");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The labelled disposable fixture container. `Drop` removes exactly
/// this container by id — on panic too, and never anything unlabelled.
struct Fixture {
    id: String,
}

impl Fixture {
    fn launch(tag: &str, name: &str) -> Fixture {
        let output = Command::new("docker")
            .args([
                "run",
                "-d",
                "--label",
                &format!("strop-test-run={tag}"),
                "--name",
                name,
                "busybox",
                "sleep",
                "300",
            ])
            .output()
            .expect("docker CLI runs");
        assert!(
            output.status.success(),
            "docker run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Fixture {
            id: String::from_utf8(output.stdout)
                .expect("docker answers UTF-8")
                .trim()
                .to_string(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.id]).output();
    }
}

fn worker_binary(artifacts: &Path) -> PathBuf {
    let original = std::env::var_os("STROP_WORKER_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let executable = std::env::current_exe().unwrap();
            let root = executable
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .unwrap();
            let musl = root.join("x86_64-unknown-linux-musl/debug/strop");
            if musl.exists() {
                musl
            } else {
                root.join("debug/strop")
            }
        });
    assert!(original.is_file(), "matching worker artifact is required");
    let binary = artifacts.join("strop-worker");
    std::fs::copy(&original, &binary).expect("stage test worker in private artifacts");
    let stripped = Command::new("strip")
        .arg(&binary)
        .status()
        .expect("native container test requires strip");
    assert!(stripped.success(), "private test worker must be stripped");
    assert!(std::fs::metadata(&binary).unwrap().len() <= MAX_WORKER_BYTES);
    binary
}

fn catalog(binary: &Path, target: &str) -> ReleaseCatalog {
    let mut file = std::fs::File::open(binary).unwrap();
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        bytes += read as u64;
        digest.update(&buffer[..read]);
    }
    let version = env!("CARGO_PKG_VERSION");
    let body = serde_json::json!({
        "schema": 1, "product": "strop", "version": version,
        "tag": format!("v{version}"), "published_at": "1970-01-01T00:00:00Z",
        "artifacts": [{"target": target, "name": format!("strop-{version}-{target}.tar.gz"),
            "sha256": format!("{:x}", digest.finalize()), "bytes": bytes, "url": ""}],
        "worker": {"protocol": strop_worker_protocol::PROTOCOL_VERSION,
            "min_editor": strop_worker_deploy::MIN_EDITOR_VERSION, "targets": [target]}
    });
    ReleaseCatalog::parse(body.to_string().as_bytes()).unwrap()
}

#[test]
fn missing_git_in_container_is_typed_through_its_verified_worker() {
    if !gate("missing_git_in_container_is_typed_through_its_verified_worker") {
        return;
    }
    let tag = tag();
    let fixture = Fixture::launch(&tag, &format!("strop-wk10-git-{tag}"));
    let artifacts = tempfile::tempdir().unwrap();
    let binary = worker_binary(artifacts.path());
    let (worker, id) = with_token(|token| {
        let engine = engine(&token).unwrap();
        let identity = inspect(&engine, &fixture.id, &token).unwrap();
        let id = ContainerId::canonical(identity.id.clone()).unwrap();
        let provider =
            ContainerProvider::capture(&engine, &identity, ShellPolicy::Required, &token).unwrap();
        let target = &provider.endpoint().target;
        let report = deploy(
            &provider,
            &DeployRequest {
                catalog: catalog(&binary, target),
                editor_version: env!("CARGO_PKG_VERSION").into(),
                consent: Consent::Granted {
                    action: "container Git integration test".into(),
                },
                supply: ArtifactSupply::LocalBinary { path: binary },
                online: false,
            },
        );
        let DeployOutcome::Probed(ready) = report.outcome else {
            panic!("container deployment probe failed: {:?}", report.outcome);
        };
        (provider.worker(&ready.object.path), id)
    });
    let target = RepoTarget::Container {
        container: id.clone(),
        workdir: PathBuf::from("/"),
    };
    let exec = GitExec::for_target_routed(&target, Some(&worker)).unwrap();
    let argv: Vec<std::ffi::OsString> = vec!["status".into()];
    match with_token(|token| exec.run(&argv, &token)) {
        Err(GitExecError::Worker(strop_git::exec::WorkerGitError::Client(
            ClientError::Domain(failure),
        ))) => assert_eq!(failure.kind, FsFailureKind::Io),
        other => panic!("busybox's missing Git must fail inside its worker: {other:?}"),
    }
    assert!(with_token(|token| {
        strop_git::container::discover(&id, Path::new("/"), Some(&worker), &token)
    })
    .is_err());
    worker.shutdown().unwrap();
}
