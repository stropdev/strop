//! The one file-selection authority (0051 §3): the same compiled
//! `FileSelectionPlan` plus the session visibility policy produce the
//! same eligible path set for file find, grep and replace. `.git/` and
//! other repository internals are protected metadata — never ordinary
//! project search results, even with `ignored:include`; neither are the
//! editor's own protocol files (`strop_fs::is_protocol_path`).

use std::path::Path;
pub(super) const PATH_LIMIT: usize = 100_000;
pub(super) const PATH_BYTES: usize = 16 * 1024 * 1024;

/// Portable display/search spelling; the native path remains the I/O identity.
pub fn display_path(path: &Path) -> std::borrow::Cow<'_, str> {
    let value = path.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '/' {
        value
    } else {
        std::borrow::Cow::Owned(value.replace(std::path::MAIN_SEPARATOR, "/"))
    }
}

use crate::query::FileSelectionPlan;

/// Session/workspace visibility defaults (query qualifiers override).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionPolicy {
    /// Show unignored dotfiles/dotfolders (config `search_show_hidden`).
    pub hidden: bool,
    /// Respect .gitignore/.ignore/.rgignore (`search_respect_ignore`).
    pub respect_ignore: bool,
}

impl SelectionPolicy {
    /// The effective policy for one query: its explicit qualifiers win.
    pub fn effective(&self, plan: &FileSelectionPlan) -> SelectionPolicy {
        SelectionPolicy {
            hidden: plan.hidden.unwrap_or(self.hidden),
            // `ignored:include` INCLUDES ignored entries = do not respect
            // the ignore files; `ignored:exclude` respects them.
            respect_ignore: plan
                .ignored
                .map(|include| !include)
                .unwrap_or(self.respect_ignore),
        }
    }
}

pub(super) const DISCOVERY_POLICY: SelectionPolicy = SelectionPolicy {
    hidden: true,
    respect_ignore: true,
};

fn visible_entry(entry: &ignore::DirEntry) -> bool {
    let name = entry.file_name();
    name != ".git" && !strop_fs::is_protocol_path(Path::new(name))
}

/// Projecting declarations into a deliberately narrower folder still needs
/// evidence that the pinned-root policy could reach it. Only descend along
/// its parent chain. In particular, never project across an ignored or linked
/// ancestor, even if the query itself asked to search there.
pub(super) fn observes_folder(pinned: &Path, folder: &Path, cancelled: &impl Fn() -> bool) -> bool {
    for result in ancestors_to(pinned, folder).build() {
        if cancelled() {
            break;
        }
        let Ok(entry) = result else { return false };
        if entry.error().is_some() {
            return false;
        }
        if entry.path() == folder {
            return entry.file_type().is_some_and(|ty| ty.is_dir());
        }
    }
    false
}

/// The same ignore and protected-path policy as discovery, pruned to the
/// one ancestor chain which could witness this target. Off the input path.
pub(super) fn ancestors_to(root: &Path, target: &Path) -> ignore::WalkBuilder {
    toward(root, target.to_owned(), DISCOVERY_POLICY)
}

fn toward(root: &Path, target: std::path::PathBuf, policy: SelectionPolicy) -> ignore::WalkBuilder {
    let mut walk = builder(root, policy);
    walk.filter_entry(move |entry| visible_entry(entry) && target.starts_with(entry.path()));
    walk
}

/// A path component never treated as ordinary project content.
fn protected(rel: &Path) -> bool {
    rel.components()
        .any(|component| component.as_os_str() == ".git")
}

/// The walk both surfaces share: ignore files by default, hidden entries
/// by policy, `.git` never, and never the editor's own protocol files
/// (locks, save and filesystem stages) that sit beside the user's.
pub(super) fn builder(cwd: &Path, effective: SelectionPolicy) -> ignore::WalkBuilder {
    let mut builder = ignore::WalkBuilder::new(cwd);
    builder
        .filter_entry(visible_entry)
        .hidden(!effective.hidden)
        .ignore(effective.respect_ignore)
        .git_ignore(effective.respect_ignore)
        .git_exclude(effective.respect_ignore);
    builder
        .git_global(effective.respect_ignore)
        .parents(effective.respect_ignore);
    builder
}

