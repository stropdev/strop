//! Local declaration authority. Only workers touch native evidence. A warm
//! proposal and an opened file use the same boundary/config/trust resolution.
use super::{AttachKey, ServerId, ServerSpec, WorkerId};
use rustix::fs::{openat2, Mode, OFlags, ResolveFlags, CWD};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use strop_core::{id::DocumentId, worker::CancelToken};
use strop_lsp::languages::Languages;
use strop_picker::{DeclaredBoundary, ProjectCatalog};

#[derive(Default)]
pub(in crate::editor::lsp) struct Placements {
    pub roots: Vec<PathBuf>,
    pub jobs: HashMap<WorkerId, Job>,
    pub servers: HashMap<ServerId, Witness>,
    pub declaration_servers: HashSet<ServerId>,
    pub refused: HashSet<AttachKey>,
    /// A verified file, not a prefix grant for every name under a warm root.
    pub documents: HashMap<DocumentId, (PathBuf, ServerId)>,
    pub retired: Arc<AtomicBool>,
}
pub(in crate::editor::lsp) struct Job {
    pub declared: bool,
    pub document: Option<DocumentId>,
    pub witness: Witness,
}
pub(in crate::editor::lsp) type Witness = Arc<OnceLock<Instance>>;

/// Reuse requires the same native cwd and actual resolved launch settings,
/// not merely another server with the same registry name.
#[derive(PartialEq)]
pub(in crate::editor::lsp) struct Instance {
    dir: (u64, u64),
    name: String,
    command: String,
    args: Vec<String>,
    options: Option<serde_json::Value>,
}
pub(crate) struct Request {
    pub(in crate::editor::lsp) declarations: Arc<[DeclaredBoundary]>,
    pub(in crate::editor::lsp) index: Option<usize>,
    pub(in crate::editor::lsp) target: PathBuf,
    pub(in crate::editor::lsp) warm: bool,
    pub(in crate::editor::lsp) retired: Arc<AtomicBool>,
    pub(in crate::editor::lsp) witness: Witness,
    pub(in crate::editor::lsp) candidates: Vec<(ServerId, Witness)>,
}

fn denied(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason)
}
fn open_below(anchor: &File, path: &Path, flags: OFlags) -> io::Result<File> {
    let path = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };
    Ok(openat2(
        anchor,
        path,
        flags | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
    )?
    .into())
}
fn open_directory(path: &Path) -> io::Result<File> {
    Ok(openat2(
        CWD,
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS,
    )?
    .into())
}

