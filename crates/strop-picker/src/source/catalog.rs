//! The project catalog (0063 §2): one bounded scan of the opened scope
//! discovers repository and language-project boundaries — nested Git
//! repositories, worktrees (`.git` file), monorepo subprojects (marker
//! files), non-Git projects and the loose scope itself. Files, Search and
//! workspace symbols share this one catalog; no surface re-walks or
//! special-cases directory names.
//!
//! Discovery is advisory structure, never authority: a project the scan
//! misses (inside an ignored tree, past the bounds) leaves its `repo:`
//! atoms undecidable — they admit, overfetching — because a lost line is
//! worse than an extra candidate (0063 §4).

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::selection::{self, SelectionPolicy};

const DISCOVERY_PATH_LIMIT: usize = 50_000;
const DISCOVERY_BYTES: usize = selection::PATH_BYTES;
const WARM_LIMIT: usize = 256;
const WARM_BYTES: usize = 256 * 1024;

/// Multiple marker families can coexist. package.json alone cannot choose
/// between JavaScript and TypeScript; preserve the existing cold status.
const FAMILIES: &[(&[&str], Option<&str>)] = &[
    (&["Cargo.toml"], Some("rust")),
    (&["pyproject.toml", "setup.py"], Some("python")),
    (&["CMakeLists.txt"], Some("cpp")),
    (&["go.mod"], Some("go")),
    (&["package.json"], None),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitBoundary {
    Repository,
    Worktree,
}

mod declarations;
pub(super) use declarations::declarations_fit;
pub use declarations::{read_workspace_declarations, DeclaredBoundary};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WarmHint {
    Language(String),
    AmbiguousPackage,
}

/// A candidate, never a binding. The existing attach route still owns
/// registry resolution, trust, capabilities and server lifetime.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WarmProject {
    #[serde(with = "strop_core::path_serde")]
    pub root: PathBuf,
    pub hint: WarmHint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Native identity relative to the workspace incarnation owning this scan.
    pub root: PathBuf,
    pub git: Option<GitBoundary>,
    markers: u8,
    pub declared: bool,
    /// None preserves inference; Some([]) disables speculative attach only.
    languages: Option<Vec<String>>,
}

impl Project {
    /// The pinned scope itself always has a catalog record; it is not a
    /// project marker unless the walk observed actual project evidence.
    pub fn discovered(&self) -> bool {
        self.git.is_some() || self.markers != 0
    }