/// The workspace-relative path of an eligible entry: a regular file, or
/// a link to one, outside protected metadata, that the plan allows. A
/// broken or unreadable link is skipped, never a walk-stopping error
/// (one dangling link must not empty Space f / Space /).
fn eligible<'a>(
    entry: &'a ignore::DirEntry,
    cwd: &Path,
    plan: &FileSelectionPlan,
) -> Option<&'a Path> {
    let kind = entry.file_type()?;
    if kind.is_symlink() {
        if !std::fs::metadata(entry.path()).ok()?.is_file() {
            return None;
        }
    } else if !kind.is_file() {
        return None;
    }
    let rel = entry.path().strip_prefix(cwd).ok()?;
    if protected(rel) || !plan.allows(&display_path(rel)) {
        return None;
    }
    Some(rel)
}

/// Stream eligible workspace-relative paths in tree order: sorting each
/// directory by name makes the file finder list the same tree the same
/// way everywhere.
pub fn walk(
    cwd: &Path,
    plan: &FileSelectionPlan,
    policy: SelectionPolicy,
    cancelled: &impl Fn() -> bool,
    emit: impl FnMut(std::path::PathBuf) -> bool,
) -> Result<(), String> {
    walk_observing(cwd, plan, policy, cancelled, |_| {}, emit).map(|_| ())
}

/// Observe directories from this same authoritative walk, before any file
/// predicates. Directory observation never excludes a candidate from selection.
pub(super) fn walk_observing(
    cwd: &Path,
    plan: &FileSelectionPlan,
    policy: SelectionPolicy,
    cancelled: &impl Fn() -> bool,
    mut visit: impl FnMut(&Path),
    mut emit: impl FnMut(std::path::PathBuf) -> bool,
) -> Result<bool, String> {
    let mut complete = true;
    let mut builder = builder(cwd, policy.effective(plan));
    builder.sort_by_file_name(|a, b| a.cmp(b));
    for result in builder.build() {
        if cancelled() {
            return Ok(false);
        }
        let entry = result.map_err(|error| error.to_string())?;
        complete &= entry.error().is_none();
        if entry.file_type().is_some_and(|ty| ty.is_dir()) {
            if let Ok(relative) = entry.path().strip_prefix(cwd) {
                visit(relative);
            }
            continue;
        }
        if let Some(rel) = eligible(&entry, cwd, plan) {
            if !emit(rel.to_owned()) {
                return Ok(false);
            }
        }
    }
    Ok(complete)
}

/// Why a gathered selection has no path list.
#[derive(Debug)]
pub enum CollectError {
    /// More than `PATH_LIMIT` paths or `PATH_BYTES` of them.
    Limit,
    Walk(String),
}

/// Gather only for consumers whose evidence requires the complete path set.
pub fn collect(
    cwd: &Path,
    plan: &FileSelectionPlan,
    policy: SelectionPolicy,
    cancelled: &(impl Fn() -> bool + Sync),
    threads: usize,
) -> Result<Vec<std::path::PathBuf>, CollectError> {
    use std::path::PathBuf;
    /// A thread's finds, handed over in runs and once more when it stops.
    struct Run<'a> {
        local: Vec<PathBuf>,
        gathered: &'a parking_lot::Mutex<Vec<PathBuf>>,
    }
    impl Drop for Run<'_> {
        fn drop(&mut self) {
            self.gathered.lock().append(&mut self.local);
        }
    }
    let gathered = parking_lot::Mutex::new(Vec::new());
    stream(cwd, plan, policy, cancelled, threads, None, || {
        let mut run = Run {
            local: Vec::new(),
            gathered: &gathered,
        };
        move |rel: &Path| {
            run.local.push(rel.to_owned());
            if run.local.len() >= 256 {
                run.gathered.lock().append(&mut run.local);
            }
            true
        }
    })?;
    Ok(gathered.into_inner())
}

