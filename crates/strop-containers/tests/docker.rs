//! Real-engine integration tests (0037 DC1a). Ordinary unit runs opt out;
//! STROP_CONTAINER_TESTS=1 requires an accessible Docker engine and never skips.
//!
//! The fixture launches one uniquely-labelled disposable busybox container
//! per test (`strop-test-run=<tag>`) and removes exactly that container in
//! cleanup, including on failure (the guard is `Drop`). Nothing unlabelled
//! is ever touched.
#![cfg(unix)]

use std::ffi::OsStr;
use std::io::Read;
use std::process::Command;
use strop_containers::{
    engine, inspect, list_dir, list_running, read_file, revalidate, ContainerError, ContainerRef,
    DirEntryKind,
};
use strop_core::worker::CancelToken;

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

/// Hand one closure a real worker-issued cancellation token (CancelToken
/// cannot be constructed outside strop-core's worker machinery).
fn with_token<T>(work: impl FnOnce(CancelToken) -> T) -> T {
    let (tokens, receiver) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel::<()>();
    let owner = strop_core::worker::spawn(
        "container-test",
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

/// The labelled disposable fixture container. `Drop` removes exactly this
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
        let fixture = Fixture { id };
        run_docker(&[
            "exec",
            &fixture.id,
            "sh",
            "-c",
            "mkdir -p /data/sub /data/empty \
             && printf 'hello strop\\n' > /data/hello.txt \
             && head -c 100000 /dev/zero > /data/big.bin \
             && printf 'inner' > /data/sub/inner.bin \
             && ln -s hello.txt /data/link",
        ]);
        fixture
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

#[test]
fn inspect_resolves_names_prefixes_and_ids_to_canonical_identities() {
    if !gate("inspect_resolves_names_prefixes_and_ids_to_canonical_identities") {
        return;
    }
    let tag = tag();
    let name = format!("strop-dc1a-inspect-{tag}");
    let _fixture = Fixture::launch(&tag, &name);
    with_token(|token| {
        let engine = engine(&token).expect("engine probes");
        assert!(!engine.server_version().is_empty());

        let identity = inspect(&engine, &name, &token).expect("inspect by name");
        assert_eq!(identity.id.len(), 64);
        assert_eq!(identity.name, name);
        assert!(identity.image.contains("busybox"));
        assert!(!identity.started_at.is_empty());

        assert_eq!(
            inspect(&engine, &identity.id, &token).expect("inspect by id"),
            identity
        );
        assert_eq!(
            inspect(&engine, &identity.id[..12], &token).expect("inspect by prefix"),
            identity
        );

        let running = list_running(&engine, &token).expect("list running");
        assert!(
            running.iter().any(|container| container.id == identity.id),
            "the fixture is among the running containers"
        );

        let reference = ContainerRef::of(&identity).expect("canonical reference");
        assert_eq!(reference.id().as_str(), identity.id);
        assert_eq!(reference.started_at(), identity.started_at);
        revalidate(&engine, &identity, &token).expect("a live identity revalidates");
    });
}

#[test]
fn listings_and_reads_roundtrip_through_the_engine() {
    if !gate("listings_and_reads_roundtrip_through_the_engine") {
        return;
    }
    let tag = tag();
    let name = format!("strop-dc1a-read-{tag}");
    let _fixture = Fixture::launch(&tag, &name);
    with_token(|token| {
        let engine = engine(&token).expect("engine probes");
        let (_identity, id) = fixture_ref(&engine, &name, &token);

        let mut entries = list_dir(&engine, &id, "/data", &token).expect("list /data");
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let spelling: Vec<(&str, DirEntryKind, Option<u64>)> = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind, entry.size))
            .collect();
        assert_eq!(
            spelling,
            vec![
                ("big.bin", DirEntryKind::File, Some(100_000)),
                ("empty", DirEntryKind::Dir, None),
                ("hello.txt", DirEntryKind::File, Some(12)),
                ("link", DirEntryKind::Symlink, None),
                ("sub", DirEntryKind::Dir, None),
            ],
            "grandchildren (sub/inner.bin) are not listed; sizes on files only"
        );

        let root = list_dir(&engine, &id, "/", &token).expect("list /");
        assert!(
            root.iter()
                .any(|entry| entry.name == "data" && entry.kind == DirEntryKind::Dir),
            "/ contains the fixture tree"
        );
        assert!(list_dir(&engine, &id, "/data/empty", &token)
            .expect("list empty dir")
            .is_empty());

        assert_eq!(
            read_file(&engine, &id, "/data/hello.txt", 4096, &token).expect("read file"),
            b"hello strop\n"
        );
        assert_eq!(
            read_file(&engine, &id, "/data/link", 4096, &token).expect("read via symlink"),
            b"hello strop\n"
        );
        assert_eq!(
            read_file(&engine, &id, "/data/big.bin", 1000, &token)
                .expect("bounded read")
                .len(),
            1000,
            "head -c semantics: max bounds the bytes, not the file"
        );
    });
}

