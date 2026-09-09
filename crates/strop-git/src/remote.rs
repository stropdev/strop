//! The read-oriented remote Git backend (0036 RW8): bounded `git`
//! commands against a worktree that exists only on an SSH endpoint,
//! executed through the shared remote-execution boundary and parsed by
//! the same structured parsers as the local backend. No libgit2 here —
//! no remote filesystem is ever assumed local — and no mutation verbs
//! exist on this path at all (RW4 keeps remote repositories read-only).
//!
//! Every machine-format boundary (`ls-tree`/`ls-files` records,
//! `rev-parse` output, `config -z` remotes, `rev-list --parents`) is a
//! pure parser fed by bytes captured from real `git`, so native —
//! possibly non-UTF-8 — filenames stay worktree identities and exit
//! codes carry the meaning instead of stderr matching. Unborn HEAD and
//! absent paths are honest `None`s; transport, tooling and truncation
//! failures are typed errors.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use strop_core::worker::CancelToken;
use strop_workspace::RemoteEndpoint;

use crate::diff::FileDiff;
use crate::exec::{GitExec, GitExecError, GitRun};
use crate::repo::{gutter_from_contents, hunks_from_buffers};
use crate::ssh::parse_effective_hostname;
use crate::target::RepoTarget;
use crate::{GitContext, Hunk};