/// Confirm the hinted path by traversing only its ancestor chain from the
/// actual walk root. Never start a walker at the leaf: that would bypass
/// ignored/hidden ancestors. A miss or a walk problem falls back to the
/// unpruned authoritative walk, which owns reporting enumeration failures.
pub(super) fn admits_hint(
    cwd: &Path,
    plan: &FileSelectionPlan,
    policy: SelectionPolicy,
    hint: &Path,
    cancelled: &impl Fn() -> bool,
) -> bool {
    for result in toward(cwd, cwd.join(hint), policy.effective(plan)).build() {
        if cancelled() {
            break;
        }
        let Ok(entry) = result else { break };
        if entry.error().is_some() {
            break;
        }
        if eligible(&entry, cwd, plan) == Some(hint) {
            return true;
        }
    }
    false
}

/// Each walker thread verifies the path before requesting another: bounded
/// work in flight, no additional search pool and no path-sized queue. Only
/// an already verified path from this very request can discharge a duplicate.
pub(super) fn stream<F>(
    cwd: &Path,
    plan: &FileSelectionPlan,
    policy: SelectionPolicy,
    cancelled: &(impl Fn() -> bool + Sync),
    threads: usize,
    verified: Option<&Path>,
    mut lane: impl FnMut() -> F,
) -> Result<bool, CollectError>
where
    F: FnMut(&Path) -> bool + Send,
{
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let failure = parking_lot::Mutex::new(None);
    let count = AtomicUsize::new(usize::from(verified.is_some()));
    let bytes = AtomicUsize::new(verified.map_or(0, |p| p.as_os_str().as_encoded_bytes().len()));
    let interrupted = AtomicBool::new(false);
    let over = AtomicBool::new(false);
    let mut builder = builder(cwd, policy.effective(plan));
    builder.threads(threads);
    builder.build_parallel().run(|| {
        let mut visit = lane();
        let (failure, count, bytes, over, interrupted) =
            (&failure, &count, &bytes, &over, &interrupted);
        Box::new(move |result| {
            if cancelled() || over.load(Ordering::Acquire) || interrupted.load(Ordering::Acquire) {
                interrupted.store(true, Ordering::Release);
                return ignore::WalkState::Quit;
            }
            let entry = match result {
                Ok(entry) => entry,
                Err(error) => {
                    failure.lock().get_or_insert(error.to_string());
                    over.store(true, Ordering::Release);
                    return ignore::WalkState::Quit;
                }
            };
            if let Some(error) = entry.error() {
                failure.lock().get_or_insert_with(|| error.to_string());
                interrupted.store(true, Ordering::Release);
                return ignore::WalkState::Quit;
            }
            let Some(rel) = eligible(&entry, cwd, plan) else {
                return ignore::WalkState::Continue;
            };
            if Some(rel) == verified {
                return ignore::WalkState::Continue;
            }
            let size = rel.as_os_str().as_encoded_bytes().len();
            if count.fetch_add(1, Ordering::AcqRel) >= PATH_LIMIT
                || bytes.fetch_add(size, Ordering::AcqRel) + size > PATH_BYTES
            {
                over.store(true, Ordering::Release);
                return ignore::WalkState::Quit;
            }
            if !visit(rel) {
                interrupted.store(true, Ordering::Release);
                return ignore::WalkState::Quit;
            }
            ignore::WalkState::Continue
        })
    });
    if let Some(error) = failure.into_inner() {
        return Err(CollectError::Walk(error));
    }
    if over.into_inner() {
        return Err(CollectError::Limit);
    }
    Ok(!interrupted.into_inner())
}