    fn hints(&self) -> impl Iterator<Item = WarmHint> + '_ {
        self.languages
            .iter()
            .flatten()
            .cloned()
            .map(WarmHint::Language)
            .chain(
                FAMILIES
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        self.languages.is_none() && self.markers & (1 << index) != 0
                    })
                    .map(|(_, (_, language))| {
                        language.map_or(WarmHint::AmbiguousPackage, |id| {
                            WarmHint::Language(id.into())
                        })
                    }),
            )
    }

    pub fn proposes_language(&self, language: &str) -> bool {
        match &self.languages {
            Some(ids) => ids.iter().any(|id| id == language),
            None => FAMILIES
                .iter()
                .enumerate()
                .any(|(i, (_, hint))| self.markers & (1 << i) != 0 && *hint == Some(language)),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProjectCatalog {
    scope: PathBuf,
    projects: Vec<Project>,
    /// Each observed directory's proved owner. Sorted once, allocation-free
    /// lookup for every hit; unvisited/ignored directories have no answer.
    directories: Vec<(PathBuf, usize)>,
    pub truncated: bool,
    payload_bytes: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CatalogStats {
    pub visited: usize,
}

impl ProjectCatalog {
    /// Watch hints carrying one of the native boundary markers affect
    /// semantic ownership, even when the observed document didn't change.
    pub fn is_marker(name: &OsStr) -> bool {
        name == ".git"
            || FAMILIES
                .iter()
                .any(|(names, _)| names.iter().any(|marker| name == *marker))
    }

    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.iter()
    }

    pub fn retained_bytes(&self) -> usize {
        self.payload_bytes
            + self.scope.capacity()
            + self.projects.capacity() * size_of::<Project>()
            + self.directories.capacity() * size_of::<(PathBuf, usize)>()
            + size_of::<Self>()
    }

    pub fn discover(scope: &Path, cancelled: &impl Fn() -> bool) -> Self {
        Self::scan(scope, cancelled, &[]).0
    }

    /// Re-observe a single actual regular file or directory with the same
    /// ignore/protected-path walk and deepest-boundary decision as a catalog.
    /// An ignored file in an observed directory is not an observed file.
    pub fn observe_target(
        scope: &Path,
        target: &Path,
        declarations: &[DeclaredBoundary],
        cancelled: &impl Fn() -> bool,
    ) -> Option<Project> {
        let relative = target.strip_prefix(scope).ok()?;
        if relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return None;
        }
        let mut observer = Self::observer(scope, declarations);
        for result in selection::ancestors_to(scope, target).build() {
            if cancelled() {
                return None;
            }
            let entry = result.ok()?;
            if entry.error().is_some() {
                return None;
            }
            let ty = entry.file_type()?;
            if ty.is_dir() {
                observer.visit_depth(entry.path().strip_prefix(scope).ok()?, entry.depth());
            }
            if entry.path() == target {
                if !ty.is_dir() && !ty.is_file() {
                    return None;
                }
                // This walk has just one branch. Its latest visited directory
                // is the target or its parent; the observer already recorded
                // the kernel's owner for it. No sort or copy of the catalog.
                let mut catalog = observer.catalog;
                if catalog.truncated {
                    return None;
                }
                let index = catalog.directories.last()?.1;
                return Some(catalog.projects.swap_remove(index));
            }
        }
        None
    }

    /// Always observe directories afresh: a parent's mtime says nothing
    /// about its grandchildren. Declaration binding uses precisely this walk;
    /// it cannot add paths to file or text selection.
    pub fn scan(
        scope: &Path,
        cancelled: &impl Fn() -> bool,
        declarations: &[DeclaredBoundary],
    ) -> (Self, CatalogStats) {
        let mut observer = Self::observer(scope, declarations);
        let mut exhausted = true;
        let mut builder = selection::builder(
            scope,
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        builder.sort_by_file_name(OsStr::cmp);
        for (count, result) in builder.build().enumerate() {
            if cancelled() || count >= DISCOVERY_PATH_LIMIT {
                exhausted = false;
                break;
            }
            let Ok(entry) = result else {
                exhausted = false;
                break;
            };
            if entry.error().is_some() {
                exhausted = false;
                break;
            }
            if !entry.file_type().is_some_and(|ty| ty.is_dir()) {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(scope) else {
                exhausted = false;
                break;
            };
            observer.visit_depth(relative, entry.depth());
            if observer.catalog.truncated {
                break;
            }
        }
        observer.finish(exhausted)
    }

    /// Feed the same directories from an already running compatible selection
    /// walk. Not public: a caller cannot turn arbitrary paths into evidence.
    pub(super) fn observer<'a>(
        scope: &'a Path,
        declarations: &'a [DeclaredBoundary],
    ) -> CatalogObserver<'a> {
        CatalogObserver {
            probe: PathBuf::new(),
            declarations,
            branch: Branch::default(),
            catalog: Self {
                scope: scope.to_owned(),
                ..Self::default()
            },
        }
    }

    /// Only paths in observed directories have a known project. A partial
    /// scan may have missed closer boundaries, so it cannot decide a filter.
    pub fn project_for(&self, path: &Path) -> Option<&Project> {
        if self.truncated
            || path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return None;
        }
        let find = |dir: &Path| {
            self.directories
                .binary_search_by(|(p, _)| p.as_path().cmp(dir))
                .ok()
        };
        let index = find(path).or_else(|| find(path.parent()?))?;
        self.projects.get(self.directories[index].1)
    }

    pub fn repo_matches(&self, path: &Path, name: &str) -> Option<bool> {
        let project = self.project_for(path)?;
        let name_at_root = project
            .root
            .file_name()
            .or_else(|| self.scope.file_name())
            .unwrap_or(self.scope.as_os_str());
        Some(name_at_root == OsStr::new(name))
    }

    /// Bound the independent delivery/attach queue, not file eligibility.
    /// The caller reports omitted warm coverage, never an empty result.
    pub fn warm_projects(&self) -> (Vec<WarmProject>, bool) {
        let mut warm = Vec::new();
        // Include spare vector slots without charging payload twice. Both
        // Vec and the receiving VecDeque reuse this bounded allocation.
        let mut bytes = WARM_LIMIT * size_of::<WarmProject>();
        for project in self.projects() {
            for hint in project.hints() {
                let root = self.scope.join(&project.root);
                let language_bytes = match &hint {
                    WarmHint::Language(id) => id.capacity(),
                    WarmHint::AmbiguousPackage => 0,
                };
                bytes += root.capacity() + language_bytes;
                if warm.len() == WARM_LIMIT || bytes > WARM_BYTES {
                    return (warm, true);
                }
                warm.push(WarmProject { root, hint });
            }
        }
        (warm, self.truncated)
    }
}