pub(super) struct Resolved {
    pub languages: Languages,
    pub root: PathBuf,
    pub pin: Pin,
}
pub(super) struct Pin {
    pub project: PathBuf,
    cwd: File,
    identity: (u64, u64),
    target: File,
    target_path: CString,
}
impl Request {
    pub(super) fn resolve(
        &self,
        workspace: &Path,
        directory: &Path,
        language: &str,
        xdg: Option<&Path>,
        token: &CancelToken,
    ) -> io::Result<Option<Resolved>> {
        let cancelled = || token.is_cancelled() || self.retired.load(Ordering::Acquire);
        let boundary = self.index.map(|index| &self.declarations[index]);
        let declarations = self.index.map_or(&[][..], |i| &self.declarations[i..=i]);
        let owner =
            ProjectCatalog::observe_target(workspace, &self.target, declarations, &cancelled);
        if boundary.is_none() && !self.warm && owner.as_ref().is_none_or(|p| !p.discovered()) {
            // Ordinary loose/new files keep the config/Git/file-directory
            // resolver. An ignored target cannot adopt a warmed project.
            if owner.is_none() && !self.candidates.is_empty() {
                return Err(denied("workspace discovery did not observe this target"));
            }
            return Ok(None);
        }
        let owner =
            owner.ok_or_else(|| denied("workspace discovery did not observe this target"))?;
        let project = workspace.join(&owner.root);
        let (member_path, member) = match boundary {
            Some(declared) => (
                workspace.join(declared.path()),
                declared.open_directory(workspace)?,
            ),
            None => (project.clone(), open_directory(&project)?),
        };
        let relative = self
            .target
            .strip_prefix(&member_path)
            .map_err(|_| denied("outside declared project"))?;
        let target = open_below(&member, relative, OFlags::PATH)?;
        let ty = target.metadata()?.file_type();
        if (self.warm && !ty.is_dir()) || (!self.warm && !ty.is_file()) {
            return Err(denied("project target changed type"));
        }
        // Both discoveries start at exactly the observed project; a cold
        // target may belong to a deeper marker inside a declaration.
        if !project.starts_with(&member_path)
            || (self.warm && (project != self.target || !owner.proposes_language(language)))
        {
            return Err(denied(
                "project or warm language no longer matches discovery",
            ));
        }
        let pin_dir = |path: &Path| match path.strip_prefix(&member_path) {
            Ok(rel) => open_below(&member, rel, OFlags::PATH | OFlags::DIRECTORY),
            Err(_) => open_directory(path),
        };
        // Preserve the existing nearest project layer. Read its bytes from
        // the same directory handle that will anchor its trusted command.
        let mut layer = None;
        for ancestor in directory.ancestors() {
            if cancelled() {
                return Err(denied("project generation retired"));
            }
            let dir = pin_dir(ancestor)?;
            let relative = Path::new(".strop/languages.toml");
            match open_below(&dir, relative, OFlags::RDONLY | OFlags::NONBLOCK) {
                Ok(file) if file.metadata()?.is_file() => {
                    if boundary.is_some() && self.warm && ancestor != project {
                        return Err(denied("project configuration belongs to a different root"));
                    }
                    let languages = Languages::load_opened(xdg, &ancestor.join(relative), file);
                    layer = Some((languages, ancestor.to_owned(), dir));
                    break;
                }
                Ok(_) => return Err(denied("project configuration is not a regular file")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(error),
            }
        }
        let (languages, root, cwd) = match layer {
            Some(layer) => layer,
            None => {
                // A marker supplies ordinary discovery's directory fallback;
                // it doesn't override the established Git root (#243).
                let root = if boundary.is_none() {
                    strop_lsp::registry::workspace_root(&project, workspace)
                } else {
                    project
                };
                let dir = pin_dir(&root)?;
                (Languages::load(xdg, None), root, dir)
            }
        };
        let meta = cwd.metadata()?;
        let pin = Pin {
            project: owner.root,
            cwd,
            identity: (meta.dev(), meta.ino()),
            target,
            target_path: CString::new(self.target.as_os_str().as_bytes())?,
        };
        pin.check(&root)?;
        Ok(Some(Resolved {
            languages,
            root,
            pin,
        }))
    }

    /// The runtime launches later than discovery. Check the manifest's
    /// actual bytes/directory witness and exact eligible target there, not
    /// just a flag set when a watcher eventually reports the change.
    pub(super) fn launch_check(
        &self,
        workspace: &Path,
        project: &Path,
        language: &'static str,
    ) -> impl FnOnce() -> io::Result<()> + Send + 'static {
        let (workspace, project) = (workspace.to_owned(), project.to_owned());
        let (declarations, index, target) =
            (self.declarations.clone(), self.index, self.target.clone());
        let (retired, warm) = (self.retired.clone(), self.warm);
        move || {
            let cancelled = || retired.load(Ordering::Acquire);
            if cancelled() {
                return Err(denied("project generation retired"));
            }
            let declarations = index.map_or(&[][..], |i| &declarations[i..=i]);
            let _member = declarations
                .first()
                .map(|d| d.open_directory(&workspace))
                .transpose()?;
            let eligible =
                ProjectCatalog::observe_target(&workspace, &target, declarations, &cancelled)
                    .is_some_and(|owner| {
                        owner.root == project
                            && (index.is_some() || owner.discovered())
                            && (!warm || owner.proposes_language(language))
                    });
            if !eligible {
                return Err(denied("workspace target no longer has its observed owner"));
            }
            Ok(())
        }
    }

