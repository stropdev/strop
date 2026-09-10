//! Real-engine integration test for the container Git backend (0037
//! DC1b). Gated exactly like strop-containers' docker gate: runs only
//! when `STROP_CONTAINER_TESTS=1` *and* `docker info` succeeds, and
//! skips loudly otherwise — a docker-less host stays green.
//!
//! The fixture is one uniquely-labelled disposable busybox container
//! (`strop-test-run=<tag>`), removed by id in cleanup including on
//! failure (the guard is `Drop`). Nothing unlabelled is ever touched.
//! busybox carries no `git`, so no real Git execution is attempted —
//! the test asserts the missing-executable refusal surfaces as a typed
//! failure promptly, never a hang.
#![cfg(unix)]

use std::ffi::OsStr;
use std::io::Read;
use std::process::Command;

use strop_core::worker::CancelToken;
use strop_git::{GitExec, GitExecError};
use strop_workspace::ContainerId;

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
    if !engine_available() {
        eprintln!("skipping {test}: STROP_CONTAINER_TESTS=1 but the docker engine is unreachable");
        return false;
    }
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

/// `git` does not exist in busybox: the run must surface the missing
/// executable promptly and typed — as an engine-boundary error or as a
/// failed run whose code is data — never a hang and never a fake
/// success.
#[test]
fn missing_git_in_container_fails_typed_never_hangs() {
    if !gate("missing_git_in_container_fails_typed_never_hangs") {
        return;
    }
    let tag = tag();
    let name = format!("strop-dc1b-git-{tag}");
    let fixture = Fixture::launch(&tag, &name);
    let id = ContainerId::canonical(fixture.id.clone()).expect("run printed the canonical id");
    let workdir = std::path::PathBuf::from("/");
    let exec = GitExec::Container {
        container: id,
        workdir: &workdir,
    };
    let argv: Vec<std::ffi::OsString> = vec!["status".into()];
    match with_token(|token| exec.run(&argv, &token)) {
        Err(GitExecError::Container(error)) => {
            eprintln!("missing git surfaced as a boundary error: {error}");
        }
        Ok(run) => {
            assert!(!run.success, "busybox has no git: {run:?}");
            assert_ne!(run.code, Some(0));
        }
        Err(other) => panic!("unexpected backend error: {other}"),
    }
}

/// The container discovery path surfaces busybox's missing `git` as a
/// typed refusal — an engine-boundary error or git's own nonzero exit
/// — promptly: never a hang, and never `Ok(None)` off a transport lie
/// (`None` is reserved for git's own not-a-repository fatal).
#[test]
fn discover_without_git_fails_typed_never_hangs() {
    if !gate("discover_without_git_fails_typed_never_hangs") {
        return;
    }
    let tag = tag();
    let name = format!("strop-dc1b-discover-{tag}");
    let fixture = Fixture::launch(&tag, &name);
    let id = ContainerId::canonical(fixture.id.clone()).expect("run printed the canonical id");
    let from = std::path::PathBuf::from("/");
    match with_token(|token| strop_git::container::discover(&id, &from, &token)) {
        Err(error) => {
            eprintln!("missing git surfaced as a typed refusal: {error}");
        }
        Ok(found) => panic!("busybox has no git: {found:?}"),
    }
}