/// Why a remote Git query failed — typed at this boundary, never a
/// bare string and never an empty list standing in for "failed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteGitError {
    /// The remote execution boundary refused or failed; the string is
    /// its own typed diagnosis (transport, supervisor, missing
    /// python3, cancellation, timeout…).
    Exec(String),
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
/// — while transport failures, missing `git`/python3 and corrupt
/// repositories are typed errors. The returned workdir is a path on
/// the endpoint, native bytes, never a local path.
pub fn discover(
    endpoint: &RemoteEndpoint,
    from: &Path,
    cancel: &CancelToken,
) -> Result<Option<PathBuf>, RemoteGitError> {
    let exec = GitExec::Remote {
        endpoint: endpoint.clone(),
        workdir: from,
    };
    let run = exec.run(&["rev-parse".into(), "--show-toplevel".into()], cancel)?;
    if run.success {
        let stdout = exit_or_bytes("rev-parse --show-toplevel", &run)?;
        return parse_toplevel(&stdout).map(Some);
    }
    // 128 with git's stable not-a-repository fatal: no repository. Any
    // other nonzero (unsafe repository, broken .git, missing git…)
    // stays a typed error.
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
    cancel: &CancelToken,
) -> Result<GitContext, RemoteGitError> {
    let exec = GitExec::Remote {
        endpoint: endpoint.clone(),
        workdir,
    };
    // exit 0 = sha, exit 1 = unborn (no commits), anything else fails.
    let head_sha = match exec.run(
        &[
            "rev-parse".into(),
            "--verify".into(),
            "--quiet".into(),
            "HEAD".into(),
        ],
        cancel,
    )? {
        run if run.success => Some(parse_sha(&run.stdout, "rev-parse HEAD")?),
        run if run.code == Some(1) => None,
        run => {
            return Err(RemoteGitError::Exit {
                op: "rev-parse HEAD",
                code: run.code.unwrap_or(-1),
                stderr: String::from_utf8_lossy(&run.stderr).trim_end().to_string(),
            })
        }
    };
    // exit 0 = branch name (unborn included: the symref exists), 128 =
    // detached HEAD (no symbolic ref), anything else fails.
    let head_branch = match exec.run(
        &["symbolic-ref".into(), "--short".into(), "HEAD".into()],
        cancel,
    )? {
        run if run.success => {
            let name = String::from_utf8_lossy(&run.stdout).trim().to_string();
            (!name.is_empty()).then_some(name)
        }
        run if run.code == Some(128) => None,
        run => {
            return Err(RemoteGitError::Exit {
                op: "symbolic-ref HEAD",
                code: run.code.unwrap_or(-1),
                stderr: String::from_utf8_lossy(&run.stderr).trim_end().to_string(),
            })
        }
    };
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
    let remotes = remotes_from_run(&config_run)?;
    Ok(GitContext {
        repo: RepoTarget::Remote {
            endpoint: endpoint.clone(),
            workdir: workdir.to_path_buf(),
        },
        head_sha,
        head_branch,
        remotes,
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
    cancel: &CancelToken,
) -> Result<(Vec<Hunk>, Vec<Hunk>, bool), RemoteGitError> {
    let contents = file_contents(endpoint, workdir, head_sha, rel, cancel)?;
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
    cancel: &CancelToken,
) -> Result<FileContents, RemoteGitError> {
    let exec = GitExec::Remote {
        endpoint: endpoint.clone(),
        workdir,
    };
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
    cancel: &CancelToken,
) -> Result<FileDiff, RemoteGitError> {
    let exec = GitExec::Remote {
        endpoint: endpoint.clone(),
        workdir,
    };
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
    cancel: &CancelToken,
) -> Result<String, crate::ssh::EffectiveHostError> {
    use crate::ssh::EffectiveHostError;
    let host = remote.host();
    if !crate::permalink::is_safe_host(host) {
        return Err(EffectiveHostError::InvalidHost);
    }
    let mut args = vec!["-G".into()];
    if let Some(user) = &remote.user {
        args.extend(["-l".into(), user.into()]);
    }
    if let Some(port) = remote.port {
        args.extend(["-p".into(), port.to_string().into()]);
    }
    args.push(host.into());
    let command = strop_remote::RemoteCommand::new("ssh", args, workdir)
        .map_err(|error| EffectiveHostError::Spawn(error.to_string()))?;
    let run = strop_remote::run(endpoint, &command, cancel)
        .map_err(|error| EffectiveHostError::Spawn(error.to_string()))?;
    if !run.status.success() {
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

// ---- pure wire parsers ---------------------------------------------------
//
// Each parser is fed by bytes captured from real `git` (see tests):
// native filename bytes survive, exit-code meanings are documented at
// their call sites, and nothing is guessed from stderr text.

/// `git rev-parse --show-toplevel`: one absolute native path with a
/// trailing newline. Relative or empty output is refused — a worktree
/// root on the endpoint is absolute by definition.
fn parse_toplevel(bytes: &[u8]) -> Result<PathBuf, RemoteGitError> {
    let trimmed = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if trimmed.is_empty() || trimmed.first() != Some(&b'/') {
        return Err(RemoteGitError::Parse("rev-parse --show-toplevel"));
    }
    Ok(bytes_to_path(trimmed))
}

/// A full object name: 40–64 lowercase-or-uppercase hex characters.
fn parse_sha(bytes: &[u8], what: &'static str) -> Result<String, RemoteGitError> {
    let text = String::from_utf8_lossy(bytes);
    let sha = text.trim();
    let valid = (40..=64).contains(&sha.len())
        && !sha.is_empty()
        && sha.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        return Err(RemoteGitError::Parse(what));
    }
    Ok(sha.to_string())
}

/// `git config -z --get-regexp '^remote\.[^.]+\.url$'` records:
/// `remote.<name>.url\n<url>\0`. Names and URLs are config strings;
/// non-UTF-8 values are refused rather than lossily renamed.
fn parse_remote_config(bytes: &[u8]) -> Result<Vec<(String, String)>, RemoteGitError> {
    let mut remotes = Vec::new();
    for record in bytes.split(|&b| b == 0) {
        if record.is_empty() {
            continue;
        }
        let Some((key, url)) = split_record(record, b'\n') else {
            return Err(RemoteGitError::Parse("config --get-regexp remote.*.url"));
        };
        let key = std::str::from_utf8(key).map_err(|_| RemoteGitError::Utf8("remote name"))?;
        let url = std::str::from_utf8(url).map_err(|_| RemoteGitError::Utf8("remote url"))?;
        let Some(name) = key
            .strip_prefix("remote.")
            .and_then(|rest| rest.strip_suffix(".url"))
        else {
            return Err(RemoteGitError::Parse("config --get-regexp remote.*.url"));
        };
        if name.is_empty() {
            return Err(RemoteGitError::Parse("config --get-regexp remote.*.url"));
        }
        remotes.push((name.to_string(), url.to_owned()));
    }
    Ok(remotes)
}

/// One `ls-tree -z` record: `mode SP type SP oid TAB path`. `None` for
/// an empty record set (the path is absent); the oid is validated hex
/// before anything embeds it in a later argv.
fn parse_tree_entry(bytes: &[u8]) -> Result<Option<(String, PathBuf)>, RemoteGitError> {
    let Some(record) = first_record(bytes) else {
        return Ok(None);
    };
    let Some((meta, path)) = split_record(record, b'\t') else {
        return Err(RemoteGitError::Parse("ls-tree"));
    };
    let mut fields = meta.split(|&b| b == b' ');
    let oid = fields.nth(2).ok_or(RemoteGitError::Parse("ls-tree"))?;
    let oid = parse_sha(oid, "ls-tree oid")?;
    Ok(Some((oid, bytes_to_path(path))))
}

/// One `ls-files -z --stage` record: `mode SP oid SP stage TAB path`.
fn parse_index_entry(bytes: &[u8]) -> Result<Option<(String, PathBuf)>, RemoteGitError> {
    let Some(record) = first_record(bytes) else {
        return Ok(None);
    };
    let Some((meta, path)) = split_record(record, b'\t') else {
        return Err(RemoteGitError::Parse("ls-files --stage"));
    };
    let mut fields = meta.split(|&b| b == b' ');
    let oid = fields
        .nth(1)
        .ok_or(RemoteGitError::Parse("ls-files --stage"))?;
    let oid = parse_sha(oid, "ls-files oid")?;
    Ok(Some((oid, bytes_to_path(path))))
}

/// `git rev-list --parents -n 1 <sha>`: `child [parent…]`, all full
/// object names. The first parent is the delta base (merges diff
/// against parent(0), matching local behavior).
fn parse_commit_parents(bytes: &[u8]) -> Result<(String, Option<String>), RemoteGitError> {
    let text = String::from_utf8_lossy(bytes);
    let mut shas = text
        .split_whitespace()
        .map(|token| parse_sha(token.as_bytes(), "rev-list --parents"));
    let commit = shas
        .next()
        .ok_or(RemoteGitError::Parse("rev-list --parents"))??;
    let parent = shas.next().transpose()?;
    Ok((commit, parent))
}

fn first_record(bytes: &[u8]) -> Option<&[u8]> {
    bytes.split(|&b| b == 0).find(|record| !record.is_empty())
}

fn split_record(bytes: &[u8], separator: u8) -> Option<(&[u8], &[u8])> {
    let index = bytes.iter().position(|&byte| byte == separator)?;
    Some((&bytes[..index], &bytes[index + 1..]))
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bytes below are captured from real `git` output (see
    // crates/strop-git/src/remote.rs doc comment); the regressions pin
    // the exact shapes the remote backend admits.

    #[test]
    fn toplevel_is_absolute_native_path() {
        assert_eq!(
            parse_toplevel(b"/srv/proj with space\n").unwrap(),
            PathBuf::from("/srv/proj with space")
        );
        assert!(
            parse_toplevel(b"srv/proj\n").is_err(),
            "relative is refused"
        );
        assert!(parse_toplevel(b"").is_err(), "empty is refused");
    }

    #[test]
    fn shas_are_validated_object_names() {
        assert_eq!(
            parse_sha(b"c59d8ceb7aeb96a1cdccff5646ec485acce32d45\n", "x").unwrap(),
            "c59d8ceb7aeb96a1cdccff5646ec485acce32d45"
        );
        assert!(parse_sha(b"main\n", "x").is_err());
        assert!(parse_sha(b"head is at 1234\n", "x").is_err());
        assert!(parse_sha(b"\n", "x").is_err());
    }

    /// config -z records split key\nvalue on NUL; a remote with no
    /// remotes is an empty vector, and a non-UTF-8 URL is refused.
    #[test]
    fn remote_config_records_parse_native() {
        let bytes = b"remote.origin.url\nhttps://example.com/acme/demo.git\0remote.up.url\ngit@gh:acme/other.git\0";
        assert_eq!(
            parse_remote_config(bytes).unwrap(),
            vec![
                (
                    "origin".to_string(),
                    "https://example.com/acme/demo.git".to_string()
                ),
                ("up".to_string(), "git@gh:acme/other.git".to_string()),
            ]
        );
        assert_eq!(
            parse_remote_config(b"").unwrap(),
            Vec::<(String, String)>::new()
        );
        assert!(parse_remote_config(b"not-a-pair\0").is_err());
        assert!(
            parse_remote_config(b"remote..url\nx\0").is_err(),
            "empty name"
        );
        assert!(
            parse_remote_config(b"remote.o.url\nhttps://a/\xff\xfe\0").is_err(),
            "non-UTF-8 url refused"
        );
    }

    /// ls-tree/ls-files -z records keep native path bytes — spaces,
    /// quotes and non-UTF-8 names arrive as the worktree identity —
    /// and an empty record set is the honest absence.
    #[test]
    fn tree_and_index_records_keep_native_paths() {
        let tree = b"100644 blob 45b983be36b73c0788dc9cbcb76cbb80fc7bb057\tsrc/a b.rs\0";
        let (oid, path) = parse_tree_entry(tree).unwrap().unwrap();
        assert_eq!(oid, "45b983be36b73c0788dc9cbcb76cbb80fc7bb057");
        assert_eq!(path, PathBuf::from("src/a b.rs"));
        assert_eq!(parse_tree_entry(b"").unwrap(), None);

        let index = b"100644 45b983be36b73c0788dc9cbcb76cbb80fc7bb057 0\tsrc/a b.rs\0";
        let (oid, path) = parse_index_entry(index).unwrap().unwrap();
        assert_eq!(oid, "45b983be36b73c0788dc9cbcb76cbb80fc7bb057");
        assert_eq!(path, PathBuf::from("src/a b.rs"));
        assert_eq!(parse_index_entry(b"").unwrap(), None);
        assert!(parse_tree_entry(b"garbage\0").is_err());
        assert!(parse_index_entry(b"100644 noshahere 0\tx\0").is_err());
    }

    /// rev-list --parents: child first, then parents; a root commit has
    /// none, and both shapes parse to (commit, Option<parent>).
    #[test]
    fn commit_parents_root_and_merged() {
        let root = b"60209d7ce72dddfafc1caacd511325c314a083bf\n";
        assert_eq!(
            parse_commit_parents(root).unwrap(),
            ("60209d7ce72dddfafc1caacd511325c314a083bf".to_string(), None)
        );
        let merged = b"c59d8ceb7aeb96a1cdccff5646ec485acce32d45 aaaa1111111111111111111111111111111111111 bbbb2222222222222222222222222222222222222\n";
        let (child, parent) = parse_commit_parents(merged).unwrap();
        assert_eq!(child, "c59d8ceb7aeb96a1cdccff5646ec485acce32d45");
        assert_eq!(
            parent.as_deref(),
            Some("aaaa1111111111111111111111111111111111111")
        );
        assert!(parse_commit_parents(b"not-a-sha\n").is_err());
        assert!(parse_commit_parents(b"").is_err());
    }

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