    pub(super) fn reusable(&self, pin: &Pin, spec: &ServerSpec<'_>) -> Option<ServerId> {
        for (server, candidate) in &self.candidates {
            if candidate.get().is_some_and(|i| {
                i.dir == pin.identity
                    && i.name == spec.name
                    && i.command == spec.command
                    && i.args == spec.args
                    && i.options.as_ref() == spec.init_options
            }) {
                return Some(*server);
            }
        }
        self.witness.get_or_init(|| Instance {
            dir: pin.identity,
            name: spec.name.to_owned(),
            command: spec.command.to_owned(),
            args: spec.args.to_owned(),
            options: spec.init_options.cloned(),
        });
        None
    }
}

impl Pin {
    pub(super) fn probe_root(&self) -> PathBuf {
        use std::os::fd::AsRawFd;
        format!("/proc/self/fd/{}", self.cwd.as_raw_fd()).into()
    }
    fn check(&self, root: &Path) -> io::Result<()> {
        let native = open_directory(root)?.metadata()?;
        if (native.dev(), native.ino()) != self.identity {
            return Err(denied("project directory was replaced"));
        }
        same_file(&self.target_path, &self.target)
    }
    pub(super) fn command(
        self,
        spec: &ServerSpec<'_>,
        root: &Path,
        retired: Arc<AtomicBool>,
    ) -> io::Result<Command> {
        let root = CString::new(root.as_os_str().as_bytes())?;
        let mut command = Command::new(spec.command);
        command.args(spec.args);
        // SAFETY: only raw filesystem syscalls and atomics after fork. All
        // C strings and file handles are prepared before entering the child.
        // fchdir uses the retained fd even if the logical path changes after
        // the comparison: exec never traverses a substituted cwd symlink.
        unsafe {
            command.pre_exec(move || {
                if retired.load(Ordering::Acquire) {
                    return Err(rustix::io::Errno::STALE.into());
                }
                same_file(&root, &self.cwd)?;
                same_file(&self.target_path, &self.target)?;
                rustix::process::fchdir(&self.cwd)?;
                Ok(())
            });
        }
        Ok(command)
    }
}
fn same_file(path: &CString, expected: &File) -> io::Result<()> {
    let fd = openat2(
        CWD,
        path.as_c_str(),
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS,
    )?;
    let a = rustix::fs::fstat(fd)?;
    let b = rustix::fs::fstat(expected)?;
    if a.st_dev == b.st_dev && a.st_ino == b.st_ino {
        Ok(())
    } else {
        Err(rustix::io::Errno::STALE.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn tree() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".strop")).unwrap();
        fs::create_dir_all(temp.path().join("member/src")).unwrap();
        fs::write(
            temp.path().join(".strop/workspace.toml"),
            "version=1\n[[project]]\npath='member'\nlanguages=['rust']\n",
        )
        .unwrap();
        fs::write(temp.path().join("member/src/mod.rs"), "").unwrap();
        temp
    }

    fn request(root: &Path, target: &str, warm: bool) -> Request {
        let declarations = strop_picker::read_workspace_declarations(root).unwrap();
        Request {
            declarations,
            index: Some(0),
            target: root.join(target),
            warm,
            retired: Arc::default(),
            witness: Arc::default(),
            candidates: vec![],
        }
    }

    fn resolve(root: &Path, r: &Request) -> io::Result<Resolved> {
        let (token, _handle) = CancelToken::standalone();
        let dir = if r.warm {
            r.target.as_path()
        } else {
            r.target.parent().unwrap()
        };
        r.resolve(root, dir, "rust", None, &token)?
            .ok_or_else(|| denied("not a project"))
    }

    #[test]
    fn warm_and_cold_agree_unless_a_more_specific_owner_or_ignore_applies() {
        let temp = tree();
        let root = temp.path();
        let warm = request(root, "member", true);
        let cold = request(root, "member/src/mod.rs", false);
        assert_eq!(resolve(root, &warm).unwrap().root, root.join("member"));
        assert_eq!(resolve(root, &cold).unwrap().root, root.join("member"));
        fs::write(root.join("member/src/Cargo.toml"), "").unwrap();
        assert_eq!(resolve(root, &cold).unwrap().root, root.join("member/src"));
        fs::write(root.join("member/src/.ignore"), "mod.rs\n").unwrap();
        assert!(resolve(root, &cold).is_err());
    }

    #[test]
    fn marker_only_cold_open_reuses_warm_owner_but_not_for_a_nested_or_ignored_file() {
        let temp = tree();
        let root = temp.path();
        fs::remove_file(root.join(".strop/workspace.toml")).unwrap();
        fs::write(root.join("member/Cargo.toml"), "").unwrap();
        let mut warm = request(root, "member", true);
        warm.index = None;
        let observed = resolve(root, &warm).unwrap();
        let spec = strop_lsp::registry::for_extension(".rs", &observed.languages).unwrap();
        assert!(warm.reusable(&observed.pin, &spec).is_none());
        let mut cold = request(root, "member/src/mod.rs", false);
        cold.index = None;
        let id = ServerId::new(9);
        cold.candidates.push((id, warm.witness.clone()));
        let before = resolve(root, &cold).unwrap();
        assert_eq!(before.root, observed.root);
        assert_eq!(cold.reusable(&before.pin, &spec), Some(id));
        fs::write(root.join("member/src/Cargo.toml"), "").unwrap();
        let after = resolve(root, &cold).unwrap();
        assert_eq!(after.root, root.join("member/src"));
        assert!(cold.reusable(&after.pin, &spec).is_none());
        fs::write(root.join("member/src/.ignore"), "mod.rs\n").unwrap();
        assert!(resolve(root, &cold).is_err());
    }

    #[test]
    fn ancestor_config_owns_cold_command_and_cannot_be_borrowed_for_warm_child() {
        let temp = tree();
        let root = temp.path();
        fs::write(
            root.join(".strop/languages.toml"),
            "[language-server.rust-analyzer]\ncommand='./bin/ra'\n",
        )
        .unwrap();
        assert!(resolve(root, &request(root, "member", true)).is_err());
        let resolved = resolve(root, &request(root, "member/src/mod.rs", false)).unwrap();
        assert_eq!(resolved.root, root);
        let spec = strop_lsp::registry::for_extension(".rs", &resolved.languages).unwrap();
        assert!(spec.project_executable);
        assert!(super::super::trust_refusal(&spec, None, &resolved.root).is_some());
    }

    #[test]
    fn launch_check_refuses_manifest_removal_without_waiting_for_notify() {
        let temp = tree();
        let root = temp.path();
        let request = request(root, "member", true);
        let resolved = resolve(root, &request).unwrap();
        let check = request.launch_check(root, &resolved.pin.project, "rust");
        fs::remove_file(root.join(".strop/workspace.toml")).unwrap();
        assert!(check().is_err());
    }

    #[test]
    fn command_rechecks_the_native_directory_at_actual_exec() {
        let temp = tree();
        let root = temp.path();
        let request = request(root, "member", true);
        let resolved = resolve(root, &request).unwrap();
        let spec = ServerSpec {
            name: "pwd",
            command: "/bin/pwd",
            args: &[],
            init_options: None,
            project_executable: false,
            install_hint: None,
        };
        let mut command = resolved
            .pin
            .command(&spec, &resolved.root, request.retired.clone())
            .unwrap();
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            format!("{}\n", root.join("member").display()).as_bytes()
        );
        fs::rename(root.join("member"), root.join("former")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.join("member")).unwrap();
        assert!(
            command.output().is_err(),
            "substituted directory must not execute"
        );
    }
}
