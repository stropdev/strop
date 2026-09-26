//! Container-gated WK08 integration tests (0058): real deployment of
//! the verified worker into a live container over the scoped tar/exec
//! provider, the preinstalled shell-less path, incarnation staleness
//! and lease-close reaping — all against a real engine.
//!
//! Gating follows strop-containers: STROP_CONTAINER_TESTS=1 requires an
//! accessible Docker engine and never skips. The worker binary must be
//! the static musl build of this exact checkout: set STROP_WORKER_BINARY,
//! or build it the supported way (the compose `container-test` stage,
//! where target/debug/strop IS the static musl binary).
#![cfg(unix)]

mod common;

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{catalog, sha256_hex};
use strop_containers::{engine, inspect, ContainerIdentity, EngineRef};
use strop_core::worker::{cache_record::CACHE_LOCK_FILE, CancelToken};
use strop_worker_deploy::container::{ContainerProvider, ShellPolicy};
use strop_worker_deploy::deploy::{
    deploy, ArtifactSupply, Consent, DeployOrigin, DeployOutcome, DeployRequest,
};
use strop_worker_deploy::provider::{DeployProvider, RemoteKind};
use strop_worker_deploy::MAX_WORKER_BYTES;
use strop_workspace::operation::{CopyVersion, OperationIntent, OperationKind, StepOutcome};
use strop_workspace::ResourceLocation;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn required() -> bool {
    std::env::var_os("STROP_CONTAINER_TESTS").as_deref() == Some(OsStr::new("1"))
}

fn engine_available() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Explicit opt-in is a required gate, not permission for a vacuous pass.
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

fn token() -> (CancelToken, strop_core::worker::CancelHandle) {
    CancelToken::standalone()
}

