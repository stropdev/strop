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

use std::path::{Path, PathBuf};

/// Bound the discovery scan like every other walk (0051 §3): a runaway
/// tree ends in a bounded catalog plus an honest `truncated` flag, never
/// an unbounded stall.
const DISCOVERY_PATH_LIMIT: usize = 50_000;

/// One language-project marker file. The list is deliberately tight and
/// typed: every entry maps to a real configuration artifact, and
/// `Makefile`-shaped guesses are not project boundaries.
const MARKERS: &[&str] = &[
    "Cargo.toml",
    "pyproject.toml",
    "setup.py",
    "CMakeLists.txt",
    "package.json",
    "go.mod",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Workspace-relative root (`""` is the scope itself).
    pub root: PathBuf,
    /// Matching name for `repo:` — the root directory's basename, or the
    /// worktree directory's basename for worktrees.
    pub name: String,
    pub kind: ProjectKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKind {
    /// A `.git` directory sits at the root.
    GitRepository,
    /// A `.git` file points at a linked worktree's gitdir.
    GitWorktree,
    /// A language-project marker sits at the root (a subproject boundary
    /// even inside a repository); the marker file names the language
    /// project family for server warm-up (0063 §2).
    Marker(&'static str),
    /// The opened scope with no deeper boundary discovered.
    Scope,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectCatalog {
    projects: Vec<Project>,
    /// True when the bounds stopped the scan early: coverage claims must
    /// say so instead of pretending completeness (0063 §2).
    pub truncated: bool,
    /// Mtime evidence of every visited directory, keyed by its
    /// workspace-relative path (0063 residual, 0058 §2): refresh prunes a
    /// subtree only while the directory's own fresh stat still matches —
    /// the observation self-heals a missed hint for direct children.
    stamps: std::collections::HashMap<PathBuf, Option<std::time::SystemTime>>,
}

/// Scan accounting for one catalog refresh (0058 S7 evidence).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CatalogStats {
    /// Directories the walk actually descended into and reclassified.
    pub visited: usize,
    /// Subtrees reused unchanged (no invalidation, matching mtime).
    pub reused: usize,
}

impl ProjectCatalog {
    /// Every discovered boundary, discovery order.
    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.iter()
    }

    /// One bounded scan of `scope`. Cancellation is cooperative; an
    /// unreadable entry is skipped, never fatal — discovery degrades to
    /// fewer boundaries, and undecidable atoms admit.
    pub fn discover(scope: &Path, cancelled: &impl Fn() -> bool) -> Self {
        let mut catalog = ProjectCatalog::default();
        catalog.projects.push(scope_project(scope));
        let mut builder = discovery_walk(scope);
        builder.filter_entry(|entry| entry.file_name() != ".git");
        let mut visited = 0usize;
        for result in builder.build() {
            if cancelled() {
                return catalog;
            }
            visited += 1;
            if visited > DISCOVERY_PATH_LIMIT {
                catalog.truncated = true;
                return catalog;
            }
            let Ok(entry) = result else {
                continue;
            };
            let Some(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() || entry.depth() == 0 {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(scope) else {
                continue;
            };
            catalog.record_stamp(relative, &entry);
            if let Some(project) = inspect(entry.path(), relative) {
                catalog.projects.push(project);
            }
        }
        catalog.finish();
        catalog
    }

    /// Refresh against the current tree (0063 residual, 0058 §2): a
    /// subtree is reused only while no hint invalidates it AND the
    /// directory's fresh mtime still matches the recorded stamp; the
    /// rest is re-walked. Hint paths and directory prefixes intersect
    /// conservatively in both directions, so an ambiguous hint never
    /// strands a stale boundary.
    pub fn refresh(
        prior: &ProjectCatalog,
        scope: &Path,
        invalidated: &[PathBuf],
        cancelled: &impl Fn() -> bool,
    ) -> (Self, CatalogStats) {
        let mut catalog = ProjectCatalog::default();
        catalog.projects.push(scope_project(scope));
        let mut stats = CatalogStats::default();
        let pruned = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut builder = discovery_walk(scope);
        let scope_owned = scope.to_path_buf();
        let invalidated = invalidated.to_vec();
        let prior_stamps = std::sync::Arc::new(prior.stamps.clone());
        let pruned_sink = std::sync::Arc::clone(&pruned);
        builder.filter_entry(move |entry| {
            if entry.file_name() == ".git" {
                return false;
            }
            let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
            if !is_dir || entry.depth() == 0 {
                return true;
            }
            let Ok(relative) = entry.path().strip_prefix(&scope_owned) else {
                return true;
            };
            let hinted = invalidated
                .iter()
                .any(|prefix| relative.starts_with(prefix) || prefix.starts_with(relative));
            if hinted {
                return true;
            }
            let mtime = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            match prior_stamps.get(relative) {
                Some(stamp) if *stamp == mtime => {
                    pruned_sink.lock().push(relative.to_path_buf());
                    false
                }
                _ => true,
            }
        });
        let mut visited = 0usize;
        for result in builder.build() {
            if cancelled() {
                catalog.finish();
                return (catalog, stats);
            }
            visited += 1;
            if visited > DISCOVERY_PATH_LIMIT {
                catalog.truncated = true;
                break;
            }
            let Ok(entry) = result else {
                continue;
            };
            let Some(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() || entry.depth() == 0 {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(scope) else {
                continue;
            };
            stats.visited += 1;
            catalog.record_stamp(relative, &entry);
            if let Some(project) = inspect(entry.path(), relative) {
                catalog.projects.push(project);
            }
        }
        // Reused subtrees contribute their prior boundaries verbatim.
        for subtree in std::mem::take(&mut *pruned.lock()) {
            stats.reused += 1;
            catalog.stamps.extend(
                prior
                    .stamps
                    .iter()
                    .filter(|(dir, _)| dir.starts_with(&subtree))
                    .map(|(dir, stamp)| (dir.clone(), *stamp)),
            );
            let reused = prior
                .projects
                .iter()
                .filter(|project| project.root.starts_with(&subtree))
                .cloned();
            catalog.projects.extend(reused);
        }
        catalog.truncated = catalog.truncated || prior.truncated;
        catalog.finish();
        (catalog, stats)
    }

    fn record_stamp(&mut self, relative: &Path, entry: &ignore::DirEntry) {
        let stamp = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        self.stamps.insert(relative.to_path_buf(), stamp);
    }

    fn finish(&mut self) {
        // Deepest-first ordering makes `project_for` a linear scan for
        // the longest enclosing root.
        self.projects
            .sort_by_key(|project| std::cmp::Reverse(project.root.as_os_str().len()));
    }

    /// The deepest project whose root encloses `path` (workspace-relative).
    pub fn project_for(&self, path: &str) -> Option<&Project> {
        let prefix = Path::new(path);
        self.projects.iter().find(|project| {
            project.root.as_os_str().is_empty() || prefix.starts_with(&project.root)
        })
    }

    /// Whether `repo:NAME` holds for `path`. Unknown paths admit —
    /// overfetch, never a silent drop.
    pub fn repo_matches(&self, path: &str, name: &str, negated: bool) -> bool {
        let matched = self
            .project_for(path)
            .is_some_and(|project| project.name == name);
        matched != negated
    }
}

/// The scope's own catalog entry (`""` root): always present.
fn scope_project(scope: &Path) -> Project {
    let scope_name = scope.file_name().map_or_else(
        || scope.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    Project {
        root: PathBuf::new(),
        name: scope_name,
        kind: ProjectKind::Scope,
    }
}

/// The one discovery walk policy: ignore rules on, `.git` internals
/// never entered (refresh layers its prune predicate on top).
fn discovery_walk(scope: &Path) -> ignore::WalkBuilder {
    let mut builder = ignore::WalkBuilder::new(scope);
    builder
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .parents(true);
    builder
}

/// Classify one directory from its on-disk boundaries.
fn inspect(absolute: &Path, relative: &Path) -> Option<Project> {
    let name = relative
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())?;
    let git = absolute.join(".git");
    let kind = if git.is_file() {
        ProjectKind::GitWorktree
    } else if git.is_dir() {
        ProjectKind::GitRepository
    } else {
        let marker = MARKERS
            .iter()
            .find(|marker| absolute.join(marker).is_file())?;
        ProjectKind::Marker(marker)
    };
    Some(Project {
        root: relative.to_path_buf(),
        name,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> bool {
        false
    }

    #[test]
    fn discovers_repositories_worktrees_markers_and_loose_scope() {
        let directory = tempfile::tempdir().unwrap();
        let scope = directory.path();
        // Nested repository.
        std::fs::create_dir_all(scope.join("engine/src")).unwrap();
        std::fs::create_dir(scope.join("engine/.git")).unwrap();
        // Linked worktree: a `.git` FILE pointing at a gitdir.
        std::fs::create_dir_all(scope.join("wt-feature")).unwrap();
        std::fs::write(
            scope.join("wt-feature/.git"),
            "gitdir: /elsewhere/engine/.git/worktrees/feature\n",
        )
        .unwrap();
        // Marker subproject inside the repository (monorepo shape).
        std::fs::create_dir_all(scope.join("engine/tools/py")).unwrap();
        std::fs::write(scope.join("engine/tools/py/pyproject.toml"), "").unwrap();
        // Loose directory: source, no boundary.
        std::fs::create_dir_all(scope.join("notes")).unwrap();
        std::fs::write(scope.join("notes/a.md"), "x").unwrap();

        let catalog = ProjectCatalog::discover(scope, &never);
        let by_root = |root: &str| {
            catalog
                .projects
                .iter()
                .find(|project| project.root == Path::new(root))
        };
        assert_eq!(
            by_root("").map(|project| project.kind),
            Some(ProjectKind::Scope)
        );
        assert_eq!(
            by_root("engine").map(|project| project.kind),
            Some(ProjectKind::GitRepository)
        );
        assert_eq!(
            by_root("wt-feature").map(|project| project.kind),
            Some(ProjectKind::GitWorktree)
        );
        assert_eq!(
            by_root("engine/tools/py").map(|project| &project.kind),
            Some(&ProjectKind::Marker("pyproject.toml"))
        );
        assert!(by_root("notes").is_none());

        // Deepest enclosing project wins; repo names match basenames.
        assert_eq!(
            catalog.project_for("engine/tools/py/x.py").map(|p| &p.name),
            Some(&"py".to_string())
        );
        assert_eq!(
            catalog.project_for("engine/src/main.rs").map(|p| &p.name),
            Some(&"engine".to_string())
        );
        assert_eq!(
            catalog.project_for("notes/a.md").map(|p| &p.name),
            Some(&scope.file_name().unwrap().to_string_lossy().into_owned())
        );
        assert!(!catalog.repo_matches("engine/src/main.rs", "tools", false));
        assert!(catalog.repo_matches("notes/a.md", "engine", true));
    }

    #[test]
    fn ignored_repositories_are_missed_soundly() {
        let directory = tempfile::tempdir().unwrap();
        let scope = directory.path();
        // Gitignore semantics apply inside a repository — the same
        // require-git default as the selection walk.
        std::fs::create_dir(scope.join(".git")).unwrap();
        std::fs::write(scope.join(".gitignore"), "hidden-repo/\n").unwrap();
        std::fs::create_dir_all(scope.join("hidden-repo")).unwrap();
        std::fs::create_dir(scope.join("hidden-repo/.git")).unwrap();

        let catalog = ProjectCatalog::discover(scope, &never);
        // The ignored repository is not in the catalog...
        assert!(catalog
            .projects
            .iter()
            .all(|project| project.name != "hidden-repo"));
        // ...and the path still classifies (to the scope project): a
        // decided answer, not an error. Undecidability belongs to the
        // admission layer (no catalog at all), which admits.
        assert!(catalog
            .project_for("hidden-repo/x.rs")
            .is_some_and(|project| project.kind == ProjectKind::Scope));
        assert!(!catalog.truncated);
    }
}
