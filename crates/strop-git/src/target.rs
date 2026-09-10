//! The typed repository boundary (0036 RW8, 0037 DC1b): every Git
//! request names the machine its worktree lives on. A local workdir is
//! openable with libgit2 and local `git`; a remote workdir is bytes on
//! another host and only bounded remote `git` commands can read it; a
//! container workdir is bytes inside a running container and only
//! bounded `docker exec` runs can read it. Keeping the three in one
//! enum — instead of a bare path that could mean any of them — makes
//! "treated a non-local path as local" a type error instead of a bug.
//!
//! The same boundary carries provenance through the memory surfaces:
//! a log row's dive, a commit's file list and a delta's `]f` step all
//! replay the [`RepoTarget`] they were launched with, so a remote or
//! container surface can never answer from the local cwd.

use std::path::{Path, PathBuf};

use strop_workspace::{ContainerId, RemoteEndpoint, RemoteFile};

/// Where a Git query runs. Exactly three real backends exist (0036,
/// 0037); there is deliberately no provider trait behind them.
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
    /// A worktree inside a running container on the local engine. The
    /// workdir names a path *inside* the container — it is never a
    /// valid local path, and no libgit2 handle may be opened against
    /// it. Identity is the canonical 64-hex inspect id: a stopped or
    /// restarted container's reads fail typed at the engine boundary.
    Container {
        container: ContainerId,
        #[serde(with = "strop_core::path_serde")]
        workdir: PathBuf,
    },
}

impl RepoTarget {
    /// The repository root as native bytes — locally openable only for
    /// [`RepoTarget::Local`]; a remote or container workdir names a path
    /// on another filesystem namespace. Callers that need to open it
    /// must match on the variant first.
    pub fn workdir(&self) -> &Path {
        match self {
            Self::Local { workdir } | Self::Remote { workdir, .. } => workdir,
            Self::Container { workdir, .. } => workdir,
        }
    }

    /// The remote-file identity for a repo-relative path (SSH-remote
    /// repositories only): endpoint plus native path, the identity an
    /// open request routes by. Local and container repositories have no
    /// remote file.
    pub fn remote_file(&self, rel: &Path) -> Option<RemoteFile> {
        match self {
            Self::Local { .. } | Self::Container { .. } => None,
            Self::Remote { endpoint, workdir } => {
                RemoteFile::from_path(endpoint.clone(), workdir.join(rel)).ok()
            }
        }
    }

    /// `true` only for an SSH-remote repository. A container repository
    /// is *not* remote in this sense — it has no endpoint and rides the
    /// local engine — but it is not local either: callers deciding "can
    /// I open this path" must not treat `!is_remote()` as local.
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }

    /// `true` for a container repository: not locally openable even
    /// though it shares this machine's engine.
    pub fn is_container(&self) -> bool {
        matches!(self, Self::Container { .. })
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
            Self::Local { .. } | Self::Container { .. } => None,
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

    fn container_target() -> RepoTarget {
        RepoTarget::Container {
            container: ContainerId::canonical("b".repeat(64)).unwrap(),
            workdir: PathBuf::from("/work/src"),
        }
    }

    /// A container repository is neither SSH-remote nor local: no
    /// endpoint, no remote-file identity, and `!is_remote()` must never
    /// read as "openable locally".
    #[test]
    fn container_target_is_neither_remote_nor_local() {
        let target = container_target();
        assert!(target.is_container());
        assert!(!target.is_remote());
        assert_eq!(target.endpoint(), None);
        assert_eq!(target.remote_file(Path::new("a.rs")), None);
        assert_eq!(target.workdir(), Path::new("/work/src"));
        assert_eq!(
            target.rel_of(Path::new("/work/src/lib.rs")),
            Some(PathBuf::from("lib.rs"))
        );
    }

    /// Container identity is the canonical id plus workdir: a different
    /// incarnation id is a different repository.
    #[test]
    fn container_identity_is_id_plus_workdir() {
        let a = container_target();
        let other_id = RepoTarget::Container {
            container: ContainerId::canonical("c".repeat(64)).unwrap(),
            workdir: PathBuf::from("/work/src"),
        };
        assert_ne!(a, other_id);
        assert_ne!(
            a,
            RepoTarget::Local {
                workdir: PathBuf::from("/work/src")
            }
        );
    }

    /// The container variant is additive on the replay wire: its name
    /// is "container", and tapes carrying the pre-container variants
    /// decode unchanged.
    #[test]
    fn serde_round_trips_container_and_decodes_legacy_variants() {
        let target = container_target();
        let text = serde_json::to_string(&target).unwrap();
        assert!(text.contains("\"container\":"), "{text}");
        let back: RepoTarget = serde_json::from_str(&text).unwrap();
        assert_eq!(target, back);

        // Legacy string-path form (pre-versioned path_serde) still decodes.
        let legacy_local: RepoTarget =
            serde_json::from_str(r#"{"local":{"workdir":"/w"}}"#).unwrap();
        assert_eq!(
            legacy_local,
            RepoTarget::Local {
                workdir: PathBuf::from("/w")
            }
        );
        let legacy_remote: RepoTarget = serde_json::from_str(
            r#"{"remote":{"endpoint":"ssh://fixture@box.example:2222","workdir":"/srv/proj"}}"#,
        )
        .unwrap();
        assert_eq!(legacy_remote, remote_target());
    }
}