fn tag() -> String {
    let mut bytes = [0u8; 8];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .expect("urandom is available on unix");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn run_docker(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("docker CLI runs");
    assert!(
        output.status.success(),
        "docker {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("docker answers UTF-8")
        .trim()
        .to_string()
}

/// True when the ELF at `path` has no PT_INTERP program header: a
/// static binary, runnable in any Linux image regardless of its libc.
fn is_static(path: &Path) -> bool {
    let bytes = std::fs::read(path).expect("worker binary reads");
    let header_ok = bytes.len() > 64
        && bytes[0..4] == [0x7f, b'E', b'L', b'F']
        && bytes[4] == 2
        && bytes[5] == 1;
    assert!(header_ok, "{}: not an ELF64-LE binary", path.display());
    let phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
    let phentsize = u16::from_le_bytes(bytes[54..56].try_into().unwrap()) as usize;
    let phnum = u16::from_le_bytes(bytes[56..58].try_into().unwrap()) as usize;
    (0..phnum).all(|index| {
        let at = phoff + index * phentsize;
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) != 3 // PT_INTERP
    })
}

/// The static musl worker binary for this exact checkout, stripped
/// under the deploy size bound into a private tempdir.
fn worker_binary(dir: &tempfile::TempDir) -> PathBuf {
    let candidate = std::env::var_os("STROP_WORKER_BINARY").map(PathBuf::from);
    let candidate = candidate.unwrap_or_else(|| {
        // <target>/[<triple>/]debug/deps/<this test> → workspace target root.
        let exe = std::env::current_exe().expect("current exe");
        let target_root = exe
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("test binary lives under target/");
        let plain = target_root.join("debug/strop");
        let musl = target_root.join("x86_64-unknown-linux-musl/debug/strop");
        if is_static(&plain) {
            plain
        } else {
            musl
        }
    });
    assert!(
        candidate.exists(),
        "no worker binary at {}; the container lane needs the static musl build \
         (docker compose run --build --rm container-test, or STROP_WORKER_BINARY=...)",
        candidate.display()
    );
    assert!(
        is_static(&candidate),
        "{}: dynamic binaries cannot run in arbitrary images; \
         build the static musl worker (compose container-test lane)",
        candidate.display()
    );
    let stripped = dir.path().join("strop-worker");
    let status = Command::new("strip")
        .arg(&candidate)
        .arg("-o")
        .arg(&stripped)
        .status()
        .expect("strip runs");
    assert!(status.success(), "strip {}", candidate.display());
    let len = std::fs::metadata(&stripped).expect("stripped binary").len();
    assert!(
        len <= MAX_WORKER_BYTES,
        "stripped worker is {len} bytes, over the {MAX_WORKER_BYTES} bound"
    );
    stripped
}

/// A labelled disposable fixture container; Drop removes exactly this
/// container by id, on panic too.
struct Fixture {
    id: String,
    image: Option<String>,
}

impl Fixture {
    /// Launch busybox without seeding (a read-only rootfs cannot take
    /// the seed write; refusal tests use this).
    fn busybox_raw(tag: &str, extra: &[&str]) -> Fixture {
        let name = format!("strop-wk08-{tag}");
        let mut args = vec!["run", "-d", "--label"];
        let label = format!("strop-test-run={tag}");
        args.push(&label);
        args.extend_from_slice(extra);
        args.extend_from_slice(&["--name", &name, "busybox", "sleep", "300"]);
        let id = run_docker(&args);
        Fixture { id, image: None }
    }

    fn busybox(tag: &str, extra: &[&str]) -> Fixture {
        let fixture = Self::busybox_raw(tag, extra);
        run_docker(&[
            "exec",
            &fixture.id,
            "sh",
            "-c",
            "mkdir -p /data && printf 'hello strop\\n' > /data/hello.txt",
        ]);
        fixture
    }

    /// A genuinely shell-less preinstalled-worker image: FROM scratch
    /// plus the static worker, nothing else — no sh, no coreutils, no
    /// Python. PID1 is the worker itself in stdio mode (stdin held open
    /// by the daemon); the tested lease execs a second instance.
    fn shellless(tag: &str, binary: &Path) -> Fixture {
        let image = format!("strop-wk08-shellless:{tag}");
        let context = tempfile::tempdir().expect("build context");
        std::fs::copy(binary, context.path().join("strop")).expect("copy worker");
        std::fs::write(
            context.path().join("Dockerfile"),
            "FROM scratch\nCOPY strop /worker/strop\nENTRYPOINT [\"/worker/strop\", \"--worker-stdio\"]\n",
        )
        .expect("Dockerfile");
        run_docker(&[
            "build",
            "--label",
            &format!("strop-test-run={tag}"),
            "-t",
            &image,
            &context.path().to_string_lossy(),
        ]);
        let id = run_docker(&[
            "run",
            "-di",
            "--label",
            &format!("strop-test-run={tag}"),
            &image,
        ]);
        Fixture {
            id,
            image: Some(image),
        }
    }

    fn identity(&self, engine_ref: &EngineRef, token: &CancelToken) -> ContainerIdentity {
        inspect(engine_ref, &self.id, token).expect("fixture inspects")
    }

    /// In-container `ps` COMMAND lines (fixture images carry busybox).
    fn processes(&self) -> String {
        run_docker(&["exec", &self.id, "ps"])
    }

    /// Bounded poll until no `worker-stdio` process remains in the
    /// container's namespace — the observable form of "the lease close
    /// reaped the worker".
    fn assert_workers_reaped(&self, within: Duration) {
        let deadline = Instant::now() + within;
        loop {
            if !self.processes().contains("worker-stdio") {
                return;
            }
            assert!(Instant::now() < deadline, "worker survived lease close");
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.id]).output();
        if let Some(image) = &self.image {
            let _ = Command::new("docker").args(["rmi", image]).output();
        }
    }
}

fn request(supply: ArtifactSupply) -> DeployRequest {
    DeployRequest {
        catalog: catalog(VERSION),
        editor_version: VERSION.to_string(),
        consent: Consent::Granted {
            action: "open-container-workspace".to_string(),
        },
        supply,
        online: true,
    }
}

fn probed(outcome: DeployOutcome) -> strop_worker_deploy::deploy::ProbedDeployment {
    match outcome {
        DeployOutcome::Probed(ready) => ready,
        other => panic!("expected a verified probe, got {other:?}"),
    }
}

