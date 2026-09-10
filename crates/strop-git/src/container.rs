//! The read-oriented container Git backend (0037 DC1b): the same
//! bounded `git` queries as the remote backend, executed inside a
//! running container through the local engine's `docker exec`
//! ([`GitExec::Container`]) and parsed by the *same* wire parsers —
//! discovery and context are the exec-generic cores shared with
//! [`crate::remote`], so no parsing or exit-code mapping is duplicated
//! here. Like the remote path, no mutation verbs exist: container
//! repositories are read-only.
//!
//! The returned workdir is a path *inside* the container — never a
//! local path, and no libgit2 handle may be opened against it. The
//! container boundary carries argv as UTF-8 text and caps retained
//! output; both surface as typed refusals from the exec layer, and a
//! truncated stream can never pose as a complete record set.

use std::path::{Path, PathBuf};

use strop_core::worker::CancelToken;
use strop_workspace::ContainerId;

use crate::exec::GitExec;
use crate::remote::{context_with, discover_with, RemoteGitError};
use crate::target::RepoTarget;
use crate::GitContext;

/// Discover the repository containing an in-container directory.
/// `Ok(None)` is the honest "no repository here" — git's own
/// not-a-repository fatal — while engine failures, missing `git` and
/// corrupt repositories are typed errors. The returned workdir is a
/// path inside the container, never a local path.
pub fn discover(
    id: &ContainerId,
    from: &Path,
    cancel: &CancelToken,
) -> Result<Option<PathBuf>, RemoteGitError> {
    let exec = GitExec::Container {
        container: id.clone(),
        workdir: from,
    };
    discover_with(&exec, cancel)
}

/// The pure cached context of an in-container repository: HEAD sha,
/// branch and remotes captured once on a worker, the same [`GitContext`]
/// the local and remote paths produce — with [`RepoTarget::Container`]
/// as the repo identity. Unborn HEAD and detached HEAD are honest
/// `None`/name states, distinguished by exit code — never by stderr
/// text.
pub fn context(
    id: &ContainerId,
    workdir: &Path,
    cancel: &CancelToken,
) -> Result<GitContext, RemoteGitError> {
    let exec = GitExec::Container {
        container: id.clone(),
        workdir,
    };
    let repo = RepoTarget::Container {
        container: id.clone(),
        workdir: workdir.to_path_buf(),
    };
    context_with(&exec, repo, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::GitRun;
    use crate::remote::{context_from_runs, discover_from_run};

    // Bytes below are captured from real `git` output, mirroring the
    // remote backend's fixtures; the regressions pin the exact shapes
    // the shared parsers admit for the container path.

    fn run(code: i32, stdout: &[u8], stderr: &[u8]) -> GitRun {
        GitRun {
            success: code == 0,
            code: Some(code),
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
            stdout_dropped: 0,
            stderr_dropped: 0,
        }
    }

    fn id() -> ContainerId {
        ContainerId::canonical("a".repeat(64)).expect("64 hex is a canonical id")
    }

    /// The container backend shares the discovery mapping: 128 with
    /// git's not-a-repository fatal is the honest `None`, any other
    /// failure stays typed, and a good run parses the in-container root.
    #[test]
    fn no_repo_is_none_and_other_failures_are_typed() {
        let absent = run(
            128,
            b"",
            b"fatal: not a git repository (or any of the parent directories): .git\n",
        );
        assert_eq!(discover_from_run(&absent), Ok(None));

        let present = run(0, b"/work/app\n", b"");
        assert_eq!(
            discover_from_run(&present).unwrap(),
            Some(PathBuf::from("/work/app"))
        );

        match discover_from_run(&run(128, b"", b"fatal: unsafe repository\n")) {
            Err(RemoteGitError::Exit { op, code, stderr }) => {
                assert_eq!(op, "rev-parse --show-toplevel");
                assert_eq!(code, 128);
                assert_eq!(stderr, "fatal: unsafe repository");
            }
            other => panic!("expected Exit, got {other:?}"),
        }
    }

    /// The container context is the same GitContext the remote path
    /// produces, only the target differs: detached HEAD is an honest
    /// `None` branch, and remotes come from the shared config parser.
    #[test]
    fn context_reports_container_target_and_head_state() {
        let repo = RepoTarget::Container {
            container: id(),
            workdir: PathBuf::from("/work/app"),
        };
        let head = run(0, b"c59d8ceb7aeb96a1cdccff5646ec485acce32d45\n", b"");
        let branch = run(128, b"", b"fatal: ref HEAD is not a symbolic ref\n");
        let config = run(0, b"remote.origin.url\ngit@gh:acme/demo.git\0", b"");
        let context = context_from_runs(repo.clone(), &head, &branch, &config).unwrap();
        assert_eq!(context.repo, repo);
        assert_eq!(
            context.head_sha.as_deref(),
            Some("c59d8ceb7aeb96a1cdccff5646ec485acce32d45")
        );
        assert_eq!(context.head_branch, None, "detached HEAD");
        assert_eq!(
            context.remotes,
            vec![("origin".to_string(), "git@gh:acme/demo.git".to_string())]
        );
    }

    /// Unborn HEAD: rev-parse exits 1 (no commits) while symbolic-ref
    /// still names the branch; a repo with no remotes exits 1 too —
    /// all three are data, never errors.
    #[test]
    fn unborn_head_and_no_remotes_are_honest_data() {
        let repo = RepoTarget::Container {
            container: id(),
            workdir: PathBuf::from("/work/app"),
        };
        let head = run(1, b"", b"");
        let branch = run(0, b"main\n", b"");
        let config = run(1, b"", b"");
        let context = context_from_runs(repo, &head, &branch, &config).unwrap();
        assert_eq!(context.head_sha, None, "unborn HEAD");
        assert_eq!(context.head_branch.as_deref(), Some("main"));
        assert!(context.remotes.is_empty());
    }
}
