//! Read-only Git queries in an admitted SSH worker namespace. Every
//! machine-format parser consumes the worker's native bytes; absent
//! Git capability is typed, never an empty repository or local retry.
//! libgit2 remains local and no Python supervisor crosses this path.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use strop_core::worker::CancelToken;
use strop_remote::worker_transport::RemoteWorker;
use strop_workspace::RemoteEndpoint;

use crate::diff::FileDiff;
use crate::exec::{run_worker_program, GitExec, GitExecError, GitRun};
use crate::repo::{gutter_from_contents, hunks_from_buffers};
use crate::ssh::parse_effective_hostname;
use crate::target::RepoTarget;
use crate::{GitContext, Hunk};

/// A bound lease is the only SSH Git execution route. The endpoint
/// check prevents a workspace alias from retargeting the worker's cwd.
fn backend<'a>(
    endpoint: &RemoteEndpoint,
    workdir: &'a Path,
    lease: Option<&RemoteWorker>,
) -> Result<GitExec<'a>, RemoteGitError> {
    let lease = lease
        .ok_or_else(|| RemoteGitError::Capability("SSH Git requires an admitted worker".into()))?;
    if lease.endpoint() != endpoint {
        return Err(RemoteGitError::Capability(format!(
            "Git scope {endpoint} does not match the admitted worker"
        )));
    }
    Ok(GitExec::Worker {
        worker: lease.worker().clone(),
        workdir,
    })
}

mod parsers;

use parsers::{
    parse_commit_parents, parse_index_entry, parse_remote_config, parse_sha, parse_toplevel,
    parse_tree_entry,
};

/// Why a remote or in-container Git query failed — typed at this
/// boundary, never a bare string and never an empty list standing in
/// for "failed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteGitError {
    /// The worker or its process/stream boundary refused or failed.
    Exec(String),
    /// No worker was admitted or the supplied lease belongs to another
    /// endpoint: never use a Python or local-path fallback.
    Capability(String),
    /// `git` itself exited nonzero: the operation name, exit code and
    /// git's stderr.
    Exit {
        op: &'static str,
        code: i32,
        stderr: String,
    },
    /// A record git produced did not have the shape this parser
    /// admits — corrupt or surprising output is refused, never guessed.
    Parse(&'static str),
    /// Blob content that must be UTF-8 for the surface was not.
    Utf8(&'static str),
    /// The remote `git` output bound truncated a stream this query
    /// needs complete.
    Truncated { op: &'static str, dropped: u64 },
}

impl std::fmt::Display for RemoteGitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exec(message) => write!(f, "{message}"),
            Self::Capability(message) => write!(f, "{message}"),
            Self::Exit { op, code, stderr } => {
                write!(f, "{op}: git exited {code}: {}", stderr.trim())
            }
            Self::Parse(what) => write!(f, "{what}: unparseable git output"),
            Self::Utf8(what) => write!(f, "{what} is not UTF-8"),
            Self::Truncated { op, dropped } => {
                write!(f, "{op}: remote output truncated ({dropped} bytes dropped)")
            }
        }
    }
}

impl std::error::Error for RemoteGitError {}

impl From<GitExecError> for RemoteGitError {
    fn from(error: GitExecError) -> Self {
        Self::Exec(error.to_string())
    }
}

/// The typed result of one bounded run that must exit zero with
/// complete stdout: bytes, or the honest failure.
fn records(
    exec: &GitExec,
    op: &'static str,
    argv: &[OsString],
    cancel: &CancelToken,
) -> Result<Vec<u8>, RemoteGitError> {
    let run = exec.run(argv, cancel)?;
    exit_or_bytes(op, &run)
}

fn exit_or_bytes(op: &'static str, run: &GitRun) -> Result<Vec<u8>, RemoteGitError> {
    if run.stdout_dropped > 0 {
        return Err(RemoteGitError::Truncated {
            op,
            dropped: run.stdout_dropped,
        });
    }
    match run.code {
        Some(0) => Ok(run.stdout.clone()),
        Some(code) => Err(RemoteGitError::Exit {
            op,
            code,
            stderr: String::from_utf8_lossy(&run.stderr).trim_end().to_string(),
        }),
        None => Err(RemoteGitError::Exit {
            op,
            code: -1,
            stderr: String::from_utf8_lossy(&run.stderr).trim_end().to_string(),
        }),
    }
}

