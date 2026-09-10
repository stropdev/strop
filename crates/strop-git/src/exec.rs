//! Three real Git execution backends (0036 RW7/RW8, 0037 DC1b): local
//! `git` via std::process with `-C workdir`, bounded remote `git` via
//! the shared remote-execution boundary ([`strop_remote::run`]), and
//! bounded in-container `git` via `docker exec` argv through
//! [`strop_containers::exec_capture`]. Not a provider framework — the
//! enum's variants are every backend that exists.
//!
//! Exit codes are data on both sides (`diff --quiet` exits 1): [`GitRun`]
//! reports success and the raw code; callers that need "did git itself
//! refuse" match on the code, not on stderr text. Remote output bounds
//! are surfaced as `dropped` counts — a parser that needs a complete
//! record stream must refuse truncated output, never parse a prefix as
//! if it were the whole answer.

use std::ffi::OsString;
use std::path::Path;

use strop_containers::ContainerError;
use strop_core::worker::CancelToken;
use strop_remote::{RemoteCommand, RemoteCommandError};
use strop_workspace::{ContainerId, RemoteEndpoint};

use crate::target::RepoTarget;

/// One bounded `git` run's native bytes.
#[derive(Debug, Clone)]
pub struct GitRun {
    /// `true` when the process exited zero.
    pub success: bool,
    /// The raw exit code where one exists (signals carry `None`).
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Bytes the remote supervisor dropped past its output bound. A
    /// nonzero count means `stdout`/`stderr` are heads (plus a stderr
    /// tail), not the complete stream.
    pub stdout_dropped: u64,
    pub stderr_dropped: u64,
}

impl GitRun {
    /// Fail typed when the caller needs the complete stdout record
    /// stream: a truncated head must never parse as "everything git
    /// said" (0036: no swallowed output errors).
    pub fn require_full_stdout(&self, op: &str) -> Result<&[u8], String> {
        if self.stdout_dropped > 0 {
            return Err(format!(
                "{op}: remote output truncated ({} bytes dropped)",
                self.stdout_dropped
            ));
        }
        Ok(&self.stdout)
    }
}

/// Why a bounded `git` run could not produce its bytes.
#[derive(Debug)]
pub enum GitExecError {
    /// The local `git` process could not be started.
    Spawn(String),
    /// The remote execution boundary refused or failed; the display
    /// keeps the boundary's own typed diagnosis (transport, supervisor,
    /// missing tooling, cancellation, timeout…).
    Remote(RemoteCommandError),
    /// The container engine boundary refused or failed (engine
    /// unavailable, container not running, output bound exceeded,
    /// cancellation…), or the workdir/argv could not be carried as
    /// UTF-8 `docker exec` argv — a typed refusal, never a lossy guess.
    Container(ContainerError),
}

impl std::fmt::Display for GitExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(message) => write!(f, "{message}"),
            Self::Remote(error) => write!(f, "{error}"),
            Self::Container(error) => write!(f, "{error}"),
        }
    }
}

/// Retained stdout for one in-container `git` run — the same scale the
/// remote backend's supervisor keeps (16 MiB); a parser that needs the
/// complete stream sees the dropped count and must refuse a head.
const CONTAINER_STDOUT_LIMIT: u64 = 16 * 1024 * 1024;

/// One [`OsString`] as `docker exec` argv text. The container boundary
/// carries argv as `String`s, so a non-UTF-8 argument (a native-byte
/// filename) is refused typed rather than lossily renamed — the remote
/// backend can carry those bytes; this one cannot.
fn argv_text(arg: &OsString) -> Result<String, GitExecError> {
    arg.clone().into_string().map_err(|arg| {
        GitExecError::Container(ContainerError::CapabilityRefused {
            what: format!(
                "container git: argument is not UTF-8 ({:?})",
                arg.to_string_lossy()
            ),
        })
    })
}

