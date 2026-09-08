//! The typed repository boundary (0036 RW8): every Git request names
//! the machine its worktree lives on. A local workdir is openable with
//! libgit2 and local `git`; a remote workdir is bytes on another host
//! and only bounded remote `git` commands can read it. Keeping the two
//! in one enum — instead of a bare path that could mean either — makes
//! "treated a remote path as local" a type error instead of a bug.
//!
//! The same boundary carries provenance through the memory surfaces:
//! a log row's dive, a commit's file list and a delta's `]f` step all
//! replay the [`RepoTarget`] they were launched with, so a remote
//! surface can never answer from the local cwd.

use std::path::{Path, PathBuf};

use strop_remote::{RemoteEndpoint, RemoteFile};

/// Where a Git query runs. Exactly two real backends exist (0036);
/// there is deliberately no provider trait behind them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoTarget {
    /// A worktree on this machine: libgit2 and local `git` apply.
    Local {
        #[serde(with = "strop_core::path_serde")]
        workdir: PathBuf,
    },
    /// A worktree on `endpoint`. The workdir names a path on the
    /// remote host — it is never a valid local path, and no libgit2
    /// handle may be opened against it.
    Remote {
        endpoint: RemoteEndpoint,
        #[serde(with = "strop_core::path_serde")]
        workdir: PathBuf,
    },
}

impl RepoTarget {
    /// The repository root as native bytes — locally openable only for
    /// [`RepoTarget::Local`]; callers that need to open it must match
    /// on the variant first.
    pub fn workdir(&self) -> &Path {
        match self {
            Self::Local { workdir } | Self::Remote { workdir, .. } => workdir,
        }
    }

    /// The remote-file identity for a repo-relative path (remote
    /// repositories only): endpoint plus native path, the identity an
    /// open request routes by. A local repository has no remote file.
    pub fn remote_file(&self, rel: &Path) -> Option<RemoteFile> {
        match self {
            Self::Local { .. } => None,
            Self::Remote { endpoint, workdir } => {
                RemoteFile::from_path(endpoint.clone(), workdir.join(rel)).ok()
            }
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }

    /// Repo-relative path for a path inside this repository — the
    /// remote flavor strips the *remote* workdir. `None` is the typed
    /// refusal for a path that is not inside the repository at all.
    pub fn rel_of(&self, path: &Path) -> Option<PathBuf> {
        path.strip_prefix(self.workdir())
            .ok()
            .map(|rel| rel.to_path_buf())
    }

    /// The absolute path of a repo-relative path *on the machine this
    /// repository lives on*.
    pub fn abs_of(&self, rel: &Path) -> PathBuf {
        self.workdir().join(rel)
    }

    pub fn endpoint(&self) -> Option<&RemoteEndpoint> {
        match self {
            Self::Local { .. } => None,
            Self::Remote { endpoint, .. } => Some(endpoint),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_target() -> RepoTarget {
        RepoTarget::Remote {
            endpoint: RemoteEndpoint::parse("ssh://fixture@box.example:2222").unwrap(),
            workdir: PathBuf::from("/srv/proj"),
        }
    }

    /// The boundary is identity: equal targets mean the same repository
    /// on the same machine; a different port is a different repository.
    #[test]
    fn remote_identity_is_endpoint_plus_workdir() {
        let a = remote_target();
        let same = RepoTarget::Remote {
            endpoint: RemoteEndpoint::parse("ssh://fixture@box.example:2222").unwrap(),
            workdir: PathBuf::from("/srv/proj"),
        };
        let other_port = RepoTarget::Remote {
            endpoint: RemoteEndpoint::parse("ssh://fixture@box.example:2223").unwrap(),
            workdir: PathBuf::from("/srv/proj"),
        };
        let other_dir = RepoTarget::Remote {
            endpoint: RemoteEndpoint::parse("ssh://fixture@box.example:2222").unwrap(),
            workdir: PathBuf::from("/other"),
        };
        assert_eq!(a, same);
        assert_ne!(a, other_port);
        assert_ne!(a, other_dir);
        assert_ne!(
            a,
            RepoTarget::Local {
                workdir: PathBuf::from("/srv/proj")
            }
        );
    }

    /// A remote path is never relative to the local machine: rel_of
    /// strips the REMOTE workdir, and the same spelling stays a local
    /// path under the local variant — the two never interchange.
    #[test]
    fn rel_of_strips_the_owning_workdir() {
        let target = remote_target();
        assert_eq!(
            target.rel_of(Path::new("/srv/proj/src/main.rs")),
            Some(PathBuf::from("src/main.rs"))
        );
        assert_eq!(target.rel_of(Path::new("/home/me/src/main.rs")), None);
    }

    /// remote_file rebuilds the canonical open identity: same endpoint,
    /// native path under the remote workdir — what a dive's source open
    /// routes to Main with.
    #[test]
    fn remote_file_carries_endpoint_and_native_path() {
        let target = remote_target();
        let file = target.remote_file(Path::new("src/a b.rs")).unwrap();
        assert_eq!(
            file.endpoint(),
            &RemoteEndpoint::parse("ssh://fixture@box.example:2222").unwrap()
        );
        assert_eq!(file.path(), Path::new("/srv/proj/src/a b.rs"));
        assert!(target.remote_file(Path::new("x")).is_some());
    }

    /// Local repositories have no remote identity and no endpoint —
    /// refusal, not a local stand-in.
    #[test]
    fn local_target_has_no_remote_identity() {
        let target = RepoTarget::Local {
            workdir: PathBuf::from("/w"),
        };
        assert_eq!(target.endpoint(), None);
        assert!(!target.is_remote());
        assert_eq!(target.remote_file(Path::new("a.rs")), None);
    }

    /// Provenance survives the replay wire: serde round-trips a remote
    /// target back to the same endpoint and workdir bytes.
    #[test]
    fn serde_round_trips_remote_provenance() {
        let target = remote_target();
        let text = serde_json::to_string(&target).unwrap();
        let back: RepoTarget = serde_json::from_str(&text).unwrap();
        assert_eq!(target, back);
    }
}