/// The full WK08 flow on an authorized writable container: consent-
/// gated deploy over the scoped tar transfer, verified activation
/// handshake, then read/write/notify through the worker lease inside
/// the container's mount namespace — and lease close reaps the worker.
#[test]
fn deploy_then_read_write_notify_and_lease_reap() {
    if !gate("deploy_then_read_write_notify_and_lease_reap") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let binary = worker_binary(&directory);
    let tag = tag();
    let fixture = Fixture::busybox(&tag, &[]);
    let (cancel, _guard) = token();
    let engine_ref = engine(&cancel).expect("engine probes");
    let identity = fixture.identity(&engine_ref, &cancel);
    let provider =
        ContainerProvider::capture(&engine_ref, &identity, ShellPolicy::Required, &cancel)
            .expect("endpoint captures");
    assert_eq!(provider.endpoint().principal, "0");
    assert_eq!(provider.endpoint().target, "x86_64-unknown-linux-musl");

    let report = deploy(
        &provider,
        &request(ArtifactSupply::LocalBinary { path: binary }),
    );
    let deployed = probed(report.outcome);
    assert_eq!(deployed.origin, DeployOrigin::Uploaded);
    assert!(deployed.object.path.contains("/strop-worker/objects/"));

    // The cache object is the principal's, private and owner-executable
    // — the daemon's root extraction never manufactured a root-owned
    // world-readable cache.
    let stat = run_docker(&[
        "exec",
        &fixture.id,
        "stat",
        "-c",
        "%u %a",
        &deployed.object.path,
    ]);
    assert_eq!(stat, "0 500", "principal-owned 0500 object");
    fixture.assert_workers_reaped(Duration::from_secs(10)); // activation's handshake left nothing
    let layout = strop_worker_deploy::cache::resolve(&provider).unwrap();
    assert!(
        provider.list(&layout.leases_dir()).unwrap().is_empty(),
        "the stopped container deployment probe cannot hold a live cache lease"
    );

    // The workspace lease: read, write and notify inside the container.
    let worker = provider.worker(&deployed.object.path);
    let (cancel, _guard) = token();
    let mut payload = worker
        .read(
            &cancel,
            ResourceLocation::local(PathBuf::from("/data/hello.txt")),
            0,
            None,
        )
        .expect("read rides the container worker");
    let mut bytes = Vec::new();
    payload.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"hello strop\n");
    let active = worker
        .session()
        .expect("the live container worker handshook");
    assert!(
        provider
            .lstat(&layout.lease(active.lease.0))
            .unwrap()
            .is_some(),
        "the container worker registers its own session before serving"
    );
    let lock_path = format!("{}/{}", layout.root(), CACHE_LOCK_FILE);
    let lock = provider.lstat(&lock_path).unwrap().unwrap();
    assert_eq!(lock.kind, RemoteKind::File);
    assert_eq!(lock.owner, provider.endpoint().principal);
    assert_eq!(lock.mode & 0o077, 0, "cache lock is private");

    // Real in-container retirement: only the unleased receipt for
    // this selected endpoint is removed, not the executing object.
    let old_bytes = b"old container worker";
    let old_sha = sha256_hex(old_bytes);
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
        .collect_cache(&cancel, &provider.endpoint().context)
        .unwrap();
    assert!(maintenance.failure.is_none(), "{:?}", maintenance.failure);
    assert!(maintenance.report.removed_objects.contains(&old_sha));
    assert!(provider.lstat(&layout.object(&old_sha)).unwrap().is_none());
    assert!(provider.lstat(&layout.receipt(&old_sha)).unwrap().is_none());
    assert!(provider.lstat(&deployed.object.path).unwrap().is_some());

    let (steps, refused) = worker
        .prepare(
            &cancel,
            vec![OperationIntent {
                kind: OperationKind::CreateFile,
                source: None,
                destination: Some(ResourceLocation::local(PathBuf::from(
                    "/data/worker-created.txt",
                ))),
                copy_version: CopyVersion::Stored,
                expected_content: None,
                store: None,
            }],
            None,
        )
        .expect("prepare rides the worker");
    assert!(refused.is_empty());
    let receipts = worker.apply(&cancel, steps, None).expect("apply commits");
    assert!(
        receipts
            .iter()
            .all(|receipt| matches!(receipt.outcome, StepOutcome::Committed { .. })),
        "create committed inside the container: {receipts:?}"
    );
    let created = run_docker(&["exec", &fixture.id, "cat", "/data/worker-created.txt"]);
    assert_eq!(created, "", "the file exists in the container namespace");

    let (events, rx) = std::sync::mpsc::channel();
    worker.set_event_sink(events);
    let (subscription, coverage) = worker
        .subscribe(
            &cancel,
            ResourceLocation::local(PathBuf::from("/data")),
            false,
        )
        .expect("subscribe admitted");
    assert_eq!(coverage, strop_worker_protocol::NotifyCoverage::Native);
    // The reconcile boundary lands first; then the external write must
    // produce a hint — the worker watches the container's own mounts.
    let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        first,
        strop_worker_protocol::Event::ReconcileBoundary { .. }
    ));
    run_docker(&[
        "exec",
        &fixture.id,
        "sh",
        "-c",
        "printf 'changed\\n' >> /data/hello.txt",
    ]);
    let mut saw_hint = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !saw_hint && Instant::now() < deadline {
        if let Ok(strop_worker_protocol::Event::Notify { hints, .. }) =
            rx.recv_timeout(Duration::from_secs(1))
        {
            saw_hint = hints.iter().any(|hint| hint.path == b"hello.txt");
        }
    }
    assert!(saw_hint, "the container write produced a notify hint");
    worker.unsubscribe(&cancel, subscription).unwrap();

    drop(worker);
    fixture.assert_workers_reaped(Duration::from_secs(15));
    assert!(
        provider
            .lstat(&layout.lease(active.lease.0))
            .unwrap()
            .is_none(),
        "orderly container worker retirement releases its own cache lease"
    );
    assert!(
        provider.lstat(&lock_path).unwrap().is_some(),
        "orderly retirement must keep the shared lock inode"
    );
}