#[test]
fn listing_streams_subtree_bulk_without_retaining_it() {
    if !gate("listing_streams_subtree_bulk_without_retaining_it") {
        return;
    }
    let tag = tag();
    let name = format!("strop-dc1a-stream-{tag}");
    let fixture = Fixture::launch(&tag, &name);
    // A direct-child directory whose subtree far exceeds the old 16 MiB
    // archive bound: 48 MiB across nested files. The listing must
    // succeed and report only the direct children.
    run_docker(&[
        "exec",
        &fixture.id,
        "sh",
        "-c",
        "mkdir -p /data/deep/a /data/deep/b \
         && for d in a b; do for i in 1 2 3 4 5 6; do \
         dd if=/dev/zero of=/data/deep/$d/f$i bs=1M count=4 2>/dev/null; \
         done; done",
    ]);
    with_token(|token| {
        let engine = engine(&token).expect("engine probes");
        let (_identity, id) = fixture_ref(&engine, &name, &token);

        let mut entries =
            list_dir(&engine, &id, "/data", &token).expect("list /data past the old archive bound");
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let spelling: Vec<(&str, DirEntryKind)> = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind))
            .collect();
        assert_eq!(
            spelling,
            vec![
                ("big.bin", DirEntryKind::File),
                ("deep", DirEntryKind::Dir),
                ("empty", DirEntryKind::Dir),
                ("hello.txt", DirEntryKind::File),
                ("link", DirEntryKind::Symlink),
                ("sub", DirEntryKind::Dir),
            ],
            "the 48 MiB subtree lists as one direct child, bulk unretained"
        );

        let deep = list_dir(&engine, &id, "/data/deep", &token).expect("list the deep dir itself");
        assert_eq!(deep.len(), 2, "deep's own children still list");
    });
}

#[test]
fn missing_paths_and_capability_mismatches_are_typed() {
    if !gate("missing_paths_and_capability_mismatches_are_typed") {
        return;
    }
    let tag = tag();
    let name = format!("strop-dc1a-typed-{tag}");
    let _fixture = Fixture::launch(&tag, &name);
    with_token(|token| {
        let engine = engine(&token).expect("engine probes");
        let (_identity, id) = fixture_ref(&engine, &name, &token);

        assert!(matches!(
            read_file(&engine, &id, "/data/missing", 100, &token),
            Err(ContainerError::NoSuchPath { .. })
        ));
        assert!(matches!(
            list_dir(&engine, &id, "/no/such/dir", &token),
            Err(ContainerError::NoSuchPath { .. })
        ));
        assert!(
            matches!(
                read_file(&engine, &id, "/data", 100, &token),
                Err(ContainerError::CapabilityRefused { .. })
            ),
            "a directory is not a file"
        );
        assert!(
            matches!(
                list_dir(&engine, &id, "/data/hello.txt", &token),
                Err(ContainerError::CapabilityRefused { .. })
            ),
            "a file is not a directory"
        );
        assert!(matches!(
            inspect(&engine, &format!("strop-no-such-{tag}"), &token),
            Err(ContainerError::NoSuchContainer { .. })
        ));
        for bad in ["-rf", "--format=json", "a b", "a/b"] {
            assert!(
                matches!(
                    inspect(&engine, bad, &token),
                    Err(ContainerError::PoisonedName { .. })
                ),
                "{bad:?} is refused before the engine sees it"
            );
        }
    });
}

#[test]
fn restart_and_recreation_are_stale_identity_refusals() {
    if !gate("restart_and_recreation_are_stale_identity_refusals") {
        return;
    }
    let tag = tag();
    let name = format!("strop-dc1a-stale-{tag}");
    with_token(|token| {
        let engine = engine(&token).expect("engine probes");
        let fixture = Fixture::launch(&tag, &name);
        let (identity, reference) = fixture_ref(&engine, &name, &token);

        // Restart: same id, new StartedAt incarnation.
        run_docker(&["restart", &identity.id]);
        assert!(
            matches!(
                read_file(&engine, &reference, "/data/hello.txt", 16, &token),
                Err(ContainerError::StaleIdentity { .. })
            ),
            "a restart between inspect and read is detected"
        );
        assert!(
            matches!(
                revalidate(&engine, &identity, &token),
                Err(ContainerError::StaleIdentity { .. })
            ),
            "the same name with a new incarnation no longer validates"
        );

        // Recreation under the same name: a different container entirely.
        let first_id = identity.id.clone();
        drop(fixture);
        let fixture2 = Fixture::launch(&tag, &name);
        assert!(
            matches!(
                revalidate(&engine, &identity, &token),
                Err(ContainerError::StaleIdentity { .. })
            ),
            "a name that resolves to a different id is refused"
        );
        assert!(
            matches!(
                read_file(&engine, &reference, "/data/hello.txt", 16, &token),
                Err(ContainerError::NoSuchContainer { .. })
            ),
            "the held reference's id is simply gone"
        );
        assert_ne!(
            inspect(&engine, &name, &token).expect("new inspect").id,
            first_id
        );
        drop(fixture2);
    });
}