fn remotes_from_run(config_run: &GitRun) -> Result<Vec<(String, String)>, RemoteGitError> {
    const OP: &str = "config --get-regexp remote.*.url";
    if config_run.code != Some(0) && config_run.code != Some(1) {
        return Err(RemoteGitError::Exit {
            op: OP,
            code: config_run.code.unwrap_or(-1),
            stderr: String::from_utf8_lossy(&config_run.stderr)
                .trim_end()
                .to_string(),
        });
    }
    if config_run.stdout_dropped > 0 {
        return Err(RemoteGitError::Truncated {
            op: OP,
            dropped: config_run.stdout_dropped,
        });
    }
    parse_remote_config(&config_run.stdout)
}

/// Discover the repository containing a remote directory. `Ok(None)` is
/// the honest "no repository here" — git's own not-a-repository fatal
/// — while transport failures, missing tooling and corrupt
/// repositories are typed errors. The returned workdir is a path on
/// the endpoint, native bytes, never a local path. `lease` routes the
/// run through the endpoint's admitted worker when the session holds
/// one (0058 WK10).
pub fn discover(
    endpoint: &RemoteEndpoint,
    from: &Path,
    lease: Option<&RemoteWorker>,
    cancel: &CancelToken,
) -> Result<Option<PathBuf>, RemoteGitError> {
    let exec = backend(endpoint, from, lease)?;
    discover_with(&exec, cancel)
}

/// The exec-generic discovery core: one bounded
/// `rev-parse --show-toplevel` through any backend, mapped the same
/// way for every non-local worktree.
pub(crate) fn discover_with(
    exec: &GitExec,
    cancel: &CancelToken,
) -> Result<Option<PathBuf>, RemoteGitError> {
    let run = exec.run(&["rev-parse".into(), "--show-toplevel".into()], cancel)?;
    discover_from_run(&run)
}

/// Map one `rev-parse --show-toplevel` run to the discovery answer:
/// exit 0 parses the toplevel; 128 with git's stable not-a-repository
/// fatal is the honest `None`; any other nonzero (unsafe repository,
/// broken .git, missing git…) stays a typed error.
pub(crate) fn discover_from_run(run: &GitRun) -> Result<Option<PathBuf>, RemoteGitError> {
    if run.success {
        let stdout = exit_or_bytes("rev-parse --show-toplevel", run)?;
        return parse_toplevel(&stdout).map(Some);
    }
    let stderr = String::from_utf8_lossy(&run.stderr);
    if run.code == Some(128) && stderr.contains("not a git repository") {
        return Ok(None);
    }
    Err(RemoteGitError::Exit {
        op: "rev-parse --show-toplevel",
        code: run.code.unwrap_or(-1),
        stderr: stderr.trim_end().to_string(),
    })
}

/// The pure cached context of a remote repository (R6): HEAD sha,
/// branch and remotes captured once on a worker. Unborn HEAD and
/// detached HEAD are honest `None`/name states, distinguished by exit
/// code — never by stderr text.
pub fn context(
    endpoint: &RemoteEndpoint,
    workdir: &Path,
    lease: Option<&RemoteWorker>,
    cancel: &CancelToken,
) -> Result<GitContext, RemoteGitError> {
    let exec = backend(endpoint, workdir, lease)?;
    let repo = RepoTarget::Remote {
        endpoint: endpoint.clone(),
        workdir: workdir.to_path_buf(),
    };
    context_with(&exec, repo, cancel)
}

/// The exec-generic context core: the same three bounded runs on any
/// backend, assembled into the same [`GitContext`] shape — only the
/// [`RepoTarget`] the caller hands in differs.
pub(crate) fn context_with(
    exec: &GitExec,
    repo: RepoTarget,
    cancel: &CancelToken,
) -> Result<GitContext, RemoteGitError> {
    // exit 0 = sha, exit 1 = unborn (no commits), anything else fails.
    let head_run = exec.run(
        &[
            "rev-parse".into(),
            "--verify".into(),
            "--quiet".into(),
            "HEAD".into(),
        ],
        cancel,
    )?;
    // exit 0 = branch name (unborn included: the symref exists), 128 =
    // detached HEAD (no symbolic ref), anything else fails.
    let branch_run = exec.run(
        &["symbolic-ref".into(), "--short".into(), "HEAD".into()],
        cancel,
    )?;
    // exit 0 or 1 (no remotes configured): both are data.
    let config_run = exec.run(
        &[
            "config".into(),
            "-z".into(),
            "--get-regexp".into(),
            "^remote\\.[^.]+\\.url$".into(),
        ],
        cancel,
    )?;
    context_from_runs(repo, &head_run, &branch_run, &config_run)
}