/// A restarted container is a different endpoint: the pinned provider
/// refuses stale operations, and a lease factory connect re-admits
/// against the engine and fails typed instead of retargeting.
#[test]
fn stale_incarnation_is_refused() {
    if !gate("stale_incarnation_is_refused") {
        return;
    }
    let _directory = tempfile::tempdir().unwrap();
    let tag = tag();
    let fixture = Fixture::busybox(&tag, &[]);
    let (cancel, _guard) = token();
    let engine_ref = engine(&cancel).expect("engine probes");
    let identity = fixture.identity(&engine_ref, &cancel);
    let provider =
        ContainerProvider::capture(&engine_ref, &identity, ShellPolicy::Required, &cancel)
            .expect("endpoint captures");

    run_docker(&["restart", &fixture.id]);

    let error = provider
        .lstat("/data/hello.txt")
        .expect_err("the pinned incarnation is stale after restart");
    assert!(
        error.to_string().contains("stale container identity"),
        "typed staleness: {error}"
    );
    let worker = provider.worker("/nonexistent");
    let (cancel2, _guard2) = token();
    let error = worker
        .health(&cancel2)
        .expect_err("the lease cannot connect to a stale incarnation");
    assert!(
        error.to_string().contains("stale container identity"),
        "lease admission re-checks the incarnation: {error}"
    );
}

