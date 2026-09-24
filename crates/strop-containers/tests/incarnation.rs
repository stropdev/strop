//! VF10 container incarnation/context assurance campaigns (plans/0057 §5
//! "Running containers"), extending the AR07 fixtures in `docker.rs`.
//! Required-mode gated exactly like that file: STROP_CONTAINER_TESTS=1 with an
//! accessible engine, never a silent skip. Fixtures are uniquely labelled and
//! self-cleaning; the context fixture restores the CLI's default context on
//! drop, including on panic.
#![cfg(unix)]

use std::ffi::OsStr;
use std::io::Read;
use std::process::Command;
use strop_containers::{
    engine, inspect, read_file, revalidate, ContainerError, ContainerRef, ExecRecord, ExecSpec,
};
use strop_core::worker::CancelToken;
use strop_workspace::ContainerId;

/// The DeadContext fixture flips the CLI's ambient default context; every
/// test in this binary holds this lock so no concurrent probe or fixture
/// command can observe the flipped selection.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

fn with_token<T>(work: impl FnOnce(CancelToken) -> T) -> T {
    let (tokens, receiver) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel::<()>();
    let owner = strop_core::worker::spawn(
        "container-vf10",
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

/// The labelled disposable fixture container; `Drop` removes exactly this
/// container by id — on panic too, and never anything unlabelled.
struct Fixture {
    id: String,
}

impl Fixture {
    fn launch(tag: &str, name: &str) -> Fixture {
        let id = run_docker(&[
            "run",
            "-d",
            "--label",
            &format!("strop-test-run={tag}"),
            "--name",
            name,
            "busybox",
            "sleep",
            "300",
        ]);
        run_docker(&["exec", &id, "mkdir", "-p", "/data"]);
        run_docker(&["exec", &id, "sh", "-c", "printf 'fixture\\n' > /data/file"]);
        Fixture { id }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.id]).output();
    }
}

fn fixture_ref(
    engine: &strop_containers::EngineRef,
    name: &str,
    token: &CancelToken,
) -> (strop_containers::ContainerIdentity, ContainerRef) {
    let identity = inspect(engine, name, token).expect("fixture inspects");
    let reference = ContainerRef::of(&identity).expect("fixture id is canonical");
    (identity, reference)
}

/// A uniquely-named dead docker context, with the CLI's selected context
/// flipped to it for the test's duration; `Drop` restores the original
/// selection and removes the dead context, even on panic.
struct DeadContext {
    name: String,
    original: String,
}

impl DeadContext {
    fn select(tag: &str) -> DeadContext {
        let original = run_docker(&["context", "show"]);
        let name = format!("strop-vf10-dead-{tag}");
        run_docker(&[
            "context",
            "create",
            &name,
            "--docker",
            "host=tcp://127.0.0.1:1",
        ]);
        run_docker(&["context", "use", &name]);
        DeadContext { name, original }
    }
}

impl Drop for DeadContext {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["context", "use", &self.original])
            .output();
        let _ = Command::new("docker")
            .args(["context", "rm", "-f", &self.name])
            .output();
    }
}

#[test]
fn a_changed_default_context_cannot_retarget_a_pinned_engine() {
    if !gate("a_changed_default_context_cannot_retarget_a_pinned_engine") {
        return;
    }
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tag = tag();
    let name = format!("strop-vf10-context-{tag}");
    let fixture = Fixture::launch(&tag, &name);
    with_token(|token| {
        // The probe pins the selected connection before the flip.
        let pinned = engine(&token).expect("engine probes");
        let (_identity, reference) = fixture_ref(&pinned, &name, &token);

        let _dead = DeadContext::select(&tag);

        // Held work stays on the probed connection: inspect, read and exec
        // admission all ignore the CLI's new default context.
        inspect(&pinned, &name, &token).expect("the pinned engine is unaffected");
        let bytes = read_file(&pinned, &reference, "/data/file", 64, &token)
            .expect("reads stay on the pinned connection");
        assert_eq!(bytes, b"fixture\n");
        let id = ContainerId::canonical(fixture.id.clone()).expect("canonical id");
        ExecSpec::resolve(&pinned, &id, "true", &[], std::path::Path::new("/"), &token)
            .expect("exec admission stays on the pinned connection");

        // A fresh probe sees the dead selection as a typed engine failure,
        // never a silent fallback to another daemon.
        assert!(
            matches!(
                engine(&token),
                Err(ContainerError::EngineUnavailable { .. })
            ),
            "a dead selected context is a typed EngineUnavailable"
        );
    });
}