/// Assemble the cached context from the three runs — pure, so both
/// non-local backends share one mapping and its tests.
pub(crate) fn context_from_runs(
    repo: RepoTarget,
    head_run: &GitRun,
    branch_run: &GitRun,
    config_run: &GitRun,
) -> Result<GitContext, RemoteGitError> {
    Ok(GitContext {
        repo,
        head_sha: head_sha_from_run(head_run)?,
        head_branch: head_branch_from_run(branch_run)?,
        remotes: remotes_from_run(config_run)?,
    })
}

fn head_sha_from_run(run: &GitRun) -> Result<Option<String>, RemoteGitError> {
    if run.success {
        return parse_sha(&run.stdout, "rev-parse HEAD").map(Some);
    }
    if run.code == Some(1) {
        return Ok(None);
    }
    Err(RemoteGitError::Exit {
        op: "rev-parse HEAD",
        code: run.code.unwrap_or(-1),
        stderr: String::from_utf8_lossy(&run.stderr).trim_end().to_string(),
    })
}

fn head_branch_from_run(run: &GitRun) -> Result<Option<String>, RemoteGitError> {
    if run.success {
        let name = String::from_utf8_lossy(&run.stdout).trim().to_string();
        return Ok((!name.is_empty()).then_some(name));
    }
    if run.code == Some(128) {
        return Ok(None);
    }
    Err(RemoteGitError::Exit {
        op: "symbolic-ref HEAD",
        code: run.code.unwrap_or(-1),
        stderr: String::from_utf8_lossy(&run.stderr).trim_end().to_string(),
    })
}

/// HEAD's and the index's blob bytes for one repo-relative path — the
/// gutter diff's inputs. `head_sha` is the context's resolved HEAD (an
/// unborn repository passes `None` and HEAD is not asked). Absent from
/// HEAD's tree / absent from the index are honest `None`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContents {
    pub head: Option<Vec<u8>>,
    pub index: Option<Vec<u8>>,
}

/// Fetch a file's HEAD and index contents and assemble the gutter's
/// typed hunk sets against the live buffer text — the same semantics
/// [`crate::Repo::unstaged_hunks`]/[`staged_hunks`]/[`is_untracked`]
/// give local buffers, from bounded remote reads. Untracked files
/// report one all-add hunk, matching local behavior.
pub fn gutter(
    endpoint: &RemoteEndpoint,
    workdir: &Path,
    head_sha: Option<&str>,
    rel: &Path,
    text: &str,
    lease: Option<&RemoteWorker>,
    cancel: &CancelToken,
) -> Result<(Vec<Hunk>, Vec<Hunk>, bool), RemoteGitError> {
    let contents = file_contents(endpoint, workdir, head_sha, rel, lease, cancel)?;
    let head = contents
        .head
        .as_deref()
        .map(|bytes| std::str::from_utf8(bytes).map_err(|_| RemoteGitError::Utf8("HEAD blob")))
        .transpose()?;
    let index = contents
        .index
        .as_deref()
        .map(|bytes| std::str::from_utf8(bytes).map_err(|_| RemoteGitError::Utf8("index blob")))
        .transpose()?;
    gutter_from_contents(head, index, text, rel)
        .map_err(|error| RemoteGitError::Exec(error.to_string()))
}