/// The shell-less preinstalled path: FROM scratch + the static worker,
/// no sh anywhere. WK06 validates the object in place; no byte of the
/// deployment machinery (cache, staging, receipts) touches the image;
/// the lease execs the worker directly and reads through it.
#[test]
fn preinstalled_shellless_worker_needs_no_deploy() {
    if !gate("preinstalled_shellless_worker_needs_no_deploy") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let binary = worker_binary(&directory);
    let tag = tag();
    let fixture = Fixture::shellless(&tag, &binary);
    let (cancel, _guard) = token();
    let engine_ref = engine(&cancel).expect("engine probes");
    let identity = fixture.identity(&engine_ref, &cancel);
    let provider = ContainerProvider::capture(&engine_ref, &identity, ShellPolicy::Absent, &cancel)
        .expect("shellless endpoint captures");

    let report = deploy(
        &provider,
        &request(ArtifactSupply::Preinstalled {
            path: "/worker/strop".to_string(),
        }),
    );
    let deployed = probed(report.outcome);
    assert_eq!(deployed.origin, DeployOrigin::Preinstalled);
    assert_eq!(deployed.object.path, "/worker/strop");
    assert_eq!(
        deployed.object.bytes,
        std::fs::metadata(&binary).unwrap().len(),
        "hashed in place"
    );

    // No deploy writes happened: scratch has no /root at all, and the
    // cache path is absent (`docker cp` of a missing path fails).
    let output = Command::new("docker")
        .args(["cp", &format!("{}:/root/.cache", fixture.id), "-"])
        .output()
        .expect("docker cp runs");
    assert!(
        !output.status.success(),
        "no cache tree exists on the preinstalled path"
    );

    // The lease works with zero deployment: read the worker's own
    // object back through the worker, byte-identical prefix.
    let worker = provider.worker(&deployed.object.path);
    let (cancel, _guard) = token();
    let mut payload = worker
        .read(
            &cancel,
            ResourceLocation::local(PathBuf::from("/worker/strop")),
            0,
            Some(4096),
        )
        .expect("read rides the shellless worker");
    let mut prefix = Vec::new();
    payload.read_to_end(&mut prefix).unwrap();
    let mut local = vec![0u8; prefix.len()];
    std::fs::File::open(&binary)
        .unwrap()
        .read_exact(&mut local)
        .unwrap();
    assert_eq!(prefix, local, "served bytes are the image's own");
    worker.shutdown().unwrap();
}

/// The same shell-less image under the automatic-deployment path is a
/// truthful refusal naming the missing capability — never a silent
/// fallback, an install attempt, or a write-as-root workaround.
#[test]
fn shellless_deploy_is_a_typed_refusal() {
    if !gate("shellless_deploy_is_a_typed_refusal") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let binary = worker_binary(&directory);
    let tag = tag();
    let fixture = Fixture::shellless(&tag, &binary);
    let (cancel, _guard) = token();
    let engine_ref = engine(&cancel).expect("engine probes");
    let identity = fixture.identity(&engine_ref, &cancel);
    let provider =
        ContainerProvider::capture(&engine_ref, &identity, ShellPolicy::Required, &cancel)
            .expect("endpoint captures");

    let report = deploy(
        &provider,
        &request(ArtifactSupply::LocalBinary { path: binary }),
    );
    match report.outcome {
        DeployOutcome::Refused(refusal) => {
            assert!(
                refusal.to_string().contains("POSIX sh"),
                "the refusal names the missing capability: {refusal}"
            );
        }
        other => panic!("expected Refused, got {other:?}"),
    }
}

/// A read-only container filesystem classifies truthfully: the cache
/// mkdir fails read-only and deployment refuses, never chmods or
/// remounts into compliance.
#[test]
fn read_only_container_refuses_truthfully() {
    if !gate("read_only_container_refuses_truthfully") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let binary = worker_binary(&directory);
    let tag = tag();
    let fixture = Fixture::busybox_raw(&tag, &["--read-only"]);
    let (cancel, _guard) = token();
    let engine_ref = engine(&cancel).expect("engine probes");
    let identity = fixture.identity(&engine_ref, &cancel);
    let provider =
        ContainerProvider::capture(&engine_ref, &identity, ShellPolicy::Required, &cancel)
            .expect("endpoint captures");

    let report = deploy(
        &provider,
        &request(ArtifactSupply::LocalBinary { path: binary }),
    );
    match report.outcome {
        DeployOutcome::Refused(refusal) => {
            assert!(
                refusal.to_string().contains("Read-only"),
                "the refusal is read-only-classified: {refusal}"
            );
        }
        other => panic!("expected Refused, got {other:?}"),
    }
}