#[test]
fn a_stopped_container_is_a_typed_refusal_not_stale_bytes() {
    if !gate("a_stopped_container_is_a_typed_refusal_not_stale_bytes") {
        return;
    }
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tag = tag();
    let name = format!("strop-vf10-stopped-{tag}");
    let fixture = Fixture::launch(&tag, &name);
    with_token(|token| {
        let selected = engine(&token).expect("engine probes");
        let (identity, reference) = fixture_ref(&selected, &name, &token);
        run_docker(&["stop", &fixture.id]);
        assert!(
            matches!(
                read_file(&selected, &reference, "/data/file", 64, &token),
                Err(ContainerError::NotRunning { .. })
            ),
            "reads against the stopped incarnation are typed"
        );
        assert!(
            matches!(
                ExecSpec::resolve(
                    &selected,
                    reference.id(),
                    "true",
                    &[],
                    std::path::Path::new("/"),
                    &token,
                ),
                Err(ContainerError::NotRunning { .. })
            ),
            "exec admission against the stopped container is typed"
        );
        // The same incarnation identity still revalidates (same id and
        // StartedAt): "stopped" is NotRunning, never misreported as stale.
        revalidate(&selected, &identity, &token).expect("identity still resolves");
    });
}

#[test]
fn exec_user_and_workdir_bind_the_selected_namespace() {
    if !gate("exec_user_and_workdir_bind_the_selected_namespace") {
        return;
    }
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tag = tag();
    let name = format!("strop-vf10-user-{tag}");
    let _fixture = Fixture::launch(&tag, &name);
    with_token(|token| {
        let selected = engine(&token).expect("engine probes");
        let (_identity, reference) = fixture_ref(&selected, &name, &token);
        let spec = ExecSpec::new(
            &selected,
            &reference,
            "sh",
            &["-c".into(), "id -u; pwd".into()],
            std::path::Path::new("/tmp"),
        )
        .expect("spec validates")
        .with_user("65534")
        .expect("numeric principal validates");
        let admitted = spec.admit(&token).expect("live incarnation admits");
        let output = admitted
            .capture(16 * 1024, &token)
            .expect("bounded capture runs");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "65534\n/tmp\n",
            "the frozen principal and working directory are the exec's"
        );
        // Option-shaped principals are refused before the engine is asked.
        assert!(matches!(
            ExecSpec::new(
                &selected,
                &reference,
                "true",
                &[],
                std::path::Path::new("/"),
            )
            .expect("spec validates")
            .with_user("--privileged"),
            Err(ContainerError::CapabilityRefused { .. })
        ));
    });
}

#[test]
fn the_launched_record_pid_is_namespace_local() {
    if !gate("the_launched_record_pid_is_namespace_local") {
        return;
    }
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let tag = tag();
    let name = format!("strop-vf10-pid-{tag}");
    let fixture = Fixture::launch(&tag, &name);
    with_token(|token| {
        let selected = engine(&token).expect("engine probes");
        let id = ContainerId::canonical(fixture.id.clone()).expect("canonical id");
        // The worker reports its own PID from inside the namespace; the
        // supervisor's launched record must agree — it is the same
        // namespace's identity, never a host PID from `docker top`.
        let admitted = ExecSpec::resolve(
            &selected,
            &id,
            "sh",
            &["-c".into(), "echo $$".into()],
            std::path::Path::new("/"),
            &token,
        )
        .expect("resolve and admit");
        let key = admitted.key().clone();
        let output = admitted
            .capture(16 * 1024, &token)
            .expect("bounded capture runs");
        let echoed: u32 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("the worker echoed its namespace-local pid");
        let records = key.records(&output.stderr);
        let launched = records.iter().find_map(|record| match record {
            ExecRecord::Launched { pid } => Some(*pid),
            _ => None,
        });
        assert_eq!(
            launched,
            Some(echoed),
            "the launched record carries the container-local pid: {records:?}"
        );
        // The fixture's init is PID 1 inside the namespace; the engine's
        // host-side view (`docker top`) is a different, larger number.
        let init = run_docker(&[
            "exec",
            &fixture.id,
            "sh",
            "-c",
            "tr '\\0' ' ' < /proc/1/cmdline",
        ]);
        assert!(init.starts_with("sleep"), "in-container PID 1: {init}");
        let host: u32 = run_docker(&["top", &fixture.id, "-eo", "pid"])
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .next()
            .expect("docker top reports a host pid");
        assert_ne!(host, 1, "the host pid namespace is not the adapter's");
    });
}