/// Where one Git query executes. Constructed from a [`RepoTarget`] so a
/// non-local workdir can only ever reach its matching backend.
#[derive(Debug, Clone)]
pub enum GitExec<'a> {
    Local {
        workdir: &'a Path,
    },
    Remote {
        endpoint: RemoteEndpoint,
        workdir: &'a Path,
    },
    Container {
        container: ContainerId,
        workdir: &'a Path,
    },
}

impl<'a> GitExec<'a> {
    /// The backend for a repository target: the local worktree runs
    /// `git -C <workdir>` in-process; the remote worktree rides one
    /// bounded supervised connection per run; the container worktree
    /// rides one bounded `docker exec` per run. No client state is held
    /// between runs.
    pub fn for_target(target: &'a RepoTarget) -> Self {
        match target {
            RepoTarget::Local { workdir } => Self::Local { workdir },
            RepoTarget::Remote { endpoint, workdir } => Self::Remote {
                endpoint: endpoint.clone(),
                workdir,
            },
            RepoTarget::Container { container, workdir } => Self::Container {
                container: container.clone(),
                workdir,
            },
        }
    }

    /// Run one bounded `git <argv>` in the repository. `argv` excludes
    /// the workdir placement — the local and container variants prefix
    /// `-C`, the remote variant sets the supervised cwd — so every argv
    /// element stays one inert argument on every backend, including
    /// filenames with spaces or newlines. (The container boundary
    /// carries argv as UTF-8 text, so a native-byte filename is refused
    /// typed there rather than lossily renamed.)
    pub fn run(&self, argv: &[OsString], cancel: &CancelToken) -> Result<GitRun, GitExecError> {
        match self {
            Self::Local { workdir } => {
                let output = std::process::Command::new("git")
                    .arg("-C")
                    .arg(workdir)
                    .args(argv)
                    .output()
                    .map_err(|error| {
                        GitExecError::Spawn(format!(
                            "spawn git {}: {error}",
                            argv.first()
                                .map(|a| a.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        ))
                    })?;
                Ok(GitRun {
                    success: output.status.success(),
                    code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                    stdout_dropped: 0,
                    stderr_dropped: 0,
                })
            }
            Self::Remote { endpoint, workdir } => {
                let command = RemoteCommand::new("git", argv.to_vec(), workdir)
                    .map_err(GitExecError::Remote)?;
                let output =
                    strop_remote::run(endpoint, &command, cancel).map_err(GitExecError::Remote)?;
                Ok(GitRun {
                    success: output.status.success(),
                    code: output
                        .status
                        .code()
                        .and_then(|code| i32::try_from(code).ok()),
                    stdout: output.stdout,
                    stderr: output.stderr,
                    stdout_dropped: output.stdout_dropped,
                    stderr_dropped: output.stderr_dropped,
                })
            }
            Self::Container { container, workdir } => {
                // `git -C <workdir>` keeps the argv shape identical to
                // the local backend; `--workdir` additionally places
                // the exec there. Both need the workdir as UTF-8 text —
                // a native-byte path is refused, never guessed lossy.
                let Some(workdir_text) = workdir.to_str() else {
                    return Err(GitExecError::Container(ContainerError::CapabilityRefused {
                        what: "container git: the working directory is not UTF-8".into(),
                    }));
                };
                let mut args = Vec::with_capacity(argv.len() + 2);
                args.push("-C".to_string());
                args.push(workdir_text.to_string());
                for arg in argv {
                    args.push(argv_text(arg)?);
                }
                let engine = strop_containers::engine(cancel).map_err(GitExecError::Container)?;
                let output = strop_containers::exec_capture(
                    &engine,
                    container,
                    "git",
                    &args,
                    workdir,
                    CONTAINER_STDOUT_LIMIT,
                    cancel,
                )
                .map_err(GitExecError::Container)?;
                Ok(GitRun {
                    success: output.code == Some(0),
                    code: output.code,
                    stdout: output.stdout,
                    stderr: output.stderr,
                    stdout_dropped: output.stdout_dropped,
                    // The container capture surface retains a bounded
                    // stderr without reporting a dropped count.
                    stderr_dropped: 0,
                })
            }
        }
    }

    /// Run one bounded `git` invocation that must exit zero with a
    /// complete stdout record stream, returning those bytes. Nonzero
    /// exits and truncated output are typed errors carrying git's own
    /// stderr — the shared shape every memory query wants.
    pub fn run_records(
        &self,
        op: &str,
        argv: &[OsString],
        cancel: &CancelToken,
    ) -> Result<Vec<u8>, String> {
        let run = self
            .run(argv, cancel)
            .map_err(|error| format!("{op}: {error}"))?;
        if !run.success {
            return Err(format!(
                "{op}: {}",
                String::from_utf8_lossy(&run.stderr).trim()
            ));
        }
        if run.stderr_dropped > 0 {
            return Err(format!(
                "{op}: remote stderr truncated ({} bytes dropped)",
                run.stderr_dropped
            ));
        }
        run.require_full_stdout(op).map(|bytes| bytes.to_vec())
    }
}

/// Test seam: hand one closure a real worker-issued cancellation
/// token (CancelToken cannot be constructed outside strop-core's
/// worker machinery) and return its value.
#[cfg(test)]
pub(crate) fn with_token<T>(work: impl FnOnce(CancelToken) -> T) -> T {
    let (tokens, receiver) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel::<()>();
    let owner = strop_core::worker::spawn(
        "git-exec-test",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn native_path_arguments_select_the_exact_index_entry() {
        use std::os::unix::ffi::OsStrExt;
        let directory = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(directory.path()).unwrap();
        let name = std::ffi::OsStr::from_bytes(b"- odd \xff.txt");
        let path = std::path::Path::new(name);
        std::fs::write(directory.path().join(path), "content\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(path).unwrap();
        index.write().unwrap();
        let exec = GitExec::Local {
            workdir: directory.path(),
        };
        let argv = ["ls-files".into(), "-z".into(), "--".into(), name.into()];
        let run = with_token(|token| exec.run(&argv, &token)).expect("git runs");
        assert!(run.success);
        assert_eq!(run.stdout, b"- odd \xff.txt\0");
    }

    /// Nonzero exits are data, not errors: the run succeeds as a run,
    /// and the caller reads the code.
    #[test]
    fn local_exit_codes_are_data() {
        let directory = tempfile::tempdir().unwrap();
        let _repo = git2::Repository::init(directory.path()).unwrap();
        let exec = GitExec::Local {
            workdir: directory.path(),
        };
        let argv: Vec<OsString> = vec![
            "rev-parse".into(),
            "--verify".into(),
            "--quiet".into(),
            "no-such-ref".into(),
        ];
        let run = with_token(|token| exec.run(&argv, &token)).expect("git runs");
        assert!(!run.success);
        assert_eq!(run.code, Some(1));
    }

    /// run_records refuses a nonzero exit with git's own stderr — an
    /// honest message, never a silent empty record set.
    #[test]
    fn run_records_reports_nonzero_exits() {
        let directory = tempfile::tempdir().unwrap();
        let _repo = git2::Repository::init(directory.path()).unwrap();
        let exec = GitExec::Local {
            workdir: directory.path(),
        };
        let argv: Vec<OsString> = vec!["log".into(), "--format=".into(), "no-such-sha".into()];
        assert!(
            with_token(|token| exec.run_records("git log", &argv, &token)).is_err(),
            "an invalid revision must not become an empty successful record set"
        );
    }

    /// require_full_stdout refuses a head posing as the whole stream.
    #[test]
    fn truncated_stdout_is_refused() {
        let run = GitRun {
            success: true,
            code: Some(0),
            stdout: b"only-a-head".to_vec(),
            stderr: Vec::new(),
            stdout_dropped: 4096,
            stderr_dropped: 0,
        };
        let error = run.require_full_stdout("git log").unwrap_err();
        assert!(error.contains("truncated"), "{error}");
    }

    /// The backend is chosen by the target: a remote RepoTarget can
    /// only construct the remote exec, a local one only local.
    #[test]
    fn for_target_selects_the_only_valid_backend() {
        let local = RepoTarget::Local {
            workdir: std::path::PathBuf::from("/w"),
        };
        assert!(matches!(GitExec::for_target(&local), GitExec::Local { .. }));
        let remote = RepoTarget::Remote {
            endpoint: RemoteEndpoint::parse("ssh://fixture@box:2222").unwrap(),
            workdir: std::path::PathBuf::from("/srv/proj"),
        };
        match GitExec::for_target(&remote) {
            GitExec::Remote { endpoint, workdir } => {
                assert_eq!(endpoint, remote.endpoint().unwrap().clone());
                assert_eq!(workdir, Path::new("/srv/proj"));
            }
            other => panic!("remote target built {other:?}"),
        }
    }

    /// A container RepoTarget constructs the container exec carrying
    /// the canonical id and workdir — nothing else can.
    #[test]
    fn for_target_selects_container_for_a_container_target() {
        let target = RepoTarget::Container {
            container: ContainerId::canonical("d".repeat(64)).unwrap(),
            workdir: std::path::PathBuf::from("/work/src"),
        };
        match GitExec::for_target(&target) {
            GitExec::Container { container, workdir } => {
                assert_eq!(container.as_str(), &"d".repeat(64));
                assert_eq!(workdir, Path::new("/work/src"));
            }
            other => panic!("container target built {other:?}"),
        }
    }

    /// A container run's argv starts `-C <workdir>` and refuses a
    /// non-UTF-8 workdir before the engine is ever probed — a typed
    /// refusal, never a lossy path guess. (A valid workdir would reach
    /// the engine probe next, which has no fixture here.)
    #[cfg(unix)]
    #[test]
    fn container_run_refuses_a_non_utf8_workdir_typed() {
        use std::os::unix::ffi::OsStrExt;
        let workdir = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/w/\xff"));
        let exec = GitExec::Container {
            container: ContainerId::canonical("d".repeat(64)).unwrap(),
            workdir: &workdir,
        };
        let argv: Vec<OsString> = vec!["status".into()];
        let error = with_token(|token| exec.run(&argv, &token)).unwrap_err();
        match error {
            GitExecError::Container(ContainerError::CapabilityRefused { what }) => {
                assert!(what.contains("UTF-8"), "{what}");
            }
            other => panic!("expected a typed capability refusal, got {other:?}"),
        }
    }

    /// A native-byte argv element (a filename git itself would accept
    /// locally) is refused typed at the container boundary — the remote
    /// backend carries such bytes; `docker exec` argv text cannot.
    #[cfg(unix)]
    #[test]
    fn container_run_refuses_a_non_utf8_argument_typed() {
        use std::os::unix::ffi::OsStrExt;
        let workdir = std::path::PathBuf::from("/work");
        let exec = GitExec::Container {
            container: ContainerId::canonical("d".repeat(64)).unwrap(),
            workdir: &workdir,
        };
        let argv: Vec<OsString> = vec![
            "ls-files".into(),
            std::ffi::OsStr::from_bytes(b"\xff.txt").into(),
        ];
        let error = with_token(|token| exec.run(&argv, &token)).unwrap_err();
        assert!(
            matches!(
                error,
                GitExecError::Container(ContainerError::CapabilityRefused { .. })
            ),
            "{error:?}"
        );
    }

    /// The container failure surfaces through GitExecError's display
    /// with the boundary's own typed diagnosis.
    #[test]
    fn container_error_displays_the_boundary_diagnosis() {
        let error = GitExecError::Container(ContainerError::NotRunning { id: "abc".into() });
        assert_eq!(error.to_string(), "container is not running: abc");
    }
}