/// One file's HEAD/index blob bytes via machine-readable presence
/// checks (`ls-tree`/`ls-files`, `-z`, literal pathspec) followed by
/// `cat-file` on the recorded oid — never a `:path` spelling whose
/// absence would need stderr matching to interpret.
pub fn file_contents(
    endpoint: &RemoteEndpoint,
    workdir: &Path,
    head_sha: Option<&str>,
    rel: &Path,
    lease: Option<&RemoteWorker>,
    cancel: &CancelToken,
) -> Result<FileContents, RemoteGitError> {
    let exec = backend(endpoint, workdir, lease)?;
    let head = match head_sha {
        Some(sha) => {
            let stdout = records(
                &exec,
                "ls-tree HEAD",
                &[
                    "ls-tree".into(),
                    "-z".into(),
                    sha.into(),
                    "--".into(),
                    rel.as_os_str().into(),
                ],
                cancel,
            )?;
            match parse_tree_entry(&stdout)? {
                Some((oid, path)) if path == rel => {
                    Some(blob_bytes(&exec, &oid, "HEAD blob", cancel)?)
                }
                // an empty record set is the honest absence; a record
                // for a different path cannot come from a literal
                // pathspec and is refused
                Some((_, _)) => return Err(RemoteGitError::Parse("ls-tree HEAD")),
                None => None,
            }
        }
        None => None,
    };
    let stdout = records(
        &exec,
        "ls-files --stage",
        &[
            "ls-files".into(),
            "-z".into(),
            "--stage".into(),
            "--".into(),
            rel.as_os_str().into(),
        ],
        cancel,
    )?;
    let index = match parse_index_entry(&stdout)? {
        Some((oid, path)) if path == rel => Some(blob_bytes(&exec, &oid, "index blob", cancel)?),
        Some((_, _)) => return Err(RemoteGitError::Parse("ls-files --stage")),
        None => None,
    };
    Ok(FileContents { head, index })
}

fn blob_bytes(
    exec: &GitExec,
    oid: &str,
    what: &'static str,
    cancel: &CancelToken,
) -> Result<Vec<u8>, RemoteGitError> {
    records(
        exec,
        what,
        &["cat-file".into(), "-p".into(), oid.into()],
        cancel,
    )
}

/// One file's structured delta at `sha` against its first parent (root
/// commits diff against empty): the dive view's data, built from the
/// two blobs and the same hunk builder the local libgit2 path uses.
pub fn commit_file_diff(
    endpoint: &RemoteEndpoint,
    workdir: &Path,
    sha: &str,
    rel: &Path,
    lease: Option<&RemoteWorker>,
    cancel: &CancelToken,
) -> Result<FileDiff, RemoteGitError> {
    let exec = backend(endpoint, workdir, lease)?;
    let stdout = records(
        &exec,
        "rev-list --parents",
        &[
            "rev-list".into(),
            "--parents".into(),
            "-n".into(),
            "1".into(),
            sha.into(),
        ],
        cancel,
    )?;
    let (commit, parent) = parse_commit_parents(&stdout)?;
    if commit != sha {
        return Err(RemoteGitError::Parse("rev-list --parents"));
    }
    let parent_blob = match parent.as_deref() {
        Some(parent) => {
            let stdout = records(
                &exec,
                "ls-tree parent",
                &[
                    "ls-tree".into(),
                    "-z".into(),
                    parent.into(),
                    "--".into(),
                    rel.as_os_str().into(),
                ],
                cancel,
            )?;
            match parse_tree_entry(&stdout)? {
                Some((oid, path)) if path == rel => {
                    Some(blob_bytes(&exec, &oid, "parent blob", cancel)?)
                }
                Some((_, _)) => return Err(RemoteGitError::Parse("ls-tree parent")),
                None => None,
            }
        }
        None => None,
    };
    let stdout = records(
        &exec,
        "ls-tree commit",
        &[
            "ls-tree".into(),
            "-z".into(),
            commit.clone().into(),
            "--".into(),
            rel.as_os_str().into(),
        ],
        cancel,
    )?;
    let commit_blob = match parse_tree_entry(&stdout)? {
        Some((oid, path)) if path == rel => blob_bytes(&exec, &oid, "commit blob", cancel)?,
        Some((_, _)) => return Err(RemoteGitError::Parse("ls-tree commit")),
        None => return Err(RemoteGitError::Exec("no diff for path".into())),
    };
    let hunks = hunks_from_buffers(parent_blob.as_deref(), &commit_blob, rel)
        .map_err(|error| RemoteGitError::Exec(error.to_string()))?;
    Ok(FileDiff::from_hunks(rel.to_path_buf(), hunks))
}