/// The protected-path exclusion as rg argv fragments (`--glob !.git`…).
/// Shared by every rg invocation so no surface searches repository
/// internals by accident.
pub const RG_PROTECTED_ARGS: &[&str] = &["--glob", "!.git", "--glob", "!.git/**"];

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(cwd: &Path, query: &str, policy: SelectionPolicy) -> Vec<String> {
        let parsed = crate::query::parse_search(query);
        let plan = FileSelectionPlan::compile(&parsed).unwrap();
        let mut out = Vec::new();
        walk(cwd, &plan, policy, &|| false, |rel| {
            out.push(rel.display().to_string());
            true
        })
        .unwrap();
        out
    }

    fn fixture() -> tempfile::TempDir {
        // .gitignore applies inside git repositories (ignore/rg default)
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("src/lib.py"), "x = 1\n").unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=x\n").unwrap();
        std::fs::write(dir.path().join(".github/workflows/ci.yml"), "on: push\n").unwrap();
        std::fs::write(dir.path().join("target/build.rs"), "// build\n").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        dir
    }

    #[test]
    fn dotfiles_show_by_default_but_ignore_files_hold() {
        let dir = fixture();
        let got = collect(
            dir.path(),
            "",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(got.contains(&".env".to_string()), "{got:?}");
        assert!(
            got.contains(&".github/workflows/ci.yml".to_string()),
            "{got:?}"
        );
        assert!(!got.contains(&"target/build.rs".to_string()), "{got:?}");
    }

    #[test]
    fn the_editor_s_own_protocol_files_are_never_listed() {
        let dir = fixture();
        let lock = format!(".strop-lock-{}", "a".repeat(64));
        std::fs::write(dir.path().join("src").join(&lock), "").unwrap();
        std::fs::create_dir(dir.path().join(".strop-fs-stage")).unwrap();
        std::fs::write(dir.path().join(".strop-fs-stage/main.rs"), "x\n").unwrap();
        std::fs::write(dir.path().join(".strop-save-main.rs"), "x\n").unwrap();
        let everything = SelectionPolicy {
            hidden: true,
            respect_ignore: false,
        };
        let listed = collect(dir.path(), "", everything);
        assert!(
            !listed.iter().any(|path| path.contains(".strop-")),
            "{listed:?}"
        );
        assert!(listed.contains(&".env".to_string()), "{listed:?}");
        // Content search gathers through the same builder.
        let plan = FileSelectionPlan::compile(&crate::query::parse_search("")).unwrap();
        let gathered = super::collect(dir.path(), &plan, everything, &|| false, 2).unwrap();
        assert!(
            !gathered
                .iter()
                .any(|path| path.to_string_lossy().contains(".strop-")),
            "{gathered:?}"
        );
        assert!(gathered.contains(&std::path::PathBuf::from("src/main.rs")));
    }

    #[test]
    fn ignored_include_surfaces_ignored_output() {
        let dir = fixture();
        let got = collect(
            dir.path(),
            "ignored:include",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(got.contains(&"target/build.rs".to_string()), "{got:?}");
    }

    #[test]
    fn hidden_exclude_restores_the_old_world() {
        let dir = fixture();
        let got = collect(
            dir.path(),
            "hidden:exclude",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(!got.contains(&".env".to_string()), "{got:?}");
        assert!(
            !got.contains(&".github/workflows/ci.yml".to_string()),
            "{got:?}"
        );
        assert!(got.contains(&"src/main.rs".to_string()), "{got:?}");
    }

    #[test]
    fn fuzzy_path_scope_filters_real_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        for path in [
            "src/modules/jweb/Parser.java",
            "src/modules/other/Parser.java",
            "modules/legacy/jweb/parser.java",
        ] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "class Parser {}\n").unwrap();
        }
        let got = collect(
            dir.path(),
            "fpath:modules/jweb/ parser",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.iter().all(|path| path.contains("jweb")));
    }

    #[test]
    fn language_filter_only_lists_that_family() {
        let dir = fixture();
        let got = collect(
            dir.path(),
            "language:rust",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert_eq!(got, vec!["src/main.rs".to_string()]);
    }

    #[test]
    fn git_internals_are_never_results() {
        let dir = fixture();
        std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "[core]\n").unwrap();
        let got = collect(
            dir.path(),
            "ignored:include",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(!got.iter().any(|p| p.starts_with(".git/")), "{got:?}");
    }

    /// One dangling link is ineligible, never a walk-stopping error:
    /// the rest of the workspace still streams (0051 §3 boundaries).
    #[test]
    fn a_broken_symlink_never_empties_the_walk() {
        let dir = fixture();
        std::os::unix::fs::symlink("missing-target", dir.path().join("dangling")).unwrap();
        std::os::unix::fs::symlink("src/main.rs", dir.path().join("linked.rs")).unwrap();
        let got = collect(
            dir.path(),
            "",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(got.contains(&"src/main.rs".to_string()), "{got:?}");
        assert!(got.contains(&"linked.rs".to_string()), "{got:?}");
        assert!(!got.contains(&"dangling".to_string()), "{got:?}");
    }
}
