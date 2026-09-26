//! Worker-routed Git (0058 WK10): finite `git` commands through an
//! admitted worker lease's exec family, crossing the real codec against
//! the in-process serve loop — the same handlers the shipped worker
//! serves. Output exactness is asserted byte-for-byte against the local
//! backend on the same repository; exit classification and cancellation
//! keep their typed shapes.
#![cfg(unix)]

use std::ffi::OsString;
use std::path::Path;

use strop_git::exec::{GitExecError, WorkerGitError};
use strop_git::{GitExec, RepoTarget};
use strop_remote::worker_transport::RemoteWorker;
use strop_worker_client::{Transport, Worker};

use strop_workspace::{ContainerId, RemoteEndpoint};
/// A live token: the standalone handle cancels on drop, so tests hold it.
fn token() -> (
    strop_core::worker::CancelToken,
    strop_core::worker::CancelHandle,
) {
    strop_core::worker::CancelToken::standalone()
}

/// The loopback lease: the real in-process serve loop over pipes.
fn worker() -> Worker {
    Worker::connect_with(|| {
        let (client_read, worker_write) = std::io::pipe()?;
        let (worker_read, client_write) = std::io::pipe()?;
        std::thread::spawn(move || {
            if let Err(error) = strop_worker::serve::run(worker_read, worker_write) {
                eprintln!("test worker serve failed: {error}");
            }
        });
        Ok(Transport {
            reader: Box::new(client_read),
            writer: Box::new(client_write),
            child: None,
            stderr: None,
        })
    })
}

/// A two-commit repository: file.txt "one\n" then "one\ntwo\n".
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let signature = git2::Signature::now("strop", "strop@test").unwrap();
    let commit = |contents: &str, message: &str, parent: Option<git2::Commit>| {
        std::fs::write(dir.path().join("file.txt"), contents).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        index.write().unwrap();
        let parents: Vec<git2::Commit> = parent.into_iter().collect();
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        let oid = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent_refs,
            )
            .unwrap();
        repo.find_commit(oid).unwrap()
    };
    let first = commit("one\n", "initial", None);
    let _second = commit("one\ntwo\n", "second", Some(first));
    dir
}

fn argv(words: &[&str]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

/// log/status/diff through the worker match the local backend's bytes
/// and exit classification exactly.
#[test]
fn worker_git_matches_local_output_and_classification() {
    let dir = repository();
    let (cancel, _hold) = token();
    let lease = worker();
    let local = GitExec::Local {
        workdir: dir.path(),
    };
    let routed = GitExec::Worker {
        worker: lease.clone(),
        workdir: dir.path(),
    };

    for args in [
        argv(&["log", "--format=%H %s"]),
        argv(&["status", "--porcelain=v1", "-z"]),
        argv(&["diff", "HEAD~1", "HEAD", "--", "file.txt"]),
    ] {
        let expected = local.run(&args, &cancel).unwrap();
        let actual = routed.run(&args, &cancel).unwrap();
        assert_eq!(actual.success, expected.success, "classification: {args:?}");
        assert_eq!(actual.code, expected.code, "exit code: {args:?}");
        assert_eq!(actual.stdout, expected.stdout, "stdout bytes: {args:?}");
        assert_eq!(actual.stderr, expected.stderr, "stderr bytes: {args:?}");
        assert_eq!(actual.stdout_dropped, 0);
        assert_eq!(actual.stderr_dropped, 0);
    }

    // `diff --quiet` on a tree with an unstaged edit exits 1 — data, on
    // both backends.
    std::fs::write(dir.path().join("file.txt"), "one\ntwo\nthree\n").unwrap();
    let quiet = argv(&["diff", "--quiet"]);
    let expected = local.run(&quiet, &cancel).unwrap();
    let actual = routed.run(&quiet, &cancel).unwrap();
    assert!(!expected.success && expected.code == Some(1));
    assert!(!actual.success && actual.code == Some(1));
    lease.shutdown().unwrap();
}

/// Discovery and context through the lease: the remote-module public
/// entry points ride the worker backend when a lease is handed in,
/// without touching the endpoint one-shot path.
#[test]
fn worker_git_discovery_and_context() {
    let dir = repository();
    let (cancel, _hold) = token();
    let lease = worker();
    let endpoint = RemoteEndpoint::parse("ssh://box.example").unwrap();
    let admitted = RemoteWorker::for_test(&endpoint, lease.clone());
    let workdir = dir.path().canonicalize().unwrap();
    let found = strop_git::remote::discover(&endpoint, &workdir, Some(&admitted), &cancel).unwrap();
    assert_eq!(found.as_deref(), Some(workdir.as_path()));
    let context =
        strop_git::remote::context(&endpoint, &workdir, Some(&admitted), &cancel).unwrap();
    assert!(context.head_sha.is_some(), "two commits have a HEAD sha");
    lease.shutdown().unwrap();
}

/// A cancelled token is a typed error before any spawn — never a run.
#[test]
fn worker_git_cancellation_is_typed() {
    let dir = repository();
    let lease = worker();
    let exec = GitExec::Worker {
        worker: lease.clone(),
        workdir: dir.path(),
    };
    let (cancel, hold) = token();
    drop(hold); // cancels the token
    let error = exec.run(&argv(&["status"]), &cancel).unwrap_err();
    assert!(
        matches!(error, GitExecError::Worker(WorkerGitError::Client(_))),
        "typed cancellation, not silence: {error}"
    );
    lease.shutdown().unwrap();
}

/// A non-local repository without an admitted worker is unavailable:
/// never run Git locally, through Python over SSH, or through an
/// unadmitted container shell supervisor.
#[test]
fn non_local_git_refuses_missing_worker_authority() {
    let remote = RepoTarget::Remote {
        endpoint: RemoteEndpoint::parse("ssh://box.example").unwrap(),
        workdir: std::path::PathBuf::from("/repo"),
    };
    let container = RepoTarget::Container {
        container: ContainerId::canonical("d".repeat(64)).unwrap(),
        workdir: std::path::PathBuf::from("/repo"),
    };
    for target in [&remote, &container] {
        assert!(matches!(
            GitExec::for_target_routed(target, None),
            Err(GitExecError::Unavailable { .. })
        ));
    }
}

/// `ssh -G` runs inside the bound worker, not as `git -G` or against
/// the editor's OpenSSH configuration. A foreign lease refuses before
/// any child starts.
#[test]
fn remote_effective_host_uses_worker_ssh_and_checks_endpoint() {
    let directory = tempfile::tempdir().unwrap();
    let endpoint = RemoteEndpoint::parse("ssh://box.example").unwrap();
    let lease = worker();
    let bound = RemoteWorker::for_test(&endpoint, lease.clone());
    let alias =
        match strop_git::permalink::parse_remote("ssh://example.invalid/owner/repository.git")
            .unwrap()
        {
            strop_git::permalink::SelectedRemote::Alias(alias) => alias,
            other => panic!("expected an SSH alias, got {other:?}"),
        };
    let (cancel, _owner) = token();
    let host = strop_git::remote::effective_host(
        &endpoint,
        directory.path(),
        &alias,
        Some(&bound),
        &cancel,
    )
    .unwrap();
    assert_eq!(host, "example.invalid");
    let wrong = RemoteEndpoint::parse("ssh://other.example").unwrap();
    assert!(matches!(
        strop_git::remote::effective_host(&wrong, directory.path(), &alias, Some(&bound), &cancel),
        Err(strop_git::ssh::EffectiveHostError::Unavailable(_))
    ));
    lease.shutdown().unwrap();
}
