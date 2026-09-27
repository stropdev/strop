//! Finite Git execution in the owning namespace: local process jobs
//! keep their in-process libgit2 hot paths, while admitted SSH/container
//! namespaces run byte-exact `git` through their one native worker.
//! No Python or shell-supervised alternate execution path.
//!
//! Exit codes are data on both sides (`diff --quiet` exits 1): [`GitRun`]
//! reports success and the raw code; callers that need "did git itself
//! refuse" match on the code, not on stderr text. Remote output bounds
//! are surfaced as `dropped` counts — a parser that needs a complete
//! record stream must refuse truncated output, never parse a prefix as
//! if it were the whole answer.

use std::ffi::OsString;
use std::path::Path;

use strop_core::worker::CancelToken;
use strop_worker_client::{ClientError, Worker};
use strop_worker_protocol::ExitStatus;

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
    /// A non-local namespace without an admitted worker cannot run
    /// Git there. Never retry through Python, docker exec or locally.
    Unavailable { namespace: &'static str },
    /// The admitted worker lease refused or failed the run (0058 WK10):
    /// transport, admission and cancellation stay the client's own typed
    /// taxonomy, and an unattested exit is [`WorkerGitError::ExitLost`],
    /// never a guessed code.
    Worker(WorkerGitError),
}

/// The worker-routed failure taxonomy (0058 WK10): the client boundary's
/// own typed error, or the one worker-native condition it cannot carry.
#[derive(Debug)]
pub enum WorkerGitError {
    /// The lease's typed refusal/failure (admission, transport,
    /// cancellation) surfaced unchanged.
    Client(ClientError),
    /// The worker's supervisor could not attest the exit (lease lost,
    /// worker died mid-run). Never mapped to a guessed exit code.
    ExitLost,
    /// An argv element or workdir could not be carried as native wire
    /// bytes on this platform — a typed refusal, never a lossy guess.
    NotRepresentable {
        /// What could not be represented, named for the user.
        what: String,
    },
    /// An output stream failed mid-run (short stream, dead connection):
    /// transport truth, never mistaken for git's own output.
    Stream(String),
}

impl std::fmt::Display for WorkerGitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(error) => write!(f, "{error}"),
            Self::ExitLost => write!(f, "worker could not attest the git exit status"),
            Self::NotRepresentable { what } => write!(f, "{what}"),
            Self::Stream(message) => write!(f, "worker stream failed: {message}"),
        }
    }
}

impl std::error::Error for WorkerGitError {}

impl std::fmt::Display for GitExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(message) => write!(f, "{message}"),
            Self::Unavailable { namespace } => {
                write!(f, "{namespace} Git requires an admitted worker")
            }
            Self::Worker(error) => write!(f, "{error}"),
        }
    }
}

/// Bounded retained output for one worker-owned Git run. A parser
/// requiring complete records refuses a truncated head.
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;

/// Where one Git query executes. Constructed from a [`RepoTarget`] so a
/// non-local workdir can only ever reach its matching backend.
// The worker lease is not `Debug`; the backends' identity is their data.
#[derive(Clone)]
pub enum GitExec<'a> {
    Local {
        workdir: &'a Path,
    },
    /// One admitted worker lease owns the repository's namespace (0058
    /// WK10): finite `git` rides the lease's supervised exec — SSH
    /// endpoint and container incarnation alike, since the lease is
    /// already the namespace boundary. The workdir is a native path in
    /// the worker's own namespace.
    Worker {
        worker: Worker,
        workdir: &'a Path,
    },
}

impl<'a> GitExec<'a> {
    /// The selected repository's native execution. Local worktrees
    /// retain in-process Git; a non-local target requires an already
    /// admitted worker lease and never falls back to a second command
    /// supervisor. Admission of the endpoint/container occurs above
    /// this pure routing decision.
    pub fn for_target_routed(
        target: &'a RepoTarget,
        lease: Option<&Worker>,
    ) -> Result<Self, GitExecError> {
        match (target, lease) {
            (RepoTarget::Local { workdir }, _) => Ok(Self::Local { workdir }),
            (RepoTarget::Remote { workdir, .. }, Some(worker))
            | (RepoTarget::Container { workdir, .. }, Some(worker)) => Ok(Self::Worker {
                worker: worker.clone(),
                workdir,
            }),
            (RepoTarget::Remote { .. }, None) => {
                Err(GitExecError::Unavailable { namespace: "SSH" })
            }
            (RepoTarget::Container { .. }, None) => Err(GitExecError::Unavailable {
                namespace: "container",
            }),
        }
    }