fn language_bytes(list: Option<&Vec<String>>) -> usize {
    list.map_or(0, |ids| {
        ids.capacity() * size_of::<String>() + ids.iter().map(String::capacity).sum::<usize>()
    })
}

pub(super) struct CatalogObserver<'a> {
    probe: PathBuf,
    declarations: &'a [DeclaredBoundary],
    branch: Branch,
    catalog: ProjectCatalog,
}

impl CatalogObserver<'_> {
    /// A native relative directory from the selection walk, including the
    /// empty root. Visits are parent-before-child in depth-first order.
    pub(super) fn visit(&mut self, dir: &Path) {
        self.visit_depth(dir, dir.components().count());
    }

    fn visit_depth(&mut self, relative: &Path, depth: usize) {
        let catalog = &mut self.catalog;
        if catalog.truncated {
            return;
        }
        if catalog.directories.len() >= DISCOVERY_PATH_LIMIT {
            catalog.truncated = true;
            return;
        }
        self.probe.clear();
        self.probe.push(&catalog.scope);
        self.probe.push(relative);
        let declared = self
            .declarations
            .iter()
            .find(|d| d.path() == relative && d.observed(&self.probe));
        self.probe.push(".git");
        let Ok(project) = inspect(&mut self.probe, relative, declared) else {
            catalog.truncated = true;
            return;
        };
        let previous_bytes = catalog.payload_bytes;
        let project_index = project.map(|project| {
            let index = catalog.projects.len();
            catalog.payload_bytes +=
                project.root.capacity() + language_bytes(project.languages.as_ref());
            catalog.projects.push(project);
            index
        });
        let owner = self.branch.visit(
            depth,
            relative.file_name().unwrap_or_default(),
            project_index,
        );
        if let Some(owner) = owner {
            let path = relative.to_owned();
            catalog.payload_bytes += path.capacity();
            catalog.directories.push((path, owner));
        }
        if catalog.retained_bytes() > DISCOVERY_BYTES {
            if owner.is_some() {
                catalog.directories.pop();
            }
            if project_index.is_some() {
                catalog.projects.pop();
            }
            catalog.payload_bytes = previous_bytes;
            catalog.projects.shrink_to_fit();
            catalog.directories.shrink_to_fit();
            catalog.truncated = true;
        }
    }

    pub(super) fn finish(mut self, exhausted: bool) -> (ProjectCatalog, CatalogStats) {
        self.catalog.truncated |= !exhausted;
        self.catalog
            .directories
            .sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let stats = CatalogStats {
            visited: self.catalog.directories.len(),
        };
        (self.catalog, stats)
    }
}

