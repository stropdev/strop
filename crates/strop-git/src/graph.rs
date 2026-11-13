//! strop's private commit-graph (plan 0101 §2.1).
//!
//! Without a commit-graph, `git log --graph` walks every commit before it
//! prints the first. strop keeps one for a large repository in its own
//! state directory — never in the user's `.git`: no file, no lock, no
//! config key — and names it to git as an alternate object directory on
//! its history reads only. The file is an index keyed by commit id, so a
//! stale one cannot change a row; git validates it and is its only reader.
//!
//! Layout: `<state>/strop/graphs/<key>/objects/info/commit-graph`, `<key>`
//! a hash of the repository's common object directory, so the worktrees of
//! one repository share a graph. A build writes into a scratch directory
//! beside it and renames over the installed file: a reader sees the old
//! file or the new one, and a killed build leaves only scratch behind.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use strop_core::worker::CancelToken;

/// The variable a history read sets; a user who set it means something
/// by it, and strop then leaves it alone.
pub const VARIABLE: &str = "GIT_ALTERNATE_OBJECT_DIRECTORIES";
/// How many repositories' graphs are kept (about 70 bytes per commit).
pub const KEEP: usize = 16;
/// The longest one build step may take.
pub const STEP_DEADLINE: Duration = Duration::from_secs(120);
/// Scratch older than this belongs to no live build.
const ABANDONED: Duration = Duration::from_secs(600);

/// The two steps of a build, each installed when it finishes: the graph
/// itself fixes the log; its changed-path filters make a file's history
/// fast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Step {
    Reachable,
    ChangedPaths,
}

impl Step {
    fn argv(self, objects: &Path) -> Vec<OsString> {
        let mut argv: Vec<OsString> =
            vec!["commit-graph".into(), "write".into(), "--reachable".into()];
        if self == Step::ChangedPaths {
            argv.push("--changed-paths".into());
        }
        let mut dir = OsString::from("--object-dir=");
        dir.push(objects);
        argv.push(dir);
        argv
    }
}

/// Where strop keeps its graphs under a state root.
pub fn graphs_root(state_root: &Path) -> PathBuf {
    state_root.join("strop").join("graphs")
}

/// The repository's common object directory (shared by its worktrees).
pub fn common_objects(workdir: &Path) -> Option<PathBuf> {
    let repo = git2::Repository::discover(workdir).ok()?;
    Some(repo.commondir().join("objects"))
}

/// This repository's private directory under `graphs`.
pub fn side_dir(graphs: &Path, objects: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(objects).unwrap_or_else(|_| objects.to_path_buf());
    let mut hasher = std::hash::DefaultHasher::new();
    std::hash::Hash::hash(&canonical, &mut hasher);
    graphs.join(format!("{:016x}", std::hash::Hasher::finish(&hasher)))
}

fn graph_file(objects: &Path) -> PathBuf {
    objects.join("info").join("commit-graph")
}

/// Whether git keeps a commit-graph of its own here. Git reads its own
/// first and would never read strop's, so strop builds nothing for it.
pub fn owns_graph(objects: &Path) -> bool {
    graph_file(objects).is_file() || objects.join("info").join("commit-graphs").is_dir()
}

/// The object directory to name to git for a history read of the
/// repository at `workdir`, when strop has a graph for it. Marks the
/// graph used, for eviction.
pub fn for_history(graphs: &Path, workdir: &Path) -> Option<PathBuf> {
    if std::env::var_os(VARIABLE).is_some() {
        return None;
    }
    let side = side_dir(graphs, &common_objects(workdir)?);
    let objects = side.join("objects");
    if !graph_file(&objects).is_file() {
        return None;
    }
    touch(&side);
    Some(objects)
}

fn touch(dir: &Path) {
    // best effort: a missed touch only makes eviction less exact
    if let Ok(file) = std::fs::File::open(dir) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Run one build step for the repository at `workdir` into `side`:
/// scratch directory, seeded with the installed file so filters are
/// reused, git with the variable set, rename over the installed file.
pub fn build(step: Step, workdir: &Path, side: &Path, cancel: &CancelToken) -> Result<(), String> {
    let io = |what: &str, path: &Path, error: std::io::Error| {
        format!("commit-graph {what} {}: {error}", path.display())
    };
    let installed = side.join("objects");
    for dir in [installed.join("info"), installed.join("pack")] {
        std::fs::create_dir_all(&dir).map_err(|error| io("create", &dir, error))?;
    }
    remove_abandoned(side);
    let scratch = side.join(format!("build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let objects = scratch.join("objects");
    for dir in [objects.join("info"), objects.join("pack")] {
        std::fs::create_dir_all(&dir).map_err(|error| io("create", &dir, error))?;
    }
    let result = (|| {
        if graph_file(&installed).is_file() {
            std::fs::copy(graph_file(&installed), graph_file(&objects))
                .map_err(|error| io("seed", &objects, error))?;
        }
        let env = [(VARIABLE, objects.as_os_str())];
        let run =
            crate::exec::local_run(workdir, &step.argv(&objects), &env, STEP_DEADLINE, cancel)
                .map_err(|error| format!("commit-graph write: {error}"))?;
        if !run.success {
            return Err(format!(
                "commit-graph write: {}",
                String::from_utf8_lossy(&run.stderr).trim()
            ));
        }
        std::fs::rename(graph_file(&objects), graph_file(&installed))
            .map_err(|error| io("install", &installed, error))
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    touch(side);
    result
}

/// Scratch directories of builds that were killed.
fn remove_abandoned(side: &Path) {
    let Ok(entries) = std::fs::read_dir(side) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > ABANDONED);
        if stale && name.as_encoded_bytes().starts_with(b"build-") {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Make room for `side`: when it is not kept yet and `keep` graphs are,
/// the least recently used directories go first.
pub fn evict(graphs: &Path, side: &Path, keep: usize) {
    if side.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(graphs) else {
        return;
    };
    let mut kept: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| {
            let used = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (used, entry.path())
        })
        .collect();
    kept.sort();
    let excess = (kept.len() + 1).saturating_sub(keep);
    for (_, dir) in kept.into_iter().take(excess) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests;
