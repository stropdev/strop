//! Finite Git execution in the owning namespace: local runs spawn `git`
//! here (libgit2 keeps the in-process hot paths), and SSH workspaces run
//! byte-exact `git` on the host as a channel on the endpoint's master
//! (plan 0090 §5). Both are the same bounded, cancellable capture.
//!
//! Exit codes are data on both sides (`diff --quiet` exits 1): [`GitRun`]
//! reports success and the raw code; callers that need "did git itself
//! refuse" match on the code, not on stderr text. Output bounds are
//! surfaced as `dropped` counts — a parser that needs a complete record
//! stream must refuse truncated output, never parse a prefix as if it
//! were the whole answer.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use strop_core::process::{capture_with, CaptureError, CapturePolicy, StdinPolicy};
use strop_core::worker::CancelToken;
use strop_remote::exec::ExecHost;

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
    /// Bytes dropped past the output bound. A nonzero count means
    /// `stdout`/`stderr` are heads, not the complete stream.
    pub stdout_dropped: u64,
    pub stderr_dropped: u64,
}

impl GitRun {
    /// Fail typed when the caller needs the complete stdout record
    /// stream: a truncated head must never parse as "everything git
    /// said".
    pub fn require_full_stdout(&self, op: &str) -> Result<&[u8], String> {
        if self.stdout_dropped > 0 {
            return Err(format!(
                "{op}: output truncated ({} bytes dropped)",
                self.stdout_dropped
            ));
        }
        Ok(&self.stdout)
    }
}

/// Why a bounded `git` run could not produce its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitExecError {
    /// `git` (or `ssh`) could not be started.
    Spawn(String),
    /// The run was cancelled by its owner.
    Cancelled,
    /// The run outlived its deadline and was killed.
    TimedOut(Duration),
    /// Supervision itself failed (waiting, reading a pipe).
    Failed(String),
    /// The repository is on another endpoint than the host offered.
    WrongHost { repo: String, host: String },
}

impl std::fmt::Display for GitExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(message) | Self::Failed(message) => write!(f, "{message}"),
            Self::Cancelled => write!(f, "git was cancelled"),
            Self::TimedOut(after) => write!(f, "git timed out after {} s", after.as_secs()),
            Self::WrongHost { repo, host } => {
                write!(f, "the repository on {repo} cannot run on {host}")
            }
        }
    }
}

impl std::error::Error for GitExecError {}

/// Retained output for one Git run. A parser requiring complete records
/// refuses a truncated head.
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
const STDERR_LIMIT: u64 = 64 * 1024;
pub(crate) const DEADLINE: Duration = Duration::from_secs(60);

/// Where one Git query executes. Constructed from a [`RepoTarget`] so a
/// non-local workdir can only ever reach its own endpoint's host.
#[derive(Clone)]
pub enum GitExec<'a> {
    Local {
        workdir: &'a Path,
    },
    /// `git` on the endpoint's host; the workdir is a path there.
    Remote {
        host: ExecHost,
        workdir: &'a Path,
    },
}

impl<'a> GitExec<'a> {
    /// The selected repository's execution: a local worktree runs here,
    /// a remote one on `host`, which must be the repository's endpoint.
    pub fn for_target(
        target: &'a RepoTarget,
        host: Option<&ExecHost>,
    ) -> Result<Self, GitExecError> {
        match target {
            RepoTarget::Local { workdir } => Ok(Self::Local { workdir }),
            RepoTarget::Remote { endpoint, workdir } => match host {
                Some(host) if host.endpoint() == endpoint => Ok(Self::Remote {
                    host: host.clone(),
                    workdir,
                }),
                Some(host) => Err(GitExecError::WrongHost {
                    repo: endpoint.to_string(),
                    host: host.endpoint().to_string(),
                }),
                None => Err(GitExecError::Spawn(format!("{endpoint} is not connected"))),
            },
        }
    }

    /// Run `git <argv>` in the selected repository. No argument is
    /// executable shell text, even with control bytes: the remote line
    /// quotes every word.
    pub fn run(&self, argv: &[OsString], cancel: &CancelToken) -> Result<GitRun, GitExecError> {
        self.run_within(argv, DEADLINE, cancel)
    }

    /// [`Self::run`] with the caller's deadline: a commit runs the
    /// repository's hooks, which may lint or test for minutes.
    pub fn run_within(
        &self,
        argv: &[OsString],
        deadline: Duration,
        cancel: &CancelToken,
    ) -> Result<GitRun, GitExecError> {
        match self {
            Self::Local { workdir } => {
                let mut command = Command::new("git");
                command.arg("-C").arg(workdir).args(argv);
                capture(&mut command, deadline, cancel)
            }
            Self::Remote { host, workdir } => {
                run_program(host, "git", workdir, argv, deadline, cancel)
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
        if run.stdout_dropped > 0 {
            return Err(format!(
                "{op}: stdout truncated ({} bytes dropped)",
                run.stdout_dropped
            ));
        }
        Ok(run.stdout)
    }
}

/// `program argv…` on `host` in `workdir`.
pub(crate) fn run_program(
    host: &ExecHost,
    program: &str,
    workdir: &Path,
    argv: &[OsString],
    deadline: Duration,
    cancel: &CancelToken,
) -> Result<GitRun, GitExecError> {
    let mut words: Vec<&OsStr> = vec![OsStr::new(program)];
    words.extend(argv.iter().map(OsString::as_os_str));
    let mut command = host.command(&words, Some(workdir));
    capture(&mut command, deadline, cancel)
}

fn capture(
    command: &mut Command,
    deadline: Duration,
    cancel: &CancelToken,
) -> Result<GitRun, GitExecError> {
    let policy = CapturePolicy {
        stdout_limit: OUTPUT_LIMIT,
        stderr_limit: STDERR_LIMIT,
        stderr_tail: 0,
        deadline,
        stdin: StdinPolicy::Null,
    };
    let output = capture_with(command, cancel, &policy).map_err(|error| match error {
        CaptureError::Spawn(message) => GitExecError::Spawn(format!("spawn git: {message}")),
        CaptureError::Cancelled => GitExecError::Cancelled,
        CaptureError::TimedOut(after) => GitExecError::TimedOut(after),
        CaptureError::Failure(failure) => GitExecError::Failed(failure.message),
    })?;
    Ok(GitRun {
        success: output.status.success(),
        code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        stdout_dropped: output.stdout_dropped,
        stderr_dropped: output.stderr_dropped,
    })
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
}