    /// Run `git <argv>` in the selected repository. The local variant
    /// prefixes `-C`; the worker runs with a native-byte cwd and argv.
    /// No argument is executable shell text, even with control bytes.
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
            Self::Worker { worker, workdir } => {
                run_worker_program(worker, b"git", workdir, argv, cancel)
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
        if run.stdout_dropped > 0 {
            return Err(format!(
                "{op}: remote stdout truncated ({} bytes dropped)",
                run.stdout_dropped
            ));
        }
        Ok(run.stdout)
    }
}

/// One trusted Git/ssh-config program in its admitted worker namespace.
/// No shell parses argv or cwd; stdout and stderr drain concurrently
/// and complete streams never pass as a truncated head.
pub(crate) fn run_worker_program(
    worker: &Worker,
    program: &'static [u8],
    workdir: &Path,
    argv: &[OsString],
    cancel: &CancelToken,
) -> Result<GitRun, GitExecError> {
    let mut wire_argv = Vec::with_capacity(argv.len());
    for arg in argv {
        wire_argv.push(argv_bytes(arg)?);
    }
    let spec = strop_worker_protocol::ExecSpec {
        program: program.to_vec(),
        argv: wire_argv,
        cwd: path_bytes(workdir)?,
        env: Vec::new(),
        service: false,
        pty: None,
    };
    let handle = worker
        .exec(cancel, spec)
        .map_err(|error| GitExecError::Worker(WorkerGitError::Client(error)))?;
    let (exec, stdin, mut stdout, stderr, exit) = handle.into_parts();
    let control = stdin.control(worker.clone(), exec);
    let stderr_pump = std::thread::spawn(move || {
        let mut stderr = stderr;
        read_bounded(&mut stderr, OUTPUT_LIMIT)
    });
    let streams: Result<_, GitExecError> = (|| {
        let (stdout, stdout_dropped) =
            read_bounded(&mut stdout, OUTPUT_LIMIT).map_err(worker_stream)?;
        let (stderr, stderr_dropped) = stderr_pump
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("stderr pump panicked")))
            .map_err(worker_stream)?;
        Ok((stdout, stdout_dropped, stderr, stderr_dropped))
    })();
    if streams.is_err() {
        let (token, _owner) = CancelToken::standalone();
        let _ = control.cancel(&token);
    }
    let (stdout, stdout_dropped, stderr, stderr_dropped) = streams?;
    match exit.wait() {
        ExitStatus::Exit(code) => Ok(GitRun {
            success: code == 0,
            code: Some(code),
            stdout,
            stderr,
            stdout_dropped,
            stderr_dropped,
        }),
        ExitStatus::Signal(_) => Ok(GitRun {
            success: false,
            code: None,
            stdout,
            stderr,
            stdout_dropped,
            stderr_dropped,
        }),
        ExitStatus::Lost => Err(GitExecError::Worker(WorkerGitError::ExitLost)),
    }
}

/// One [`OsString`] as wire argv bytes (0058 WK10): Unix keeps arbitrary
/// non-NUL bytes; other platforms require UTF-8 — a typed refusal, never
/// a lossy rename of a filename the far end's git must open.
fn argv_bytes(arg: &OsString) -> Result<Vec<u8>, GitExecError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(arg.as_os_str().as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        arg.to_str()
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| {
                GitExecError::Worker(WorkerGitError::NotRepresentable {
                    what: format!(
                        "worker git: argument is not UTF-8 ({:?})",
                        arg.to_string_lossy()
                    ),
                })
            })
    }
}

/// One workdir as wire cwd bytes — the same byte-exactness rule as
/// [`argv_bytes`].
fn path_bytes(path: &Path) -> Result<Vec<u8>, GitExecError> {
    argv_bytes(&path.as_os_str().to_os_string())
}

/// A payload read failure is transport truth (the stream ended short or
/// the connection died), typed at the worker boundary — never mistaken
/// for git's own output.
fn worker_stream(error: std::io::Error) -> GitExecError {
    GitExecError::Worker(WorkerGitError::Stream(error.to_string()))
}

/// Read one stream to EOF keeping the first `cap` bytes; the remainder
/// is drained and counted as dropped (the worker's pump never stalls
/// behind an unread stream), so a parser still refuses a head rather
/// than parsing a prefix as the whole answer.
fn read_bounded(reader: &mut impl std::io::Read, cap: u64) -> std::io::Result<(Vec<u8>, u64)> {
    let mut kept = Vec::new();
    let mut dropped = 0_u64;
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok((kept, dropped));
        }
        let room = (cap as usize).saturating_sub(kept.len());
        let take = room.min(read);
        kept.extend_from_slice(&buffer[..take]);
        dropped += (read - take) as u64;
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
}