/// Resolve an SSH alias against OpenSSH's effective configuration *on
/// the endpoint* (`ssh -G` run remotely): the alias belongs to the
/// machine whose remote carries it, and evaluating it with the local
/// user's config would answer for the wrong host. Owned worker work,
/// cancellable like every bounded remote run.
pub fn effective_host(
    endpoint: &RemoteEndpoint,
    workdir: &Path,
    remote: &crate::permalink::AliasRemote,
    lease: Option<&RemoteWorker>,
    cancel: &CancelToken,
) -> Result<String, crate::ssh::EffectiveHostError> {
    use crate::ssh::EffectiveHostError;
    let host = remote.host();
    if !crate::permalink::is_safe_host(host) {
        return Err(EffectiveHostError::InvalidHost);
    }
    let mut args: Vec<OsString> = vec!["-G".into()];
    if let Some(user) = &remote.user {
        args.extend(["-l".into(), user.into()]);
    }
    if let Some(port) = remote.port {
        args.extend(["-p".into(), port.to_string().into()]);
    }
    args.push(host.into());
    let lease = lease.ok_or_else(|| {
        EffectiveHostError::Unavailable(
            "SSH alias resolution requires an admitted worker on the endpoint".into(),
        )
    })?;
    if lease.endpoint() != endpoint {
        return Err(EffectiveHostError::Unavailable(format!(
            "SSH alias scope {endpoint} does not match the admitted worker"
        )));
    }
    let run = run_worker_program(lease.worker(), b"ssh", workdir, &args, cancel)
        .map_err(|error| EffectiveHostError::Spawn(error.to_string()))?;
    if !run.success {
        return Err(EffectiveHostError::Failed(
            String::from_utf8_lossy(&run.stderr).trim().to_string(),
        ));
    }
    if run.stdout_dropped > 0 {
        return Err(EffectiveHostError::Failed(
            "output truncated before a hostname line".into(),
        ));
    }
    let stdout = String::from_utf8_lossy(&run.stdout);
    let hostname = parse_effective_hostname(&stdout).ok_or(EffectiveHostError::NoHostname)?;
    if hostname == host && !hostname.contains('.') {
        return Err(EffectiveHostError::Unresolved);
    }
    Ok(hostname)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exit-code meaning is preserved end to end: the error carries the
    /// operation, git's own exit code and stderr — never a guessed
    /// empty result.
    #[test]
    fn nonzero_exit_is_a_typed_error() {
        let run = GitRun {
            success: false,
            code: Some(128),
            stdout: Vec::new(),
            stderr: b"fatal: unsafe repository\n".to_vec(),
            stdout_dropped: 0,
            stderr_dropped: 0,
        };
        match exit_or_bytes("rev-parse --show-toplevel", &run) {
            Err(RemoteGitError::Exit { op, code, stderr }) => {
                assert_eq!(op, "rev-parse --show-toplevel");
                assert_eq!(code, 128);
                assert_eq!(stderr, "fatal: unsafe repository");
            }
            other => panic!("expected Exit, got {other:?}"),
        }
    }

    #[test]
    fn no_matching_remote_config_is_data_but_real_failure_is_not() {
        let mut run = GitRun {
            success: false,
            code: Some(1),
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_dropped: 0,
            stderr_dropped: 0,
        };
        assert!(remotes_from_run(&run).unwrap().is_empty());
        run.code = Some(3);
        assert!(matches!(
            remotes_from_run(&run),
            Err(RemoteGitError::Exit { code: 3, .. })
        ));
        run.code = Some(1);
        run.stdout_dropped = 1;
        assert!(matches!(
            remotes_from_run(&run),
            Err(RemoteGitError::Truncated { .. })
        ));
    }

    /// Truncated stdout cannot pose as a complete record stream.
    #[test]
    fn truncation_is_typed() {
        let run = GitRun {
            success: true,
            code: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_dropped: 4096,
            stderr_dropped: 0,
        };
        assert_eq!(
            exit_or_bytes("ls-tree", &run),
            Err(RemoteGitError::Truncated {
                op: "ls-tree",
                dropped: 4096
            })
        );
    }
}