/// Scan-local exact interning, never hashes-as-identities. Sibling components
/// with the same spelling have the same token; it is the *whole prefix*
/// selected by the kernel that identifies the boundary.
#[derive(Default)]
struct Branch {
    components: HashMap<OsString, u64>,
    roots: Vec<Vec<u64>>,
    by_depth: Vec<Option<usize>>,
    path: Vec<u64>,
}

impl Branch {
    fn visit(&mut self, depth: usize, name: &OsStr, project: Option<usize>) -> Option<usize> {
        self.path.truncate(depth.saturating_sub(1));
        if depth > 0 {
            let id = match self.components.get(name) {
                Some(id) => *id,
                None => {
                    let next = self.components.len() as u64;
                    self.components.insert(name.to_owned(), next);
                    next
                }
            };
            self.path.push(id);
        }
        while self.roots.last().is_some_and(|r| r.len() >= depth) {
            self.roots.pop();
        }
        self.by_depth.resize(depth + 1, None);
        self.by_depth[depth] = project;
        if project.is_some() {
            self.roots.push(self.path.clone());
        }
        let owner = strop_kernel::boundary::deepest(&self.path, &self.roots)?;
        self.by_depth[owner]
    }
}

fn file_type(path: &Path) -> std::io::Result<Option<std::fs::FileType>> {
    match path.symlink_metadata() {
        Ok(metadata) => Ok(Some(metadata.file_type())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn inspect(
    probe: &mut PathBuf,
    relative: &Path,
    declared: Option<&DeclaredBoundary>,
) -> std::io::Result<Option<Project>> {
    let git = file_type(probe)?.and_then(|ty| {
        if ty.is_dir() {
            Some(GitBoundary::Repository)
        } else if ty.is_file() {
            Some(GitBoundary::Worktree)
        } else {
            None
        }
    });
    let mut markers = 0;
    for (index, (names, _)) in FAMILIES.iter().enumerate() {
        for name in *names {
            probe.set_file_name(name);
            if file_type(probe)?.is_some_and(|ty| ty.is_file()) {
                markers |= 1 << index;
                break;
            }
        }
    }
    Ok(
        (relative.as_os_str().is_empty() || git.is_some() || markers != 0 || declared.is_some())
            .then(|| Project {
                root: relative.to_owned(),
                git,
                markers,
                declared: declared.is_some(),
                languages: declared.and_then(|d| d.languages().map(<[_]>::to_vec)),
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::ffi::OsStrExt};

    fn owner<'a>(catalog: &'a ProjectCatalog, file: &str) -> &'a Path {
        &catalog.project_for(Path::new(file)).unwrap().root
    }

    fn declaration(root: &str, ids: Option<&[&str]>) -> DeclaredBoundary {
        DeclaredBoundary::new(
            root.into(),
            ids.map(|ids| ids.iter().map(|&s| s.into()).collect()),
        )
        .unwrap()
    }

    #[test]
    fn multilingual_repositories_worktrees_and_same_basename_siblings() {
        let temp = tempfile::tempdir().unwrap();
        let scope = temp.path();
        for dir in [
            "team-a/service/.git",
            "team-a/service/pkg",
            "team-b/service",
            "team-a/service-other",
            "loose",
        ] {
            fs::create_dir_all(scope.join(dir)).unwrap();
        }
        for marker in [
            "Cargo.toml",
            "team-a/service/CMakeLists.txt",
            "team-a/service/go.mod",
            "team-a/service/pkg/pyproject.toml",
            "team-a/service/pkg/setup.py",
            "team-b/service/.git",
            "team-b/service/go.mod",
        ] {
            fs::write(scope.join(marker), "").unwrap();
        }
        let catalog = ProjectCatalog::discover(scope, &|| false);
        assert!(!catalog.truncated);
        assert_eq!(
            owner(&catalog, "team-a/service/main.cpp"),
            Path::new("team-a/service")
        );
        assert_eq!(
            owner(&catalog, "team-b/service/main.go"),
            Path::new("team-b/service")
        );
        assert_eq!(
            owner(&catalog, "team-a/service/pkg/m.py"),
            Path::new("team-a/service/pkg")
        );
        assert_eq!(owner(&catalog, "team-a/service-other/note"), Path::new(""));
        assert_eq!(owner(&catalog, "loose/note"), Path::new(""));
        assert!(catalog.project_for(&scope.join("outside")).is_none());
        assert!(catalog
            .project_for(Path::new("team-a/service/../x"))
            .is_none());
        let project = catalog
            .project_for(Path::new("team-a/service/main.cpp"))
            .unwrap();
        assert_eq!(project.git, Some(GitBoundary::Repository));
        assert_eq!(
            catalog
                .project_for(Path::new("team-b/service/main.go"))
                .unwrap()
                .git,
            Some(GitBoundary::Worktree)
        );
        let (warm, capped) = catalog.warm_projects();
        assert!(!capped);
        let candidates: Vec<_> = warm
            .into_iter()
            .map(|p| {
                let WarmHint::Language(id) = p.hint else {
                    panic!("unexpected ambiguous marker")
                };
                (p.root.strip_prefix(scope).unwrap().to_owned(), id)
            })
            .collect();
        assert_eq!(
            candidates,
            [
                ("".into(), "rust".into()),
                ("team-a/service".into(), "cpp".into()),
                ("team-a/service".into(), "go".into()),
                ("team-a/service/pkg".into(), "python".into()),
                ("team-b/service".into(), "go".into()),
            ]
        );
    }

    #[test]
    fn declared_and_discovered_boundaries_share_ownership_not_walk_authority() {
        let temp = tempfile::tempdir().unwrap();
        let scope = temp.path();
        let outside = tempfile::tempdir().unwrap();
        for dir in [
            ".git",
            "mixed/child",
            "markerless",
            "cold",
            "ignored",
            "loose",
        ] {
            fs::create_dir_all(scope.join(dir)).unwrap();
        }
        for file in [
            "mixed/go.mod",
            "mixed/child/Cargo.toml",
            "cold/CMakeLists.txt",
        ] {
            fs::write(scope.join(file), "").unwrap();
        }
        fs::write(scope.join(".gitignore"), "ignored/\n").unwrap();
        std::os::unix::fs::symlink(outside.path(), scope.join("link")).unwrap();
        let declarations = [
            declaration("mixed", Some(&["go", "rust", "unknown", "go"])),
            declaration("mixed/child", None),
            declaration("markerless", None),
            declaration("cold", Some(&[])),
            declaration("ignored", Some(&["go"])),
            declaration(".git", None),
            declaration("link", Some(&["rust"])),
        ];
        for invalid in [
            "",
            ".",
            "./mixed",
            "mixed/./child",
            "mixed/../child",
            "/outside",
        ] {
            assert!(DeclaredBoundary::new(invalid.into(), None).is_none());
        }
        let catalog = ProjectCatalog::scan(scope, &|| false, &declarations).0;
        assert_eq!(
            owner(&catalog, "mixed/child/main.rs"),
            Path::new("mixed/child")
        );
        assert_eq!(owner(&catalog, "markerless/n"), Path::new("markerless"));
        assert_eq!(owner(&catalog, "loose/n"), Path::new(""));
        let mixed = catalog.project_for(Path::new("mixed/a.go")).unwrap();
        assert!(mixed.declared);
        assert_eq!(
            mixed.hints().collect::<Vec<_>>(),
            ["go", "rust", "unknown"].map(|s| WarmHint::Language(s.into()))
        );
        assert_eq!(
            catalog
                .projects()
                .filter(|p| p.root == Path::new("mixed"))
                .count(),
            1
        );
        assert_eq!(catalog.warm_projects().0.len(), 4);
        for path in [".git/x", "ignored/x", "link/x", "absent/x"] {
            assert_eq!(catalog.repo_matches(Path::new(path), "mixed"), None);
        }
        fs::remove_dir(scope.join("markerless")).unwrap();
        std::os::unix::fs::symlink(outside.path(), scope.join("markerless")).unwrap();
        let rebound = ProjectCatalog::scan(scope, &|| false, &declarations).0;
        assert!(rebound.project_for(Path::new("markerless/n")).is_none());
    }

    #[test]
    fn native_control_names_and_lossy_collisions_are_not_identity() {
        let temp = tempfile::tempdir().unwrap();
        let scope = temp.path();
        for bytes in [b"p\xff\n", b"p\xfe\n"] {
            let name = OsStr::from_bytes(bytes);
            fs::create_dir(scope.join(name)).unwrap();
            fs::write(scope.join(name).join("go.mod"), "").unwrap();
        }
        let catalog = ProjectCatalog::discover(scope, &|| false);
        for bytes in [b"p\xff\n", b"p\xfe\n"] {
            let path = Path::new(OsStr::from_bytes(bytes));
            assert_eq!(catalog.project_for(&path.join("n")).unwrap().root, path);
            assert_eq!(catalog.repo_matches(&path.join("n"), "p�\n"), Some(false));
        }
        let (warm, _) = catalog.warm_projects();
        let decoded: Vec<WarmProject> =
            serde_json::from_slice(&serde_json::to_vec(&warm).unwrap()).unwrap();
        assert_eq!(decoded, warm);
    }

    #[test]
    fn current_scan_detects_a_grandchild_move_without_a_watch_hint() {
        let temp = tempfile::tempdir().unwrap();
        let scope = temp.path();
        for dir in ["one/src/deep", "two/src/deep"] {
            fs::create_dir_all(scope.join(dir)).unwrap();
        }
        fs::write(scope.join("one/src/deep/go.mod"), "").unwrap();
        let before = ProjectCatalog::discover(scope, &|| false);
        fs::rename(
            scope.join("one/src/deep/go.mod"),
            scope.join("two/src/deep/go.mod"),
        )
        .unwrap();
        let after = ProjectCatalog::discover(scope, &|| false);
        assert_eq!(owner(&before, "one/src/deep/f"), Path::new("one/src/deep"));
        assert_eq!(owner(&after, "one/src/deep/f"), Path::new(""));
        assert_eq!(owner(&after, "two/src/deep/f"), Path::new("two/src/deep"));
        let cancelled = ProjectCatalog::discover(scope, &|| true);
        assert!(cancelled.truncated);
        assert_eq!(
            cancelled.repo_matches(Path::new("two/src/deep/f"), "deep"),
            None
        );
    }

    #[test]
    fn bounded_warm_delivery_does_not_narrow_membership() {
        let temp = tempfile::tempdir().unwrap();
        let scope = temp.path();
        fs::create_dir(scope.join("many")).unwrap();
        fs::create_dir(scope.join("loose")).unwrap();
        let hints = (0..=WARM_LIMIT).map(|n| format!("language-{n}")).collect();
        let declared = DeclaredBoundary::new("many".into(), Some(hints)).unwrap();
        let (catalog, _) = ProjectCatalog::scan(scope, &|| false, &[declared]);
        let (warm, limited) = catalog.warm_projects();
        assert!(limited);
        assert_eq!(warm.len(), WARM_LIMIT);
        assert!(!catalog.truncated);
        assert_eq!(owner(&catalog, "loose/note"), Path::new(""));
        assert_eq!(owner(&catalog, "many/note"), Path::new("many"));
    }
}
